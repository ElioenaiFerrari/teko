//! Credential module for `teko-ledger` (ADR-0037).
//!
//! * A credential TYPE (for example "cnh-b", "candidato") is created at genesis and governed by an
//!   issuing ORGAN: an M-of-N set of member keys (their device wallet keys).
//! * ISSUANCE is on the chain: a member PROPOSES (the proposal counts as the first approval),
//!   other members APPROVE, each with its OWN transaction signed on its device. At M approvals the
//!   credential is PENDING for the type's CONTEST WINDOW, inside which the quorum may VETO
//!   (public reason); afterwards it is ACTIVE until `valid_until`, unless the quorum REVOKES it.
//! * The window and the validity are measured in BLOCK TIME (`Ctx::time`), like the asset trades
//!   (ADR-0036): the status is computed, no transaction is needed to expire or activate.
//! * The holder is a per-type CREDENTIAL key. Private types (a driving licence) keep only a
//!   commitment on the chain; public types (candidate, elected) keep their claims in clear.
//!
//! LIMIT (the oracle problem): the ledger proves which organ members issued the credential, not
//! that what they attested is true. NOT AUDITED; no legal validity.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod candidate;
pub mod civic;
pub mod civil;
mod module;
mod query;
mod types;

pub use module::{
    CANDIDATE_TYPE, CREDENTIALS_MODULE, CredentialsModule, DEFAULT_CONTEST_SECS, ELECTED_TYPE,
    credential_id, genesis_system_type, genesis_type, genesis_vote_set, module, public_commitment,
};
pub use query::{credential, credentials_of, credentials_of_type, status, type_info, types};
pub use types::{ClaimElectedOp, CredRec, Op, ProposeOp, Status, TypeRec, VoteOp, VoteSet};
