use alloc::vec::Vec;

use p256::ecdsa::signature::{Signer, Verifier};
use p256::ecdsa::{Signature, SigningKey};

use crate::error::CryptoError;
use crate::keys::{tagged_hash, verifying_key_from_bytes};

/// Signs `tagged_hash(tag, [msg])` (deterministic ECDSA, RFC 6979).
/// The tag guarantees a signature made for one purpose is invalid for another.
pub fn sign_tagged(key: &SigningKey, tag: &str, msg: &[u8]) -> Vec<u8> {
    let digest = tagged_hash(tag, &[msg]);
    let sig: Signature = key.sign(&digest);
    sig.to_bytes().to_vec()
}

pub fn verify_tagged(
    public_key: &[u8],
    tag: &str,
    msg: &[u8],
    signature: &[u8],
) -> Result<(), CryptoError> {
    let vk = verifying_key_from_bytes(public_key)?;
    let sig = Signature::from_slice(signature).map_err(|_| CryptoError::InvalidSignature)?;
    let digest = tagged_hash(tag, &[msg]);
    vk.verify(&digest, &sig)
        .map_err(|_| CryptoError::InvalidSignature)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::{generate_signing_key, public_key_bytes};
    use crate::tags;
    use proptest::prelude::*;
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    proptest! {
        #[test]
        fn sign_verify_and_domain_binding(seed in any::<u64>(), msg in proptest::collection::vec(any::<u8>(), 0..64)) {
            let mut rng = StdRng::seed_from_u64(seed);
            let sk = generate_signing_key(&mut rng);
            let pk = public_key_bytes(&sk);
            let sig = sign_tagged(&sk, tags::DOC_SIGN, &msg);
            prop_assert!(verify_tagged(&pk, tags::DOC_SIGN, &msg, &sig).is_ok());
            prop_assert!(verify_tagged(&pk, tags::ATTR_PROOF, &msg, &sig).is_err());
            let mut other = msg.clone();
            other.push(1);
            prop_assert!(verify_tagged(&pk, tags::DOC_SIGN, &other, &sig).is_err());
        }
    }
}
