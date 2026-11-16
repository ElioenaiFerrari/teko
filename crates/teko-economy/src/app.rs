use std::collections::HashSet;

use teko_bft::App;
use teko_crypto::public_key_bytes;
use teko_ledger::{Tx, TxError};

use crate::chain::{DecidedBlock, EconError, EconomyChain, MAX_BLOCK_TXS};

/// Seconds a proposed block time may differ from the voter's own clock (ADR-0034).
pub const MAX_CLOCK_DRIFT_SECS: u64 = 60;

/// Largest mempool (bounds memory).
const MAX_MEMPOOL: usize = 20_000;

/// The application one node plugs into its BFT engine: the chain plus a mempool.
pub struct EconApp {
    pub chain: EconomyChain,
    mempool: Vec<Tx>,
    seen: HashSet<[u8; 32]>,
    /// This node's CONSENSUS public key.
    me: Vec<u8>,
    /// Unix seconds; injected so tests are deterministic.
    clock: Box<dyn Fn() -> u64 + Send>,
}

impl EconApp {
    pub fn new(
        chain: EconomyChain,
        key: &teko_crypto::SigningKey,
        clock: Box<dyn Fn() -> u64 + Send>,
    ) -> Self {
        Self {
            chain,
            mempool: Vec::new(),
            seen: HashSet::new(),
            me: public_key_bytes(key),
            clock,
        }
    }

    pub fn mempool_len(&self) -> usize {
        self.mempool.len()
    }

    /// How many transactions of `sender` are waiting in the mempool.
    pub fn queued_from(&self, sender: &teko_ledger::Address) -> u64 {
        self.mempool
            .iter()
            .filter(|t| &t.address() == sender)
            .count() as u64
    }

    /// Queues a transaction after the cheap admission checks. Returns false for a
    /// duplicate (already queued).
    pub fn submit(&mut self, tx: Tx) -> Result<bool, String> {
        let id = tx.id();
        if self.seen.contains(&id) {
            return Ok(false);
        }
        if self.mempool.len() >= MAX_MEMPOOL {
            return Err("mempool cheio".into());
        }
        // Several transactions of one sender may wait together: each must carry the next
        // nonce after the ones already queued.
        let sender = tx.address();
        let queued = self
            .mempool
            .iter()
            .filter(|t| t.address() == sender)
            .count() as u64;
        match self.chain.ledger().check_tx(&tx) {
            Ok(()) if queued == 0 => {}
            Err(TxError::BadNonce { expected, got }) if got == expected + queued && queued > 0 => {}
            Ok(()) => return Err("nonce já em uso por uma transação pendente".into()),
            Err(e) => return Err(format!("{e:?}")),
        }
        self.seen.insert(id);
        self.mempool.push(tx);
        Ok(true)
    }

    /// Applies a final block and drops the transactions it carried (and any that are
    /// no longer valid) from the mempool.
    pub fn on_decided(&mut self, d: DecidedBlock) -> Result<(), EconError> {
        self.chain.apply_decided(d)?;
        let ledger = self.chain.ledger();
        // Keep what can still be included: valid now, or waiting for an earlier nonce.
        let keep: Vec<Tx> = self
            .mempool
            .drain(..)
            .filter(|t| match ledger.check_tx(t) {
                Ok(()) => true,
                Err(TxError::BadNonce { expected, got }) => got > expected,
                Err(_) => false,
            })
            .collect();
        self.seen = keep.iter().map(Tx::id).collect();
        self.mempool = keep;
        Ok(())
    }
}

impl App for EconApp {
    fn build(&mut self, _height: u64) -> Vec<u8> {
        let take = self.mempool.len().min(MAX_BLOCK_TXS);
        // the proposer's clock, never at or before the previous block
        let ts = (self.clock)().max(self.chain.last_timestamp() + 1);
        self.chain.propose(&self.me, &self.mempool[..take], ts)
    }

    fn validate(&self, _height: u64, _payload: &[u8]) -> bool {
        // Never used: `validate_from` always applies (the engine calls it).
        false
    }

    fn validate_from(&self, height: u64, proposer: &[u8], payload: &[u8]) -> bool {
        if self.chain.height() + 1 != height {
            return false;
        }
        // besides the ledger rules, this validator only votes for a block time near ITS clock
        match self.chain.check(proposer, payload) {
            Ok(b) => (self.clock)().abs_diff(b.timestamp) <= MAX_CLOCK_DRIFT_SECS,
            Err(_) => false,
        }
    }
}
