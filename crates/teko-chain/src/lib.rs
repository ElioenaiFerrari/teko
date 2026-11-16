//! The Teko VOTE chain (ADR-0014, ADR-0034): bulletin-board entries ordered by Byzantine
//! consensus among the independent institutions.
//!
//! WHAT IT IS: blocks of bulletin-board entries, decided by the BFT engine (`teko-bft`) with
//! **equal voting power** (1 institution = 1 vote, no coin, no stake), each carrying a COMMIT
//! CERTIFICATE (more than 2/3 of the institutions), chained by hash and bound to a `chain_id`.
//! Anyone can replay the blocks and rebuild the SAME Merkle log, verifying every certificate on
//! the way (`Chain::apply`). A decided block is final; a proposer that censors loses its turn to
//! the next round; without a quorum the chain pauses, it never forks.
//!
//! WHAT IT DOES NOT DO: judge ballots. It only orders; the `Tallier` counts and rejects.
//! NOT AUDITED. The `mainnet` profile is deliberately locked in this phase.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod app;
mod block;
mod chain;
mod error;
mod profile;

pub use app::{MAX_CLOCK_DRIFT_SECS, VoteApp};
pub use block::{Block, BlockEntry, DecidedBlock, block_hash, genesis_hash};
pub use chain::Chain;
pub use error::ChainError;
pub use profile::{NetworkKind, NetworkProfile};
