//! The economy ledger (coin, assets, stake) on BFT consensus (M6a, ADR-0028).
//!
//! SANS-I/O: this crate holds the chain rules and the application the BFT `Engine`
//! drives; sockets, threads and files live in `teko-node`.
//!
//! * A BLOCK carries transactions, its proposer's CONSENSUS key and the state root the
//!   ledger reaches; it is final only with a COMMIT CERTIFICATE (more than 2/3 of the
//!   voting power).
//! * The validator set of height `h` is a function of the state after block `h-1`:
//!   the genesis founders during the founder phase, then the stakers.
//! * Rewards and fees go to the OWNER of the proposer's consensus key.
//! * A CLIENT verifies a whole chain without trusting any node: `EconomyChain::replay`.
//!
//! NOT AUDITED.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod app;
mod chain;
mod genesis;

pub use app::EconApp;
pub use chain::{DecidedBlock, EconBlock, EconError, EconomyChain, quantize};
pub use genesis::{ClassSpec, EconomyGenesis};
