use alloc::string::String;
use alloc::vec::Vec;

use prost::Message;
use teko_crypto::{SigningKey, sign_tagged, tags, verify_tagged};
use thiserror::Error;

use crate::trust::TrustPolicy;

/// Batch certificate: the authority root vouches for a batch key shared by
/// MANY devices (FIDO-style batch attestation). Contains no per-device data.
#[derive(Clone, PartialEq, Message)]
pub struct BatchCert {
    #[prost(string, tag = "1")]
    pub batch_id: String,
    #[prost(bytes = "vec", tag = "2")]
    pub public_key: Vec<u8>,
    #[prost(bytes = "vec", tag = "3")]
    pub signature: Vec<u8>,
}

/// Factory-provisioned attestation material burned into each device of a batch.
/// SECRET (contains the batch private key); never leaves the factory/device.
#[derive(Clone, PartialEq, Message)]
pub struct FactoryData {
    #[prost(bytes = "vec", tag = "1")]
    pub batch_key: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    pub batch_cert: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedBatch {
    pub batch_id: String,
    pub public_key: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AttestationError {
    #[error("malformed certificate chain")]
    MalformedChain,
    #[error("certificate not signed by the trusted authority root")]
    UntrustedRoot,
    #[error("device batch `{0}` has been revoked")]
    Revoked(String),
    #[error("invalid attestation signature")]
    BadSignature,
}

fn cert_signing_bytes(c: &BatchCert) -> Vec<u8> {
    let mut unsigned = c.clone();
    unsigned.signature = Vec::new();
    unsigned.encode_to_vec()
}

pub fn issue_batch_cert(
    root_key: &SigningKey,
    batch_id: &str,
    batch_public_key: &[u8],
) -> BatchCert {
    let mut cert = BatchCert {
        batch_id: String::from(batch_id),
        public_key: batch_public_key.to_vec(),
        signature: Vec::new(),
    };
    cert.signature = sign_tagged(root_key, tags::ATTEST_CERT, &cert_signing_bytes(&cert));
    cert
}

/// Verifies `chain` (= [batch_cert]) up to the pinned authority root.
pub fn verify_batch_chain(
    policy: &TrustPolicy,
    chain: &[Vec<u8>],
) -> Result<VerifiedBatch, AttestationError> {
    let [raw] = chain else {
        return Err(AttestationError::MalformedChain);
    };
    let cert = BatchCert::decode(raw.as_slice()).map_err(|_| AttestationError::MalformedChain)?;
    verify_tagged(
        &policy.authority_root,
        tags::ATTEST_CERT,
        &cert_signing_bytes(&cert),
        &cert.signature,
    )
    .map_err(|_| AttestationError::UntrustedRoot)?;
    if policy.is_revoked(&cert.batch_id) {
        return Err(AttestationError::Revoked(cert.batch_id));
    }
    Ok(VerifiedBatch {
        batch_id: cert.batch_id,
        public_key: cert.public_key,
    })
}

/// What the device signs for an `AttestationChallenge`: the host nonce bound to
/// the reported firmware version and state. Never includes user data or votes.
pub fn attestation_message(nonce: &[u8], fw_version: u32, state: i32) -> Vec<u8> {
    let mut m = Vec::with_capacity(nonce.len() + 8);
    m.extend_from_slice(&fw_version.to_be_bytes());
    m.extend_from_slice(&state.to_be_bytes());
    m.extend_from_slice(nonce);
    m
}

pub fn sign_attestation(
    batch_key: &SigningKey,
    nonce: &[u8],
    fw_version: u32,
    state: i32,
) -> Vec<u8> {
    sign_tagged(
        batch_key,
        tags::ATTESTATION,
        &attestation_message(nonce, fw_version, state),
    )
}

pub fn verify_attestation(
    policy: &TrustPolicy,
    nonce: &[u8],
    fw_version: u32,
    state: i32,
    chain: &[Vec<u8>],
    signature: &[u8],
) -> Result<VerifiedBatch, AttestationError> {
    let batch = verify_batch_chain(policy, chain)?;
    verify_tagged(
        &batch.public_key,
        tags::ATTESTATION,
        &attestation_message(nonce, fw_version, state),
        signature,
    )
    .map_err(|_| AttestationError::BadSignature)?;
    Ok(batch)
}

/// Message signed (a) by the identity key as proof of possession and (b) by the
/// batch key as attestation, during credential issuance.
pub fn issue_request_message(authority_nonce: &[u8], identity_pk: &[u8]) -> Vec<u8> {
    let mut m = Vec::with_capacity(authority_nonce.len() + identity_pk.len() + 8);
    m.extend_from_slice(&(authority_nonce.len() as u32).to_be_bytes());
    m.extend_from_slice(authority_nonce);
    m.extend_from_slice(identity_pk);
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use rand::SeedableRng;
    use rand::rngs::StdRng;
    use teko_crypto::{generate_signing_key, public_key_bytes};

    fn world(seed: u64) -> (SigningKey, SigningKey, TrustPolicy, Vec<Vec<u8>>) {
        let mut rng = StdRng::seed_from_u64(seed);
        let root = generate_signing_key(&mut rng);
        let batch = generate_signing_key(&mut rng);
        let cert = issue_batch_cert(&root, "lote-001", &public_key_bytes(&batch));
        let policy = TrustPolicy {
            authority_root: public_key_bytes(&root),
            fw_signers: alloc::vec![public_key_bytes(&root)],
            fw_threshold: 1,
            revoked_batches: Vec::new(),
            ..TrustPolicy::default()
        };
        (root, batch, policy, alloc::vec![cert.encode_to_vec()])
    }

    proptest! {
        #[test]
        fn genuine_device_verifies_and_nonce_is_bound(seed in any::<u64>(), nonce in proptest::collection::vec(any::<u8>(), 16..48), fw in any::<u32>()) {
            let (_, batch, policy, chain) = world(seed);
            let sig = sign_attestation(&batch, &nonce, fw, 1);
            let v = verify_attestation(&policy, &nonce, fw, 1, &chain, &sig).unwrap();
            prop_assert_eq!(v.batch_id, "lote-001");
            let mut other = nonce.clone();
            other[0] ^= 1;
            prop_assert!(verify_attestation(&policy, &other, fw, 1, &chain, &sig).is_err());
            prop_assert!(verify_attestation(&policy, &nonce, fw.wrapping_add(1), 1, &chain, &sig).is_err());
        }

        #[test]
        fn forged_chain_is_rejected(seed in any::<u64>(), fake in any::<u64>()) {
            let (_, _, policy, _) = world(seed);
            prop_assume!(fake != seed);
            let (_, fake_batch, _, fake_chain) = world(fake);
            let sig = sign_attestation(&fake_batch, b"n", 1, 1);
            prop_assert!(verify_attestation(&policy, b"n", 1, 1, &fake_chain, &sig).is_err());
        }
    }

    #[test]
    fn revoked_batch_is_rejected() {
        let (_, batch, mut policy, chain) = world(5);
        policy.revoked_batches.push(String::from("lote-001"));
        let sig = sign_attestation(&batch, b"n", 1, 1);
        assert_eq!(
            verify_attestation(&policy, b"n", 1, 1, &chain, &sig),
            Err(AttestationError::Revoked(String::from("lote-001")))
        );
    }

    #[test]
    fn malformed_chain_never_panics() {
        let (_, _, policy, _) = world(1);
        assert!(verify_batch_chain(&policy, &[]).is_err());
        assert!(verify_batch_chain(&policy, &[alloc::vec![1, 2, 3]]).is_err());
    }
}
