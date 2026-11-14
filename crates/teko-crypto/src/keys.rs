use alloc::vec::Vec;

use p256::ecdsa::{SigningKey, VerifyingKey};
use p256::elliptic_curve::Generate;
use rand_core::CryptoRng;
use sha3::{Digest, Sha3_256};
use zeroize::Zeroizing;

use crate::error::CryptoError;

/// SHA3-256 over a length-prefixed tag and length-prefixed parts.
/// Unambiguous encoding: different (tag, parts) never collide by concatenation.
pub fn tagged_hash(tag: &str, parts: &[&[u8]]) -> [u8; 32] {
    let mut h = Sha3_256::new();
    h.update((tag.len() as u32).to_be_bytes());
    h.update(tag.as_bytes());
    for p in parts {
        h.update((p.len() as u32).to_be_bytes());
        h.update(p);
    }
    h.finalize().into()
}

/// Purpose-bound key derivation from a root secret (keyed tagged hash).
/// NOTE: SHA3 is not length-extendable, so a keyed hash is a sound PRF here;
/// migrating to KMAC256 is tracked in KEY-HIERARCHY (open question 2).
pub fn derive_key(root: &[u8; 32], tag: &str) -> Zeroizing<[u8; 32]> {
    Zeroizing::new(tagged_hash(tag, &[root.as_slice()]))
}

/// Derives a P-256 signing key for `tag` (rejection-sampling the scalar).
pub fn derive_signing_key(root: &[u8; 32], tag: &str) -> Result<SigningKey, CryptoError> {
    for counter in 0u8..=255 {
        let candidate = Zeroizing::new(tagged_hash(tag, &[root.as_slice(), &[counter]]));
        if let Ok(k) = SigningKey::from_slice(candidate.as_slice()) {
            return Ok(k);
        }
    }
    Err(CryptoError::Derivation)
}

pub fn generate_signing_key<R: CryptoRng + ?Sized>(rng: &mut R) -> SigningKey {
    SigningKey::generate_from_rng(rng)
}

/// SEC1 compressed public key (33 bytes).
pub fn public_key_bytes(key: &SigningKey) -> Vec<u8> {
    key.verifying_key().to_sec1_point(true).as_bytes().to_vec()
}

pub fn verifying_key_from_bytes(bytes: &[u8]) -> Result<VerifyingKey, CryptoError> {
    VerifyingKey::from_sec1_bytes(bytes).map_err(|_| CryptoError::InvalidKey)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tags;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn domain_separation(root in any::<[u8; 32]>()) {
            let a = derive_key(&root, tags::IDENTITY);
            let b = derive_key(&root, tags::PAIRWISE);
            let c = derive_key(&root, tags::VOTER);
            prop_assert_ne!(*a, *b);
            prop_assert_ne!(*a, *c);
            prop_assert_ne!(*b, *c);
        }

        #[test]
        fn tag_and_part_boundaries_are_unambiguous(x in "[a-z]{1,8}", y in "[a-z]{1,8}") {
            // ("ab","c") must differ from ("a","bc")
            let h1 = tagged_hash("t", &[x.as_bytes(), y.as_bytes()]);
            let joined = alloc::format!("{x}{y}");
            let h2 = tagged_hash("t", &[joined.as_bytes()]);
            prop_assert_ne!(h1, h2);
        }

        #[test]
        fn derived_signing_key_is_deterministic(root in any::<[u8; 32]>()) {
            let a = derive_signing_key(&root, tags::IDENTITY).unwrap();
            let b = derive_signing_key(&root, tags::IDENTITY).unwrap();
            prop_assert_eq!(public_key_bytes(&a), public_key_bytes(&b));
        }
    }
}
