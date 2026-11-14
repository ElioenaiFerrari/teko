//! Election manifests and lifecycle (Phase 2, ADR-0016).
//!
//! - A **manifest** (candidates, schedule, rules) is signed M-of-N by the
//!   governing institutions. The device verifies it and shows candidate names
//!   FROM THE MANIFEST, never from the host.
//! - The **lifecycle** is a forward-only state machine; every transition is
//!   itself a message signed M-of-N and bound to one manifest.
//!
//! No voting cryptography lives here (registration, proofs: ADR-0008/0013).
#![cfg_attr(not(test), no_std)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

extern crate alloc;

mod error;
mod lifecycle;
mod manifest;
mod policy;
mod text;

pub use error::ElectionError;
pub use lifecycle::{
    ElectionState, Phase, Transition, replay, sign_transition, transition_hash, verify_transition,
};
pub use manifest::{
    BALLOT_KIND_PUBLIC, ElectionManifest, ElectionOption, Schedule, manifest_hash, sign_manifest,
    validate_manifest, verify_manifest,
};
pub use policy::GovernancePolicy;
pub use text::sanitize_text;
