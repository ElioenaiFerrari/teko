//! Voter keys (Phase 2, ADR-0006): the voter credential is a SIGNING KEY, one per
//! contest, derived from a voter root that lives only inside the device.
//!
//! Why a key and not `Poseidon(s)`: the host that builds the ZK proof must never
//! learn the secret, so the device signs and the host proves over the signature.
//! Why one key PER CONTEST (not per election): the same key in the President's
//! and the Mayor's contests would let anyone link the two votes and shrink the
//! President's anonymity set to one municipality.
//!
//! Registration, nullifiers and proofs are NOT implemented yet (ADR-0008/0013).

use p256::ecdsa::SigningKey;
use zeroize::Zeroizing;

use crate::error::CryptoError;
use crate::keys::tagged_hash;
use crate::tags;

/// Independent key for `(election_id, contest_id)`. Deterministic, so the device
/// never has to store it; unrelated across contests, elections and roots.
pub fn derive_voter_key(
    voter_root: &[u8; 32],
    election_id: &str,
    contest_id: &str,
) -> Result<SigningKey, CryptoError> {
    for counter in 0u8..=255 {
        let candidate = Zeroizing::new(tagged_hash(
            tags::VOTER_KEY,
            &[
                voter_root.as_slice(),
                election_id.as_bytes(),
                contest_id.as_bytes(),
                &[counter],
            ],
        ));
        if let Ok(k) = SigningKey::from_slice(candidate.as_slice()) {
            return Ok(k);
        }
    }
    Err(CryptoError::Derivation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::{derive_signing_key, public_key_bytes};
    use proptest::prelude::*;

    fn pk(root: &[u8; 32], e: &str, c: &str) -> Vec<u8> {
        public_key_bytes(&derive_voter_key(root, e, c).unwrap())
    }

    proptest! {
        #[test]
        fn deterministic_and_independent_across_contests_elections_and_roots(
            root in any::<[u8; 32]>(), other in any::<[u8; 32]>(),
            e1 in "[a-z0-9-]{1,12}", e2 in "[a-z0-9-]{1,12}", c1 in "[a-z0-9-]{1,12}", c2 in "[a-z0-9-]{1,12}",
        ) {
            prop_assert_eq!(pk(&root, &e1, &c1), pk(&root, &e1, &c1));
            if c1 != c2 { prop_assert_ne!(pk(&root, &e1, &c1), pk(&root, &e1, &c2), "President vs Mayor must not link"); }
            if e1 != e2 { prop_assert_ne!(pk(&root, &e1, &c1), pk(&root, &e2, &c1)); }
            if root != other { prop_assert_ne!(pk(&root, &e1, &c1), pk(&other, &e1, &c1)); }
            // boundary ambiguity: ("ab","c") vs ("a","bc")
            prop_assert_ne!(pk(&root, "ab", "c"), pk(&root, "a", "bc"));
        }

        #[test]
        fn voter_keys_are_domain_separated_from_the_identity_key(root in any::<[u8; 32]>()) {
            // Even if the same 32 bytes were (wrongly) used as the identity root.
            let id = public_key_bytes(&derive_signing_key(&root, tags::IDENTITY).unwrap());
            prop_assert_ne!(id, pk(&root, "e", "c"));
        }
    }
}
