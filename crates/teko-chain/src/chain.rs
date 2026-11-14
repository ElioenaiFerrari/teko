use teko_attestation::SignerSignature;
use teko_attestation::multisig::count_distinct_valid;
use teko_bulletin::merkle::Frontier;
use teko_bulletin::{BulletinBoard, Entry, MemoryBoard};
use teko_crypto::{SigningKey, public_key_bytes, sign_tagged, tags};

use crate::block::{
    Block, BlockEntry, MAX_BLOCK_BYTES, MAX_BLOCK_ENTRIES, block_hash, genesis_hash,
};
use crate::error::ChainError;
use crate::profile::NetworkProfile;

const MAX_ENTRY_PAYLOAD: usize = 1024 * 1024;

/// A verified replica: the blocks, plus the bulletin log they rebuild. Nothing
/// enters except through `apply`, which checks everything.
pub struct Chain {
    profile: NetworkProfile,
    blocks: Vec<Block>,
    board: MemoryBoard,
    frontier: Frontier,
}

impl Chain {
    pub fn new(profile: NetworkProfile) -> Self {
        Self {
            profile,
            blocks: Vec::new(),
            board: MemoryBoard::new(),
            frontier: Frontier::new(),
        }
    }

    pub fn profile(&self) -> &NetworkProfile {
        &self.profile
    }

    /// Number of blocks (= the next block's height).
    pub fn height(&self) -> u64 {
        self.blocks.len() as u64
    }

    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    pub fn blocks_from(&self, height: u64) -> &[Block] {
        let h = usize::try_from(height)
            .unwrap_or(usize::MAX)
            .min(self.blocks.len());
        &self.blocks[h..]
    }

    /// The bulletin log rebuilt from the blocks (read-only).
    pub fn board(&self) -> &MemoryBoard {
        &self.board
    }

    /// Consumes the chain, keeping the verified Merkle log.
    pub fn into_board(self) -> MemoryBoard {
        self.board
    }

    pub fn tip_hash(&self) -> [u8; 32] {
        self.blocks
            .last()
            .map_or_else(|| genesis_hash(&self.profile.chain_id), block_hash)
    }

    /// Round-robin proposer: validator `(height + round) % N`. Round 0 is the normal
    /// turn; a higher round is a view change when that proposer stays silent.
    pub fn proposer_for(&self, height: u64, round: u32) -> &[u8] {
        let n = self.profile.validators.len().max(1) as u64;
        self.profile
            .validators
            .get(((height + u64::from(round)) % n) as usize)
            .map_or(&[][..], Vec::as_slice)
    }

    /// Builds the UNSIGNED next block for `entries`, with the exact Merkle state
    /// that appending them would produce.
    pub fn propose(&self, entries: &[Entry], timestamp: u64, round: u32) -> Block {
        let mut f = self.frontier.clone();
        for e in entries {
            f.push(e.leaf_hash());
        }
        Block {
            chain_id: self.profile.chain_id.clone(),
            height: self.height(),
            prev_hash: self.tip_hash().to_vec(),
            timestamp,
            proposer: self.proposer_for(self.height(), round).to_vec(),
            round,
            entries: entries.iter().map(BlockEntry::from).collect(),
            log_size: self.board.len() + entries.len() as u64,
            log_root: f.root().to_vec(),
            signatures: Vec::new(),
        }
    }

