//! Voter secret primitives. **STUB**: SHA3 stands in for Poseidon, and there is
//! no ZK proof. Replaced in the voting phase (ADR-0006, ADR-0008).

use crate::keys::tagged_hash;
use crate::tags;

/// `C = H(s)` (stand-in for `Poseidon(s)`).
pub fn commitment(secret: &[u8; 32]) -> [u8; 32] {
    tagged_hash(tags::VOTER_COMMIT, &[secret.as_slice()])
}

/// Per-election nullifier (stand-in for PLUME / ERC-7524).
pub fn nullifier(secret: &[u8; 32], election_id: &str) -> [u8; 32] {
    tagged_hash(
        tags::VOTER_NULLIFIER,
        &[secret.as_slice(), election_id.as_bytes()],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn nullifiers_are_per_election_and_unlinkable_to_commitment(s in any::<[u8;32]>(), e1 in "[a-z]{1,8}", e2 in "[a-z]{1,8}") {
            prop_assume!(e1 != e2);
            prop_assert_ne!(nullifier(&s, &e1), nullifier(&s, &e2));
            prop_assert_ne!(nullifier(&s, &e1), commitment(&s));
        }
    }
}
