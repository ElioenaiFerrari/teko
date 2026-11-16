use prost::Message;
use teko_bft::{Commit, Validator, ValidatorSet, block_id};
use teko_crypto::{tagged_hash, tags};
use teko_ledger::{Address, BlockOutcome, Ledger, Tx};

use crate::genesis::EconomyGenesis;

/// Most transactions one block carries.
pub const MAX_BLOCK_TXS: usize = 1_000;
/// Total voting power is spread over this many units (see `quantize`).
const POWER_UNITS: u128 = 10_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EconError {
    Genesis(String),
    Decode,
    WrongHeight {
        expected: u64,
        got: u64,
    },
    WrongParent,
    /// The block's proposer is not the validator the certificate says proposed it.
    BadProposer,
    NotAValidator,
    BadTx(String),
    StateRoot,
    Commit,
    EmptyValidatorSet,
    /// The block's time does not move forward (it must be strictly after the previous block's).
    BadTime,
}

/// What the validators agree on (its encoding is the BFT payload).
#[derive(Clone, PartialEq, Message)]
pub struct EconBlock {
    #[prost(uint64, tag = "1")]
    pub height: u64,
    /// `block_id` of the previous block (or the genesis id).
    #[prost(bytes = "vec", tag = "2")]
    pub prev_id: Vec<u8>,
    /// CONSENSUS key of the proposer: rewards and fees go to its owner.
    #[prost(bytes = "vec", tag = "3")]
    pub proposer: Vec<u8>,
    /// Unix seconds claimed by the proposer (NOT a trusted clock).
    #[prost(uint64, tag = "4")]
    pub timestamp: u64,
    #[prost(bytes = "vec", repeated, tag = "5")]
    pub txs: Vec<Vec<u8>>,
    /// State root the ledger reaches after this block.
    #[prost(bytes = "vec", tag = "6")]
    pub state_root: Vec<u8>,
}

/// A final block: the payload and the proof that more than 2/3 of the power signed it.
#[derive(Clone, PartialEq, Message)]
pub struct DecidedBlock {
    #[prost(bytes = "vec", tag = "1")]
    pub payload: Vec<u8>,
    #[prost(message, optional, tag = "2")]
    pub commit: Option<Commit>,
}

impl DecidedBlock {
    pub fn block(&self) -> Result<EconBlock, EconError> {
        let b = EconBlock::decode(self.payload.as_slice()).map_err(|_| EconError::Decode)?;
        if b.encode_to_vec() != self.payload {
            return Err(EconError::Decode);
        }
        Ok(b)
    }
}

/// Turns stake into voting power. The BFT proposer rotation hands out CONTIGUOUS runs
/// proportional to the power, so raw stakes (millions) would let the first validator
/// propose for a very long stretch; instead power = max(1, stake * 10_000 / total).
pub fn quantize(stakes: &[u64]) -> Vec<u64> {
    let total: u128 = stakes.iter().map(|s| u128::from(*s)).sum();
    if total == 0 {
        return stakes.iter().map(|_| 1).collect();
    }
    stakes
        .iter()
        .map(|s| ((u128::from(*s) * POWER_UNITS) / total).max(1) as u64)
        .collect()
}

fn genesis_id(chain_id: &str) -> Vec<u8> {
    tagged_hash(tags::BFT_BLOCK, &[b"economy-genesis", chain_id.as_bytes()]).to_vec()
}

/// The economy ledger plus its final blocks.
pub struct EconomyChain {
    genesis: EconomyGenesis,
    ledger: Ledger,
    tip: Vec<u8>,
    blocks: Vec<DecidedBlock>,
    /// Time of the last final block (0 before the first). A block's time is the PROPOSER's;
    /// replay only requires that it moves forward, how close it is to the real clock is checked
    /// live by each validator (`EconApp::validate_from`, ADR-0034).
    last_timestamp: u64,
}

impl EconomyChain {
    pub fn new(genesis: EconomyGenesis) -> Result<Self, EconError> {
        let ledger = genesis.build_ledger().map_err(EconError::Genesis)?;
        let tip = genesis_id(&genesis.chain_id);
        Ok(Self {
            genesis,
            ledger,
            tip,
            blocks: Vec::new(),
            last_timestamp: 0,
        })
    }

    /// A client rebuilds and VERIFIES a chain from the genesis and the final blocks,
    /// trusting no node: every certificate is checked against the validator set the
    /// replayed state implies at that height.
    pub fn replay(genesis: EconomyGenesis, blocks: &[DecidedBlock]) -> Result<Self, EconError> {
        let mut c = Self::new(genesis)?;
        for b in blocks {
            c.apply_decided(b.clone())?;
        }
        Ok(c)
    }

    pub fn chain_id(&self) -> &str {
        &self.genesis.chain_id
    }

    pub fn genesis(&self) -> &EconomyGenesis {
        &self.genesis
    }

    /// Height of the last final block (0 = only genesis).
    pub fn height(&self) -> u64 {
        self.ledger.height()
    }

    pub fn ledger(&self) -> &Ledger {
        &self.ledger
    }

    pub fn blocks(&self) -> &[DecidedBlock] {
        &self.blocks
    }

    /// Final blocks from `height` (1-based) on.
    pub fn blocks_from(&self, height: u64) -> &[DecidedBlock] {
        let skip = height.saturating_sub(1) as usize;
        self.blocks.get(skip..).unwrap_or(&[])
    }