    /// Everything about a block EXCEPT the signature count: network, position in
    /// the chain, proposer's turn and the exact Merkle state it claims.
    pub fn check_content(&self, b: &Block) -> Result<Vec<Entry>, ChainError> {
        let bad = ChainError::Block;
        if b.chain_id != self.profile.chain_id {
            return Err(bad("other chain_id"));
        }
        if b.height != self.height() {
            return Err(bad("height is not the next one"));
        }
        if b.prev_hash.as_slice() != self.tip_hash().as_slice() {
            return Err(bad("prev_hash does not match the tip"));
        }
        if b.proposer.as_slice() != self.proposer_for(b.height, b.round) {
            return Err(bad("not this proposer's turn"));
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
        let mut f = self.frontier.clone();
        for e in &entries {
            f.push(e.leaf_hash());
        }
        if b.log_size != self.board.len() + entries.len() as u64
            || b.log_root.as_slice() != f.root().as_slice()
        {
            return Err(bad("log size/root do not match the entries"));
        }
        Ok(entries)
    }

    /// A validator's signature over a block it has CHECKED. Refuses anything
    /// that does not verify, so a validator never signs a bad block.
    pub fn sign(&self, key: &SigningKey, b: &Block) -> Result<SignerSignature, ChainError> {
        self.check_content(b)?;
        let me = public_key_bytes(key);
        if !self.profile.validators.contains(&me) {
            return Err(ChainError::Block("signer is not a validator"));
        }
        Ok(SignerSignature {
            signer: me,
            signature: sign_tagged(key, tags::BLOCK, &b.header_hash()),
        })
    }

    pub fn count_signatures(&self, b: &Block) -> usize {
        count_distinct_valid(
            &self.profile.validators,
            tags::BLOCK,
            &b.header_hash(),
            &b.signatures,
        )
    }

    /// Verifies a block completely (content + M distinct validator signatures,
    /// the proposer's among them) and commits it.
    pub fn apply(&mut self, b: Block) -> Result<(), ChainError> {
        let entries = self.check_content(&b)?;
        let got = self.count_signatures(&b);
        if got < self.profile.threshold {
            return Err(ChainError::Signatures {
                got,
                need: self.profile.threshold,
            });
        }
        let proposer_signed = b.signatures.iter().any(|s| {
            s.signer == b.proposer
                && count_distinct_valid(
                    std::slice::from_ref(&s.signer),
                    tags::BLOCK,
                    &b.header_hash(),
                    std::slice::from_ref(s),
                ) == 1
        });
        if !proposer_signed {
            return Err(ChainError::Block("the proposer did not sign its own block"));
        }
        for e in &entries {
            self.frontier.push(e.leaf_hash());
            self.board
                .append(e.clone())
                .map_err(|e| ChainError::Log(e.to_string()))?;
        }
        self.blocks.push(b);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::NetworkKind;
    use rand::SeedableRng;
    use rand::rngs::StdRng;
    use teko_bulletin::EntryKind;
    use teko_crypto::generate_signing_key;

    fn setup(kind: NetworkKind) -> (Vec<SigningKey>, Chain) {
        let mut rng = StdRng::seed_from_u64(9);
        let keys: Vec<SigningKey> = (0..3).map(|_| generate_signing_key(&mut rng)).collect();
        let pubs = keys.iter().map(public_key_bytes).collect();
        (
            keys,
            Chain::new(NetworkProfile::new(kind, pubs, 2).unwrap()),
        )
    }

    fn entries(n: u8) -> Vec<Entry> {
        (0..n)
            .map(|i| Entry::new(EntryKind::BALLOT, vec![i; 40]))
            .collect()
    }

    /// A fully signed block by validators `who` (the proposer is always included).
    fn signed(c: &Chain, keys: &[SigningKey], es: &[Entry], who: &[usize]) -> Block {
        let h = c.height();
        let mut b = c.propose(es, 1_000 + h, 0);
        for i in who {
            b.signatures.push(c.sign(&keys[*i], &b).unwrap());
        }
        b
    }

    #[test]
    fn blocks_chain_and_rebuild_exactly_the_same_merkle_log() {
        let (k, mut c) = setup(NetworkKind::Devnet);
        let mut reference = MemoryBoard::new();
        for round in 0..5u8 {
            let es = entries(round + 1);
            let p = (c.height() % 3) as usize;
            let b = signed(&c, &k, &es, &[p, (p + 1) % 3]);
            c.apply(b).unwrap();
            for e in es {
                reference.append(e).unwrap();
            }
            assert_eq!(c.board().root(), reference.root(), "round {round}");
        }
        assert_eq!((c.height(), c.board().len()), (5, 15));
        // A second replica fed only the blocks reaches the same state, verifying all.
        let mut other = Chain::new(c.profile().clone());
        for b in c.blocks() {
            other.apply(b.clone()).unwrap();
        }
        assert_eq!(other.board().root(), c.board().root());
        assert_eq!(other.tip_hash(), c.tip_hash());
    }

    #[test]
    fn a_block_needs_m_distinct_validators_including_the_proposer() {
        let (k, mut c) = setup(NetworkKind::Devnet);
        let es = entries(2);
        // only the proposer: 1 of 2
        let b = signed(&c, &k, &es, &[0]);
        assert_eq!(
            c.apply(b.clone()),
            Err(ChainError::Signatures { got: 1, need: 2 })
        );
        // the same signature twice still counts once
        let mut dup = b.clone();
        dup.signatures.push(dup.signatures[0].clone());
        assert!(matches!(
            c.apply(dup),
            Err(ChainError::Signatures { got: 1, .. })
        ));
        // two validators that are NOT the proposer: enough signatures, but the proposer is missing
        let mut nop = c.propose(&es, 1_000, 0);
        for i in [1usize, 2] {
            nop.signatures.push(c.sign(&k[i], &nop).unwrap());
        }
        assert_eq!(
            c.apply(nop),
            Err(ChainError::Block("the proposer did not sign its own block"))
        );
        // an outsider's signature counts for nothing
        let mut rng = StdRng::seed_from_u64(77);
        let outsider = generate_signing_key(&mut rng);
        let mut out = c.propose(&es, 1_000, 0);
        out.signatures.push(c.sign(&k[0], &out).unwrap());
        out.signatures.push(SignerSignature {
            signer: public_key_bytes(&outsider),
            signature: sign_tagged(&outsider, tags::BLOCK, &out.header_hash()),
        });
        assert!(matches!(
            c.apply(out),
            Err(ChainError::Signatures { got: 1, .. })
        ));
        assert_eq!(c.height(), 0, "nothing was committed");
        // 2 of 3 including the proposer: accepted
        c.apply(signed(&c, &k, &es, &[0, 2])).unwrap();
        assert_eq!(c.height(), 1);
    }

    #[test]
    fn every_tampered_field_is_refused() {
        let (k, mut c) = setup(NetworkKind::Devnet);
        c.apply(signed(&c, &k, &entries(2), &[0, 1])).unwrap();
        let p = (c.height() % 3) as usize;
        let good = signed(&c, &k, &entries(3), &[p, (p + 1) % 3]);
        let check = |name: &str, f: &dyn Fn(&mut Block)| {
            let mut b = good.clone();
            f(&mut b);
            assert!(c.clone_apply(b).is_err(), "{name} must be refused");
        };
        check("chain_id", &|b| b.chain_id = "teko-other-1".into());
        check("height", &|b| b.height += 1);
        check("prev_hash", &|b| b.prev_hash[0] ^= 1);
        check("proposer", &|b| b.proposer = b.signatures[1].signer.clone());
        check("entry", &|b| b.entries[0].payload[0] ^= 1);
        check("extra entry", &|b| b.entries.push(b.entries[0].clone()));
        check("no entries", &|b| b.entries.clear());
        check("log_root", &|b| b.log_root[0] ^= 1);
        check("log_size", &|b| b.log_size += 1);
        check("timestamp (signed)", &|b| b.timestamp += 1);
        check("entry kind", &|b| b.entries[0].kind = 300);
        c.apply(good).unwrap();
    }

    #[test]
    fn a_validator_never_signs_a_bad_block_or_one_it_is_not_part_of() {
        let (k, c) = setup(NetworkKind::Devnet);
        let mut b = c.propose(&entries(1), 1, 0);
        b.log_root[0] ^= 1;
        assert!(c.sign(&k[0], &b).is_err());
        let mut rng = StdRng::seed_from_u64(5);
        let outsider = generate_signing_key(&mut rng);
        let ok = c.propose(&entries(1), 1, 0);
        assert!(c.sign(&outsider, &ok).is_err());
    }

    #[test]
    fn a_block_from_another_network_is_useless_here() {
        let (k, mut dev) = setup(NetworkKind::Devnet);
        let (_, mut test) = setup(NetworkKind::Testnet); // same keys, other chain_id
        let b = signed(&dev, &k, &entries(2), &[0, 1]);
        assert!(test.apply(b.clone()).is_err(), "devnet block on testnet");
        dev.apply(b).unwrap();
    }

    #[test]
    fn conflicting_blocks_for_one_height_are_both_individually_valid_but_only_one_commits() {
        // Equivocation: two different blocks, same height, same proposer.
        let (k, mut c) = setup(NetworkKind::Devnet);
        let a = signed(&c, &k, &entries(2), &[0, 1]);
        let b = signed(&c, &k, &entries(3), &[0, 2]);
        assert_ne!(a.header_hash(), b.header_hash());
        c.apply(a).unwrap();
        assert!(
            c.apply(b).is_err(),
            "the second one no longer fits the chain"
        );
    }

    impl Chain {
        /// Test helper: would this block be accepted? (without committing)
        fn clone_apply(&self, b: Block) -> Result<(), ChainError> {
            let mut copy = Chain::new(self.profile.clone());
            for x in &self.blocks {
                copy.apply(x.clone())?;
            }
            copy.apply(b)
        }
    }
}
