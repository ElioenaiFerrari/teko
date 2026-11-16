use std::collections::HashSet;

use teko_bft::App;
use teko_bulletin::{BulletinBoard, Entry};
use teko_crypto::{SigningKey, public_key_bytes};

use crate::block::{Block, DecidedBlock};
use crate::chain::{Chain, MAX_ENTRY_PAYLOAD};
use crate::error::ChainError;

/// Seconds a proposed block time may differ from the voter's own clock (ADR-0034).
pub const MAX_CLOCK_DRIFT_SECS: u64 = 60;
const MAX_MEMPOOL: usize = 50_000;
const MAX_PER_BLOCK: usize = 1_000;

/// The application one institution's node plugs into its BFT engine: the chain plus a mempool of
/// bulletin entries. The validator ORDERS entries only; ballots are counted (and bad ones
/// rejected and counted) by the `Tallier`, so the log stays the source of truth (ADR-0034).
pub struct VoteApp {
    pub chain: Chain,
    mempool: Vec<Entry>,
    seen: HashSet<[u8; 32]>,
    me: Vec<u8>,
    clock: Box<dyn Fn() -> u64 + Send>,
}

impl VoteApp {
    pub fn new(chain: Chain, key: &SigningKey, clock: Box<dyn Fn() -> u64 + Send>) -> Self {
        let mut seen = HashSet::new();
        for i in 0..chain.board().len() {
            if let Some(e) = chain.board().entry(i) {
                seen.insert(e.leaf_hash());
            }
        }
        Self {
            chain,
            mempool: Vec::new(),
            seen,
            me: public_key_bytes(key),
            clock,
        }
    }

    pub fn mempool_len(&self) -> usize {
        self.mempool.len()
    }

    /// Queues an entry. `Ok(false)` for one already queued or already in the log.
    pub fn submit(&mut self, e: Entry) -> Result<bool, String> {
        if e.payload.len() > MAX_ENTRY_PAYLOAD {
            return Err("entry too large".into());
        }
        if self.mempool.len() >= MAX_MEMPOOL {
            return Err("mempool full".into());
        }
        if !self.seen.insert(e.leaf_hash()) {
            return Ok(false);
        }
        self.mempool.push(e);
        Ok(true)
    }

    /// Applies a final block and drops what it carried from the mempool.
    pub fn on_decided(&mut self, d: DecidedBlock) -> Result<(), ChainError> {
        let block = d.block()?;
        self.chain.apply(d)?;
        let included: HashSet<[u8; 32]> = block
            .entries
            .iter()
            .filter_map(|e| e.to_entry().ok().map(|x| x.leaf_hash()))
            .collect();
        self.seen.extend(included.iter().copied());
        self.mempool.retain(|e| !included.contains(&e.leaf_hash()));
        Ok(())
    }

    /// The block a final decision carried, for callers that need its time.
    pub fn last_block(&self) -> Option<Block> {
        self.chain.blocks().last().and_then(|d| d.block().ok())
    }
}

impl App for VoteApp {
    fn build(&mut self, _height: u64) -> Vec<u8> {
        let take: Vec<Entry> = self.mempool.iter().take(MAX_PER_BLOCK).cloned().collect();
        self.chain.propose(&self.me, &take, (self.clock)())
    }

    fn validate(&self, _height: u64, _payload: &[u8]) -> bool {
        // Never used: the engine calls `validate_from`.
        false
    }

    fn validate_from(&self, height: u64, proposer: &[u8], payload: &[u8]) -> bool {
        if self.chain.height() + 1 != height {
            return false;
        }
        // besides the chain rules, a validator only votes for a block time near ITS clock
        match self.chain.check(proposer, payload) {
            Ok((b, _)) => (self.clock)().abs_diff(b.timestamp) <= MAX_CLOCK_DRIFT_SECS,
            Err(_) => false,
        }
    }
}
