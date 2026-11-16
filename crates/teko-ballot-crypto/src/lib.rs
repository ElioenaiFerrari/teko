//! Encrypted ballots (ADR-0019): nobody can see partial results before the
//! voting closes, and no individual vote is ever opened.
//!
//! - Each ballot is a ONE-HOT vector of exponential-ElGamal ciphertexts on P-256
//!   (one per option) with proofs that every entry encrypts 0 or 1 and that the
//!   entries add up to exactly 1 (`ballot`).
//! - Ballots are added HOMOMORPHICALLY; only the per-option TOTALS are decrypted,
//!   by `threshold` of `n` validators, each with a proof that its share is
//!   correct (`elgamal`, `proofs`).
//! - The election key is made by a distributed key generation so no single
//!   validator ever holds it (`dkg`).
//!
//! Fiat-Shamir challenges bind the WHOLE statement (public key, every
//! ciphertext, and a caller context with election, contest and voter key): the
//! "weak Fiat-Shamir" mistake of early Helios allowed forged proofs.
//!
//! NOT AUDITED. Built on `p256` 0.14 because no audited threshold-ElGamal crate
//! for P-256 exists. Constant-time behaviour comes from `p256`; the discrete-log
//! search for the final small totals is variable-time on PUBLIC data only.
#![no_std]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

extern crate alloc;
#[cfg(test)]
extern crate std;

mod ballot;
mod codec;
pub mod dkg;
pub mod dkg_wire;
mod elgamal;
mod proofs;

pub use ballot::{
    EncryptedBallot, MAX_OPTIONS, encrypt_ballot, encrypt_ballot_with, open_ballot, sum_ballots,
    verify_ballot,
};
pub use codec::{point_from_bytes, point_to_bytes, scalar_from_bytes, scalar_to_bytes};
pub use elgamal::{
    Ciphertext, combine_shares, decrypt_share, encrypt_bit, lagrange_at_zero, solve_small_dlog,
};
pub use proofs::{BitProof, DecryptProof, SumProof, prove_decryption, verify_decryption};

pub use p256::{ProjectivePoint as Point, Scalar};
