use thiserror::Error;

use crate::lifecycle::Phase;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ElectionError {
    #[error("invalid manifest: {0}")]
    InvalidManifest(&'static str),
    #[error("invalid governance policy: {0}")]
    InvalidPolicy(&'static str),
    #[error("insufficient signatures: {valid} valid of {required} required")]
    InsufficientSignatures { valid: usize, required: usize },
    #[error("transition belongs to another election")]
    WrongElection,
    #[error("transition is bound to a different manifest")]
    WrongManifest,
    #[error("bad sequence number: expected {expected}, got {got}")]
    BadSequence { expected: u64, got: u64 },
    #[error("illegal transition {from:?} -> {to:?}")]
    IllegalTransition { from: Phase, to: Phase },
    #[error("transition does not start from the current phase")]
    WrongFromPhase,
    #[error("timestamp goes backwards")]
    TimeTravel,
    #[error("too early for this phase according to the schedule")]
    TooEarly,
    #[error("a reason is required to cancel")]
    MissingReason,
    #[error("unknown phase value")]
    UnknownPhase,
}
