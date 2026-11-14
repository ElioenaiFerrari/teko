//! Bulletin board (ADR-0014): an append-only, publicly verifiable Merkle log
//! behind a `BulletinBoard` trait, so a blockchain backend can replace it later
//! without touching the cryptography.
//!
//! Design follows the Merkle trees of Certificate Transparency (RFC 6962 /
//! RFC 9162): inclusion proofs ("this entry is in the log") and consistency
//! proofs ("the log only grew, nothing was rewritten"). Hashing is
//! domain-separated with Teko tags instead of the RFC's 0x00/0x01 prefixes.
//!
//! A Merkle root does NOT commit to the tree size; the size is authenticated by
//! the operator's signed head `(size, root)`, so always verify proofs against a
//! `SignedHead`, not a bare root.
//!
//! This is a single-operator log. Without independent witnesses comparing
//! signed heads, an operator could show different logs to different people
//! (split view): see ADR-0014 open questions.

mod board;
mod error;
pub mod merkle;

pub use board::{
    BulletinBoard, Entry, EntryKind, FileBoard, MemoryBoard, Receipt, SignedHead, sign_head,
    verify_entry_inclusion, verify_head,
};
pub use error::BoardError;
pub use merkle::Hash;
