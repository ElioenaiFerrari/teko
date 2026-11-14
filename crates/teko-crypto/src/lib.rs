//! Teko cryptography: domain-separated hashing, key derivation, civic/pairwise
//! IDs (bech32m), signatures (P-256, ADR-0002) and credentials with selective
//! disclosure (ADR-0005, simplified SD-JWT-style).
//!
//! STUBS (documented, to be replaced in the voting phase): the voter
//! commitment/nullifier use SHA3 instead of Poseidon; no ZK proofs.
#![cfg_attr(not(test), no_std)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

extern crate alloc;

pub mod credential;
mod error;
pub mod hex;
mod ids;
mod keys;
mod sign;
pub mod tags;
pub mod voter;

pub use error::CryptoError;
pub use ids::{
    CIVIC_HRP, PAIRWISE_HRP, canonical_service_id, civic_id, pairwise_id, parse_civic_id,
};
pub use keys::{
    derive_key, derive_signing_key, generate_signing_key, public_key_bytes, tagged_hash,
    verifying_key_from_bytes,
};
pub use sign::{sign_tagged, verify_tagged};

pub use p256::ecdsa::{SigningKey, VerifyingKey};
