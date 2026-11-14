//! BFT consensus core: the Tendermint algorithm (Buchman, Kwon, Milosevic 2018),
//! stake-weighted, written SANS-I/O: the engine consumes messages and timeouts and
//! returns actions (broadcast, schedule a timeout, decide, report equivocation).
//! Networking, clocks and block contents live outside, so the whole protocol can be
//! tested in a deterministic simulator with delays, losses, partitions and
//! Byzantine validators.
//!
//! GUARANTEES (with less than 1/3 of the voting power faulty): SAFETY (no two
//! different blocks are ever decided at one height) always, LIVENESS once the
//! network is synchronous again. A decided block carries a COMMIT CERTIFICATE (more
//! than 2/3 of the power signed a precommit), so anyone can verify finality offline.
//! Two conflicting signed votes by one validator are EVIDENCE of equivocation: the
//! proof the economic layer uses to slash.
//!
//! NOT AUDITED. Heights are decided one at a time; catching up a node that missed a
//! height uses the commit certificate (`Commit::verify`) from another node.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod engine;
mod types;

pub use engine::{Action, App, Engine, Message, Step, timeout_ms};
pub use types::{
    BftError, Commit, Evidence, Proposal, Validator, ValidatorSet, Vote, VoteKind, block_id,
};
