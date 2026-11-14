use alloc::string::String;
use alloc::vec::Vec;

use prost::Message;
use sha3::{Digest, Sha3_256};
use teko_crypto::{SigningKey, hex, public_key_bytes, sign_tagged, tagged_hash, tags};
use thiserror::Error;

use crate::trust::TrustPolicy;

#[derive(Clone, PartialEq, Message)]
pub struct SignerSignature {
    /// SEC1 public key of the signer (must be one of the N policy signers).
    #[prost(bytes = "vec", tag = "1")]
    pub signer: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    pub signature: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
pub struct FirmwareManifest {
    #[prost(uint32, tag = "1")]
    pub version: u32,
    #[prost(uint64, tag = "2")]
    pub size: u64,
    #[prost(bytes = "vec", tag = "3")]
    pub hash: Vec<u8>,
    #[prost(message, repeated, tag = "4")]
    pub signatures: Vec<SignerSignature>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FirmwareError {
    #[error("malformed manifest")]
    Malformed,
    #[error("insufficient signatures: {valid} valid of {required} required")]
    InsufficientSignatures { valid: usize, required: usize },
    #[error("downgrade refused: {offered} < installed {installed}")]
    Downgrade { offered: u32, installed: u32 },
    #[error("firmware hash does not match manifest")]
    HashMismatch,
}

fn signing_bytes(m: &FirmwareManifest) -> Vec<u8> {
    let mut unsigned = m.clone();
    unsigned.signatures = Vec::new();
    unsigned.encode_to_vec()
}

/// Produces one signer's signature over (version, size, hash).
pub fn sign_manifest(key: &SigningKey, manifest: &FirmwareManifest) -> SignerSignature {
    SignerSignature {
        signer: public_key_bytes(key),
        signature: sign_tagged(key, tags::FW_MANIFEST, &signing_bytes(manifest)),
    }
}

/// Number of DISTINCT policy signers with a valid signature on `manifest`.
pub fn count_valid_signers(policy: &TrustPolicy, manifest: &FirmwareManifest) -> usize {
    crate::multisig::count_distinct_valid(
        &policy.fw_signers,
        tags::FW_MANIFEST,
        &signing_bytes(manifest),
        &manifest.signatures,
    )
}

/// Bootloader check. M distinct, known signers must have valid signatures
/// (duplicates and unknown keys count zero), then anti-downgrade.
pub fn verify_manifest(
    policy: &TrustPolicy,
    manifest: &FirmwareManifest,
    installed_version: u32,
) -> Result<(), FirmwareError> {
    if manifest.hash.len() != 32 {
        return Err(FirmwareError::Malformed);
    }
    let valid = count_valid_signers(policy, manifest);
    if valid < policy.fw_threshold {
        return Err(FirmwareError::InsufficientSignatures {
            valid,
            required: policy.fw_threshold,
        });
    }
    if manifest.version < installed_version {
        return Err(FirmwareError::Downgrade {
            offered: manifest.version,
            installed: installed_version,
        });
    }
    Ok(())
}

/// Incremental hash of the firmware blob (streams chunk by chunk).
pub struct BlobHasher(Sha3_256);

impl Default for BlobHasher {
    fn default() -> Self {
        Self::new()
    }
}

impl BlobHasher {
    pub fn new() -> Self {
        let mut h = Sha3_256::new();
        h.update((tags::FW_BLOB.len() as u32).to_be_bytes());
        h.update(tags::FW_BLOB.as_bytes());
        Self(h)
    }

    pub fn update(&mut self, data: &[u8]) {
        self.0.update(data);
    }

