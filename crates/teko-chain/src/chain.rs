use prost::Message;
use teko_bft::{Validator, ValidatorSet, ValidatorSetChange, block_id};
use teko_bulletin::merkle::Frontier;
use teko_bulletin::{BulletinBoard, Entry, EntryKind, MemoryBoard};

use crate::block::{
    Block, BlockEntry, DecidedBlock, MAX_BLOCK_BYTES, MAX_BLOCK_ENTRIES, genesis_hash,
};
use crate::error::ChainError;
use crate::profile::NetworkProfile;

pub(crate) const MAX_ENTRY_PAYLOAD: usize = 1024 * 1024;

/// A certified block and the Merkle proof that one entry is in it (see `inclusion_evidence`).
#[derive(Clone, Debug)]
pub struct InclusionEvidence {
    pub decided: DecidedBlock,
    pub proof: Vec<teko_bulletin::Hash>,
}

/// A verified replica: the final blocks, plus the bulletin log they rebuild. Nothing enters
/// except through `apply`, which checks everything: the commit certificate (more than 2/3 of the
/// institutions, equal power), the proposer, the content and the exact Merkle state.
/// One period of the institutions (ADR-0045): epoch 0 is the genesis set; every approved change
/// starts the next epoch at its `effective_height`.
#[derive(Clone, Debug, PartialEq)]
pub struct Epoch {
    pub epoch: u64,
    /// The first block height that belongs to this epoch.
    pub from_height: u64,
    /// The institutions of the epoch, ORDERED (the order decides the proposer).
    pub set: Vec<Vec<u8>>,
    /// The approved change that created it (`None` for the genesis epoch).
    pub change: Option<ValidatorSetChange>,
}

pub struct Chain {
    profile: NetworkProfile,
    /// Always at least the genesis epoch, ascending by `from_height`. The last one may still be
    /// PENDING (its `from_height` is ahead of the chain).
    epochs: Vec<Epoch>,
    blocks: Vec<DecidedBlock>,
    board: MemoryBoard,
    frontier: Frontier,
    tip: Vec<u8>,
    last_timestamp: u64,
}

impl Chain {
    pub fn new(profile: NetworkProfile) -> Self {
        let tip = genesis_hash(&profile.chain_id).to_vec();
        let genesis = Epoch {
            epoch: 0,
            from_height: 1,
            set: profile.validators.clone(),
            change: None,
        };
        Self {
            profile,
            epochs: vec![genesis],
            blocks: Vec::new(),
            board: MemoryBoard::new(),
            frontier: Frontier::new(),
            tip,
            last_timestamp: 0,
        }
    }

    pub fn profile(&self) -> &NetworkProfile {
        &self.profile
    }

    /// Number of blocks. The next block has height `height() + 1`.
    pub fn height(&self) -> u64 {
        self.blocks.len() as u64
    }

    pub fn blocks(&self) -> &[DecidedBlock] {
        &self.blocks
    }

    /// Final blocks with height > `after` (`after` = 0 gives the whole chain).
    pub fn blocks_after(&self, after: u64) -> &[DecidedBlock] {
        let h = usize::try_from(after)
            .unwrap_or(usize::MAX)
            .min(self.blocks.len());
        self.blocks.get(h..).unwrap_or(&[])
    }

    /// The bulletin log rebuilt from the blocks (read-only).
    pub fn board(&self) -> &MemoryBoard {
        &self.board
    }

    /// What a voter's DEVICE needs to check, alone, that the entry at `index` is final (ADR-0041):
    /// the first certified block whose log already holds it, and the Merkle proof of the entry in
    /// THAT block's `log_root`. `None` if the entry is not in a block yet.
    pub fn inclusion_evidence(&self, index: u64) -> Option<InclusionEvidence> {
        let (decided, size) = self.blocks.iter().find_map(|d| {
            let b = d.block().ok()?;
            (b.log_size > index).then(|| (d.clone(), b.log_size))
        })?;
        let leaves: Vec<_> = (0..size)
            .map(|i| self.board.entry(i).map(Entry::leaf_hash))
            .collect::<Option<_>>()?;
        let proof = teko_bulletin::merkle::inclusion_proof(&leaves, usize::try_from(index).ok()?)?;
        Some(InclusionEvidence { decided, proof })
    }

