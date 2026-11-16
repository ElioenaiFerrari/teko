//! Election manifests and lifecycle (Phase 2, ADR-0016).
//!
//! - An **election** holds several **contests** (President, Governor, Mayor...),
//!   each with its OWN jurisdiction, candidates, dates and lifecycle. The
//!   manifest is signed M-of-N; the device verifies it and shows candidate names
//!   FROM THE MANIFEST, never from the host.
//! - Each contest's **lifecycle** is a forward-only state machine; every
//!   transition is itself signed M-of-N and bound to that exact contest.
//! - **Eligibility** follows the jurisdiction: the President is national, a
//!   governor state-wide, a mayor municipal.
//!
//! No voting cryptography lives here (registration, proofs: ADR-0008/0013).
#![cfg_attr(not(test), no_std)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

extern crate alloc;

mod ballot;
mod eligibility;
mod error;
mod lifecycle;
mod manifest;
mod outcome;
mod policy;
mod registration;
mod result;
mod tally;
mod text;

pub use ballot::{
    BallotBody, ElectionBundle, ballot_ctx, ciphertext_hash, sign_ballot, verify_ballot,
};
pub use eligibility::{Domicile, Scope, ScopeLevel, is_eligible};
pub use error::ElectionError;
pub use lifecycle::{
    ContestState, ElectionStatus, Phase, Transition, replay, replay_chain, sign_transition,
    transition_hash, verify_transition,
};
pub use manifest::{
    BALLOT_KIND_PUBLIC, Contest, ElectionManifest, ElectionOption, FORMAT_VERSION, Office,
    ResultsVisibility, Schedule, contest_hash, manifest_hash, sign_manifest, validate_manifest,
    validate_revision, verify_manifest, verify_manifest_chain,
};
pub use outcome::{
    ElectionOutcome, OutcomeKind, SignedOutcome, count_outcome_signers, decide, outcome_bytes,
    sign_outcome,
};
pub use policy::GovernancePolicy;
pub use registration::{
    RegistrationBlinder, RegistrationVerifier, SubmittedRegistration, registration_message,
    registration_request_message,
};
pub use result::{DecryptionShares, build_shares, decrypt_ctx};
pub use tally::{
    BallotCheck, BallotVerdict, ContestTally, ElectionTally, LogEntry, LogKind, OptionTally,
    Precheck, PublicView, Ranking, RegistrationCheck, RejectCounts, SubmittedBallot, Tallier,
    precheck, precheck_registration, tally, tally_with,
};
pub use text::sanitize_text;