    pub fn finalize(self) -> [u8; 32] {
        self.0.finalize().into()
    }
}

pub fn blob_hash(blob: &[u8]) -> [u8; 32] {
    let mut h = BlobHasher::new();
    h.update(blob);
    h.finalize()
}

pub fn verify_blob(
    manifest: &FirmwareManifest,
    blob_digest: &[u8; 32],
) -> Result<(), FirmwareError> {
    if manifest.hash.as_slice() == blob_digest.as_slice() {
        Ok(())
    } else {
        Err(FirmwareError::HashMismatch)
    }
}

/// Short human-checkable fingerprint shown on BOTH the Suite and the device.
pub fn manifest_fingerprint(manifest: &FirmwareManifest) -> String {
    let d = tagged_hash(tags::FW_MANIFEST, &[&signing_bytes(manifest)]);
    let h = hex::encode(d.get(..8).unwrap_or(&d));
    let mut out = String::new();
    for (i, c) in h.chars().enumerate() {
        if i > 0 && i % 4 == 0 {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use rand::SeedableRng;
    use rand::rngs::StdRng;
    use teko_crypto::generate_signing_key;

    fn setup(seed: u64, threshold: usize) -> (Vec<SigningKey>, TrustPolicy, FirmwareManifest) {
        let mut rng = StdRng::seed_from_u64(seed);
        let keys: Vec<SigningKey> = (0..3).map(|_| generate_signing_key(&mut rng)).collect();
        let policy = TrustPolicy {
            authority_root: public_key_bytes(&keys[0]),
            fw_signers: keys.iter().map(public_key_bytes).collect(),
            fw_threshold: threshold,
            revoked_batches: Vec::new(),
            ..TrustPolicy::default()
        };
        let m = FirmwareManifest {
            version: 5,
            size: 10,
            hash: blob_hash(b"firmware").to_vec(),
            signatures: Vec::new(),
        };
        (keys, policy, m)
    }

    proptest! {
        #[test]
        fn accepted_iff_m_distinct_valid_signers(seed in any::<u64>(), mask in 0u8..8) {
            let (keys, policy, mut m) = setup(seed, 2);
            let sigs: Vec<_> = keys.iter().enumerate().filter(|(i, _)| mask & (1 << i) != 0)
                .map(|(_, k)| sign_manifest(k, &m)).collect();
            m.signatures = sigs;
            let ok = verify_manifest(&policy, &m, 1).is_ok();
            prop_assert_eq!(ok, mask.count_ones() >= 2);
        }

        #[test]
        fn duplicate_signatures_count_once(seed in any::<u64>(), copies in 2usize..6) {
            let (keys, policy, mut m) = setup(seed, 2);
            let s = sign_manifest(&keys[0], &m);
            m.signatures = alloc::vec![s; copies];
            let e = verify_manifest(&policy, &m, 1).unwrap_err();
            let one = matches!(e, FirmwareError::InsufficientSignatures { valid: 1, .. });
            prop_assert!(one);
        }

        #[test]
        fn downgrade_is_refused(seed in any::<u64>(), installed in 6u32..100) {
            let (keys, policy, mut m) = setup(seed, 2);
            m.signatures = keys.iter().take(2).map(|k| sign_manifest(k, &m)).collect();
            let e = verify_manifest(&policy, &m, installed).unwrap_err();
            let down = matches!(e, FirmwareError::Downgrade { .. });
            prop_assert!(down);
            prop_assert!(verify_manifest(&policy, &m, 5).is_ok());
        }

        #[test]
        fn tampering_invalidates_signatures(seed in any::<u64>(), new_version in 6u32..1000) {
            let (keys, policy, mut m) = setup(seed, 2);
            m.signatures = keys.iter().take(3).map(|k| sign_manifest(k, &m)).collect();
            m.version = new_version;
            prop_assert!(verify_manifest(&policy, &m, 0).is_err());
        }

        #[test]
        fn unknown_signers_do_not_count(seed in any::<u64>(), outsider in any::<u64>()) {
            let (keys, policy, mut m) = setup(seed, 2);
            let mut rng = StdRng::seed_from_u64(outsider ^ 0xDEAD_BEEF);
            let rogue = generate_signing_key(&mut rng);
            prop_assume!(public_key_bytes(&rogue) != public_key_bytes(&keys[0]));
            m.signatures = alloc::vec![sign_manifest(&keys[0], &m), sign_manifest(&rogue, &m)];
            prop_assert!(verify_manifest(&policy, &m, 0).is_err());
        }
    }

    #[test]
    fn blob_hash_streaming_equals_one_shot() {
        let mut h = BlobHasher::new();
        h.update(b"firm");
        h.update(b"ware");
        assert_eq!(h.finalize(), blob_hash(b"firmware"));
    }

    #[test]
    fn hash_mismatch_detected() {
        let (_, _, m) = setup(1, 2);
        assert_eq!(
            verify_blob(&m, &blob_hash(b"other")),
            Err(FirmwareError::HashMismatch)
        );
        assert!(verify_blob(&m, &blob_hash(b"firmware")).is_ok());
    }
}