    /// Consumes the chain, keeping the verified Merkle log.
    pub fn into_board(self) -> MemoryBoard {
        self.board
    }

    /// Identity of the last block (the genesis hash before the first).
    pub fn tip_hash(&self) -> &[u8] {
        &self.tip
    }

    /// Time of the last block (0 before the first).
    pub fn last_timestamp(&self) -> u64 {
        self.last_timestamp
    }

    /// Every epoch so far, oldest first (the last may be pending).
    pub fn epochs(&self) -> &[Epoch] {
        &self.epochs
    }

    /// The epoch in force at block height `h`.
    pub fn epoch_at(&self, h: u64) -> &Epoch {
        self.epochs
            .iter()
            .rev()
            .find(|e| e.from_height <= h)
            .unwrap_or(&self.epochs[0])
    }

    /// The institutions in force at block height `h` (ADR-0045).
    pub fn set_at(&self, h: u64) -> &[Vec<u8>] {
        &self.epoch_at(h).set
    }

    /// The approved changes after epoch `known`, in order: what a DEVICE that knows epoch `known`
    /// needs to follow the chain of institutions up to now.
    pub fn changes_after(&self, known: u64) -> Vec<ValidatorSetChange> {
        self.epochs
            .iter()
            .filter(|e| e.epoch > known)
            .filter_map(|e| e.change.clone())
            .collect()
    }

    /// The institutions of the NEXT height: equal voting power, one vote each (ADR-0034).
    pub fn validator_set(&self) -> Result<ValidatorSet, ChainError> {
        self.validator_set_for(self.height() + 1)
    }

    /// The institutions in force at block height `h`.
    pub fn validator_set_for(&self, h: u64) -> Result<ValidatorSet, ChainError> {
        ValidatorSet::new(
            self.set_at(h)
                .iter()
                .map(|k| Validator {
                    key: k.clone(),
                    power: 1,
                })
                .collect(),
        )
        .map_err(|_| ChainError::Profile("invalid validator set"))
    }

    /// The change a block carries, if any (at most one per block).
    fn change_in(entries: &[Entry]) -> Result<Option<ValidatorSetChange>, ChainError> {
        let mut found: Option<ValidatorSetChange> = None;
        for e in entries
            .iter()
            .filter(|e| e.kind == EntryKind::VALIDATOR_SET)
        {
            let c = ValidatorSetChange::decode(e.payload.as_slice())
                .map_err(|_| ChainError::Block("malformed validator set change"))?;
            if found.replace(c).is_some() {
                return Err(ChainError::Block("at most one set change per block"));
            }
        }
        Ok(found)
    }

    /// Rules of a set change carried by the block at height `h` (ADR-0045). A block that breaks them
    /// is invalid, so no honest institution votes for it and no replica accepts it.
    fn check_change(&self, c: &ValidatorSetChange, h: u64) -> Result<(), ChainError> {
        let bad = ChainError::Block;
        let last = self.epochs.last().ok_or(bad("no epoch"))?;
        if last.from_height > h {
            return Err(bad("a set change is already pending"));
        }
        c.follow(&self.profile.chain_id, last.epoch, &last.set)
            .map_err(bad)?;
        if c.effective_height <= h {
            return Err(bad(
                "a set change takes effect after the block that carries it",
            ));
        }
        Ok(())
    }