    /// The validator set for the NEXT height, from the current state.
    pub fn validator_set(&self) -> Result<ValidatorSet, EconError> {
        let entries = teko_coin::validator_set(&self.ledger);
        if entries.is_empty() {
            return Err(EconError::EmptyValidatorSet);
        }
        let powers = quantize(&entries.iter().map(|e| e.stake).collect::<Vec<_>>());
        ValidatorSet::new(
            entries
                .into_iter()
                .zip(powers)
                .map(|(e, power)| Validator { key: e.key, power })
                .collect(),
        )
        .map_err(|_| EconError::EmptyValidatorSet)
    }

    /// Decodes and fully checks a proposed payload for the next height WITHOUT changing
    /// anything (a dry run of the ledger). `proposer` is who the consensus says proposed.
    pub fn check(&self, proposer: &[u8], payload: &[u8]) -> Result<EconBlock, EconError> {
        let b = EconBlock::decode(payload).map_err(|_| EconError::Decode)?;
        if b.encode_to_vec() != payload {
            return Err(EconError::Decode);
        }
        let expected = self.height() + 1;
        if b.height != expected {
            return Err(EconError::WrongHeight {
                expected,
                got: b.height,
            });
        }
        if b.prev_id != self.tip {
            return Err(EconError::WrongParent);
        }
        if b.proposer != proposer {
            return Err(EconError::BadProposer);
        }
        if self.validator_set()?.power_of(&b.proposer).is_none() {
            return Err(EconError::NotAValidator);
        }
        if !self.blocks.is_empty() && b.timestamp <= self.last_timestamp {
            return Err(EconError::BadTime);
        }
        if b.txs.len() > MAX_BLOCK_TXS {
            return Err(EconError::BadTx("too many transactions".into()));
        }
        let txs: Vec<Tx> = b
            .txs
            .iter()
            .map(|t| Tx::from_bytes(t).map_err(|e| EconError::BadTx(format!("{e:?}"))))
            .collect::<Result<_, _>>()?;
        let owner = teko_coin::owner_of(&self.ledger, &b.proposer);
        let (root, _) = self
            .ledger
            .check_block(b.height, &owner, &txs)
            .map_err(|e| EconError::BadTx(format!("{e:?}")))?;
        if root.as_slice() != b.state_root.as_slice() {
            return Err(EconError::StateRoot);
        }
        Ok(b)
    }

    /// Builds the payload this node proposes: the valid prefix of `mempool`, the state
    /// root it reaches, proposed by `proposer` (this node's CONSENSUS key).
    pub fn propose(&self, proposer: &[u8], mempool: &[Tx], timestamp: u64) -> Vec<u8> {
        let owner = teko_coin::owner_of(&self.ledger, proposer);
        let mut kept = self.ledger.select_valid(&owner, mempool.to_vec());
        kept.truncate(MAX_BLOCK_TXS);
        let height = self.height() + 1;
        let root = self
            .ledger
            .check_block(height, &owner, &kept)
            .map(|(r, _)| r.to_vec())
            .unwrap_or_default();
        EconBlock {
            height,
            prev_id: self.tip.clone(),
            proposer: proposer.to_vec(),
            timestamp,
            txs: kept.iter().map(Tx::to_bytes).collect(),
            state_root: root,
        }
        .encode_to_vec()
    }

    /// Accepts a FINAL block: the certificate must be valid for the validator set of
    /// this height, name the right proposer, and the block must execute to its root.
    pub fn apply_decided(&mut self, d: DecidedBlock) -> Result<(), EconError> {
        self.apply_decided_outcome(d).map(|_| ())
    }

    /// Like `apply_decided`, returning what the block did (receipts and events), for
    /// indexers such as the explorer.
    pub fn apply_decided_outcome(&mut self, d: DecidedBlock) -> Result<BlockOutcome, EconError> {
        let block = d.block()?;
        let commit = d.commit.as_ref().ok_or(EconError::Commit)?;
        let set = self.validator_set()?;
        if commit.height != block.height
            || commit.block_id != block_id(&d.payload)
            || !commit.verify(&self.genesis.chain_id, &set)
        {
            return Err(EconError::Commit);
        }
        if set.proposer(commit.height, commit.round).key != block.proposer {
            return Err(EconError::BadProposer);
        }
        let checked = self.check(&block.proposer, &d.payload)?;
        let txs: Vec<Tx> = checked
            .txs
            .iter()
            .map(|t| Tx::from_bytes(t).map_err(|e| EconError::BadTx(format!("{e:?}"))))
            .collect::<Result<_, _>>()?;
        let owner = teko_coin::owner_of(&self.ledger, &checked.proposer);
        let out = self
            .ledger
            .apply_block(checked.height, &owner, &txs)
            .map_err(|e| EconError::BadTx(format!("{e:?}")))?;
        if out.state_root.as_slice() != checked.state_root.as_slice() {
            return Err(EconError::StateRoot);
        }
        self.tip = block_id(&d.payload);
        self.last_timestamp = checked.timestamp;
        self.blocks.push(d);
        Ok(out)
    }

    /// Time of the last final block (0 before the first).
    pub fn last_timestamp(&self) -> u64 {
        self.last_timestamp
    }

    /// Owner address of a consensus key (where its rewards go).
    pub fn owner_of(&self, key: &[u8]) -> Address {
        teko_coin::owner_of(&self.ledger, key)
    }
}
