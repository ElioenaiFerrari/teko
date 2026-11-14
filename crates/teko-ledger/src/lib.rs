//! Generic deterministic ledger (M2 of the ledger program, ADR-0020).
//!
//! A `Ledger` is a replicated state machine: it executes BLOCKS of signed
//! transactions against a key-value state and exposes a Merkle STATE ROOT that
//! consensus (`teko-bft`) agrees on. What a transaction DOES is decided by pluggable
//! `Module`s (coin, assets, ...); the core only provides:
//!   * the signed transaction envelope (`Tx`): chain id, per-account nonce, module id;
//!   * replay protection (nonce) and chain binding;
//!   * namespaced storage: a module can write only its own keys (it can read others');
//!   * ATOMIC blocks: if any transaction fails, nothing is applied;
//!   * a state root and inclusion proofs, so light clients need not replay.
//!
//! NOT AUDITED. Fees, stake and coin rules are NOT here: they belong to modules.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod ledger;
mod state;
mod tx;

pub use ledger::{
    BlockOutcome, Ctx, Event, Ledger, LedgerError, MAX_TXS_PER_BLOCK, Module, ModuleError, Receipt,
};
pub use state::{State, StateProof, verify_state_proof};
pub use tx::{Address, Hash, MAX_PAYLOAD, Tx, TxError, address_of};