    /// The payload `proposer` (this node's consensus key) proposes for the next height: the
    /// entries, the exact Merkle state appending them would produce, and a time that is the
    /// proposer's clock but never at or before the previous block's.
    pub fn propose(&self, proposer: &[u8], entries: &[Entry], clock_now: u64) -> Vec<u8> {
        let mut f = self.frontier.clone();
        for e in entries {
            f.push(e.leaf_hash());
        }
        let block = Block {
            chain_id: self.profile.chain_id.clone(),
            height: self.height() + 1,
            prev_hash: self.tip.clone(),
            timestamp: clock_now.max(self.last_timestamp.saturating_add(1)),
            proposer: proposer.to_vec(),
            entries: entries.iter().map(BlockEntry::from).collect(),
            log_size: self.board.len() + entries.len() as u64,
            log_root: f.root().to_vec(),
        };
        prost::Message::encode_to_vec(&block)
    }

    /// A DRY RUN: everything about a proposed payload for the next height, with `proposer` being
    /// who the consensus says proposed it. Returns the block and its entries.
    pub fn check(
        &self,
        proposer: &[u8],
        payload: &[u8],
    ) -> Result<(Block, Vec<Entry>), ChainError> {
        let bad = ChainError::Block;
        let b = Block::decode_strict(payload)?;
        if b.chain_id != self.profile.chain_id {
            return Err(bad("other chain_id"));
        }
        if b.height != self.height() + 1 {
            return Err(bad("height is not the next one"));
        }
        if b.prev_hash != self.tip {
            return Err(bad("prev_hash does not match the tip"));
        }
        if b.proposer != proposer {
            return Err(bad("not the proposer the consensus named"));
        }
        if !self.set_at(b.height).contains(&b.proposer) {
            return Err(bad("proposer is not an institution of this network"));
        }
        if !self.blocks.is_empty() && b.timestamp <= self.last_timestamp {
            return Err(ChainError::BadTime);
        }
        if b.entries.is_empty() || b.entries.len() > MAX_BLOCK_ENTRIES {
            return Err(bad("entry count out of range"));
        }
        let total: usize = b.entries.iter().map(|e| e.payload.len() + 8).sum();
        if total > MAX_BLOCK_BYTES
            || b.entries
                .iter()
                .any(|e| e.payload.len() > MAX_ENTRY_PAYLOAD)
        {
            return Err(bad("block too large"));
        }
        let entries: Vec<Entry> = b
            .entries
            .iter()
            .map(BlockEntry::to_entry)
            .collect::<Result<_, _>>()?;
        if let Some(c) = Self::change_in(&entries)? {
            self.check_change(&c, b.height)?;
        }
        let mut f = self.frontier.clone();
        for e in &entries {
            f.push(e.leaf_hash());
        }
        if b.log_size != self.board.len() + entries.len() as u64
            || b.log_root.as_slice() != f.root().as_slice()
        {
            return Err(bad("log size/root do not match the entries"));
        }
        Ok((b, entries))
    }

    /// Verifies a FINAL block completely (certificate, proposer, content, Merkle state) and
    /// commits it.
    pub fn apply(&mut self, d: DecidedBlock) -> Result<(), ChainError> {
        let block = d.block()?;
        let commit = d.commit.as_ref().ok_or(ChainError::Commit)?;
        let set = self.validator_set_for(block.height)?;
        if commit.height != block.height
            || commit.block_id != block_id(&d.payload)
            || !commit.verify(&self.profile.chain_id, &set)
        {
            return Err(ChainError::Commit);
        }
        if set.proposer(commit.height, commit.round).key != block.proposer {
            return Err(ChainError::Block(
                "the proposer is not the one the certificate names",
            ));
        }
        let (block, entries) = self.check(&block.proposer.clone(), &d.payload)?;
        let change = Self::change_in(&entries)?;
        for e in &entries {
            self.frontier.push(e.leaf_hash());
            self.board
                .append(e.clone())
                .map_err(|e| ChainError::Log(e.to_string()))?;
        }
        self.tip = block_id(&d.payload);
        self.last_timestamp = block.timestamp;
        self.blocks.push(d);
        if let Some(c) = change {
            self.epochs.push(Epoch {
                epoch: c.epoch,
                from_height: c.effective_height,
                set: c.validators.clone(),
                change: Some(c),
            });
        }
        Ok(())
    }
}
