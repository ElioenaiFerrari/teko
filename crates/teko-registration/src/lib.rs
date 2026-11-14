//! Anonymous voter registration with RFC 9474 blind RSA signatures (ADR-0008).
//!
//! The registration authority signs a BLINDED message, so it never sees the
//! voter's public key; the voter unblinds the signature and shows
//! `(key, signature)` to the registry. The registry (and anyone) can verify the
//! signature, but nobody can link it to the issuance.
//!
//! Variant: **RSABSSA-SHA384-PSS-Randomized** (RFC 9474 §5; the RECOMMENDED one):
//! SHA-384, MGF1-SHA-384, 48-byte salt, 32-byte random message prefix.
//!
//! WHY THIS IS HERE AND NOT A LIBRARY: the maintained crate
//! (`blind-rsa-signatures` 0.18) targets `rsa` 0.10 *release candidates*; the
//! advisory RUSTSEC-2023-0071 (timing side channel in RSA private-key
//! operations) is only fixed from `rsa` 0.10.0 final, which that crate does not
//! compile against. This implementation follows the RFC step by step on top of
//! `rsa` 0.10.0's raw operations and is checked against the RFC's own test
//! vector (`tests/rfc9474.rs`). It is NOT independently audited.

mod bsrsa;

mod adapter;
pub use adapter::RsaRegistration;
pub use bsrsa::{
    BlindResult, BlindSignature, Blinded, IssuerKey, RegError, Token, blind, blind_deterministic,
    blind_with, emsa_pss_encode_for_tests, finalize, verify,
};
