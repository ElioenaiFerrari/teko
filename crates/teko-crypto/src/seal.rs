//! Sealing to a public key (ADR-0039): the issuer's device seals the private attributes of a
//! credential to the HOLDER's credential key, and the ciphertext goes on the chain with the
//! proposal. Only the holder's device can open it, so after restoring the wallet (which re-derives
//! the credential keys) the holder gets the attributes back.
//!
//! Construction (ECIES-style): an ephemeral P-256 key, ECDH with the recipient, a SHA3-based KDF
//! with domain tags binding both public keys, and ChaCha20-Poly1305 with the context as
//! associated data. The key is fresh for every message, so the derived nonce never repeats under a
//! key. NOT AUDITED.

use alloc::vec::Vec;

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use p256::ecdh::diffie_hellman;
use p256::ecdsa::SigningKey;
use rand_core::CryptoRng;

use crate::error::CryptoError;
use crate::keys::{generate_signing_key, public_key_bytes, tagged_hash, verifying_key_from_bytes};
use crate::tags;

const EPH_LEN: usize = 33;
const TAG_LEN: usize = 16;
/// Largest plaintext (a credential's claims and salt).
pub const MAX_PLAINTEXT: usize = 2_200;

fn derive(shared: &[u8], eph: &[u8], recipient: &[u8]) -> (Key, Nonce) {
    let key = tagged_hash(tags::SEAL_KEY, &[shared, eph, recipient]);
    let n = tagged_hash(tags::SEAL_NONCE, &[shared, eph, recipient]);
    let mut n12 = [0u8; 12];
    n12.copy_from_slice(&n[..12]);
    (Key::from(key), Nonce::from(n12))
}

/// Seals `plaintext` to `recipient` (SEC1 public key). `context` is authenticated, not hidden.
pub fn seal<R: CryptoRng + ?Sized>(
    recipient: &[u8],
    context: &[u8],
    plaintext: &[u8],
    rng: &mut R,
) -> Result<Vec<u8>, CryptoError> {
    if plaintext.len() > MAX_PLAINTEXT {
        return Err(CryptoError::InvalidArgument);
    }
    let vk = verifying_key_from_bytes(recipient)?;
    let eph = generate_signing_key(rng);
    let eph_pk = public_key_bytes(&eph);
    let shared = diffie_hellman(eph.as_nonzero_scalar(), vk.as_affine());
    let (key, nonce) = derive(shared.raw_secret_bytes().as_slice(), &eph_pk, recipient);
    let ct = ChaCha20Poly1305::new(&key)
        .encrypt(
            &nonce,
            Payload {
                msg: plaintext,
                aad: context,
            },
        )
        .map_err(|_| CryptoError::Unseal)?;
    let mut out = eph_pk;
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Opens what `seal` produced, with the recipient's private key and the same `context`.
pub fn open(recipient: &SigningKey, context: &[u8], sealed: &[u8]) -> Result<Vec<u8>, CryptoError> {
    if sealed.len() < EPH_LEN + TAG_LEN || sealed.len() > EPH_LEN + MAX_PLAINTEXT + TAG_LEN {
        return Err(CryptoError::Unseal);
    }
    let (eph_pk, ct) = sealed.split_at(EPH_LEN);
    let vk = verifying_key_from_bytes(eph_pk).map_err(|_| CryptoError::Unseal)?;
    let shared = diffie_hellman(recipient.as_nonzero_scalar(), vk.as_affine());
    let (key, nonce) = derive(
        shared.raw_secret_bytes().as_slice(),
        eph_pk,
        &public_key_bytes(recipient),
    );
    ChaCha20Poly1305::new(&key)
        .decrypt(
            &nonce,
            Payload {
                msg: ct,
                aad: context,
            },
        )
        .map_err(|_| CryptoError::Unseal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    fn key(seed: u64) -> SigningKey {
        generate_signing_key(&mut StdRng::seed_from_u64(seed))
    }

    #[test]
    fn only_the_recipient_opens_it_with_the_same_context() {
        let (r, other) = (key(1), key(2));
        let sealed = seal(
            &public_key_bytes(&r),
            b"cnh-b",
            b"nome=Ana;cat=B",
            &mut StdRng::seed_from_u64(9),
        )
        .unwrap();
        assert_eq!(open(&r, b"cnh-b", &sealed).unwrap(), b"nome=Ana;cat=B");
        assert!(open(&other, b"cnh-b", &sealed).is_err());
        assert!(
            open(&r, b"cnh-a", &sealed).is_err(),
            "the context is authenticated"
        );
        // the ciphertext hides the plaintext
        assert!(!sealed.windows(8).any(|w| w == b"nome=Ana"));
    }

    #[test]
    fn any_change_to_the_sealed_bytes_is_refused() {
        let r = key(1);
        let sealed = seal(
            &public_key_bytes(&r),
            b"c",
            b"segredo",
            &mut StdRng::seed_from_u64(9),
        )
        .unwrap();
        for i in [0, 5, 33, sealed.len() - 1] {
            let mut bad = sealed.clone();
            bad[i] ^= 1;
            assert!(open(&r, b"c", &bad).is_err(), "byte {i}");
        }
        assert!(open(&r, b"c", &sealed[..40]).is_err());
        assert!(open(&r, b"c", &[]).is_err());
    }

    #[test]
    fn each_seal_is_fresh_and_oversized_or_invalid_input_is_refused() {
        let r = key(3);
        let pk = public_key_bytes(&r);
        let a = seal(&pk, b"c", b"x", &mut StdRng::seed_from_u64(1)).unwrap();
        let b = seal(&pk, b"c", b"x", &mut StdRng::seed_from_u64(2)).unwrap();
        assert_ne!(a, b);
        assert!(
            seal(
                &pk,
                b"c",
                &vec![0; MAX_PLAINTEXT + 1],
                &mut StdRng::seed_from_u64(1)
            )
            .is_err()
        );
        assert!(seal(&[1, 2, 3], b"c", b"x", &mut StdRng::seed_from_u64(1)).is_err());
        assert!(
            seal(
                &pk,
                b"c",
                &vec![7; MAX_PLAINTEXT],
                &mut StdRng::seed_from_u64(1)
            )
            .is_ok()
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(24))]
        #[test]
        fn open_inverts_seal(seed in 0u64..1000, msg in proptest::collection::vec(any::<u8>(), 0..300), ctx in proptest::collection::vec(any::<u8>(), 0..40)) {
            let r = key(seed + 1);
            let sealed = seal(&public_key_bytes(&r), &ctx, &msg, &mut StdRng::seed_from_u64(seed)).unwrap();
            prop_assert_eq!(open(&r, &ctx, &sealed).unwrap(), msg);
        }
    }
}
