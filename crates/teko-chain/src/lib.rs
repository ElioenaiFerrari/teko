//! Federated proof-of-authority chain for the Teko voting prototype (ADR-0014).
//!
//! WHAT IT IS: blocks of bulletin-board entries, each signed by M of N
//! validators (the independent institutions), chained by hash and bound to a
//! `chain_id`. Anyone can replay the blocks and rebuild the SAME Merkle log,
//! verifying every signature on the way (`Chain::apply`).
//!
//! WHAT IT IS NOT: Byzantine consensus. Validators sign only what they verified,
//! and two conflicting blocks for one height are DETECTED (equivocation), but a
//! malicious proposer can still delay or censor for its turn. No mining.
//! The `mainnet` profile is deliberately locked in this phase.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod block;
mod chain;
mod error;
mod profile;

pub use block::{Block, BlockEntry, block_hash, genesis_hash};
pub use chain::Chain;
pub use error::ChainError;
pub use profile::{NetworkKind, NetworkProfile};
