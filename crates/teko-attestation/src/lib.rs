//! Firmware manifests (M-of-N, anti-downgrade), device attestation chains
//! (batch keys, ADR-0007) and the trust policy shared by bootloader, Suite
//! and issuer.
#![cfg_attr(not(test), no_std)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

extern crate alloc;

mod attestation;
mod firmware;
pub mod multisig;
mod trust;

pub use attestation::{
    AttestationError, BatchCert, FactoryData, VerifiedBatch, attestation_message, issue_batch_cert,
    issue_request_message, sign_attestation, verify_attestation, verify_batch_chain,
};
pub use firmware::{
    BlobHasher, FirmwareError, FirmwareManifest, SignerSignature, blob_hash, count_valid_signers,
    manifest_fingerprint, sign_manifest, verify_blob, verify_manifest,
};
pub use trust::{TrustError, TrustPolicy};
