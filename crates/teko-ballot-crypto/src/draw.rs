//! The verifiable DRAW that breaks a tie (ADR-0043): a THRESHOLD VRF on the election key of the key
//! ceremony. Everyone derives the same point `H` from the contest; each validator publishes
//! `d_i * H` with a Chaum-Pedersen proof against its verification key; any `t` valid shares combine
//! (Lagrange) into `P = d * H`, which is UNIQUE whichever `t` shares are used. Nobody knows `d`, so
//! nobody can predict or steer `P`, and nobody can pick a favourable subset of shares.

use alloc::vec::Vec;

use p256::{ProjectivePoint, Scalar};
use rand_core::{CryptoRng, Rng};
use teko_crypto::{tagged_hash, tags};

use crate::elgamal::{Ciphertext, lagrange_at_zero};
use crate::proofs::{DecryptProof, prove_decryption, verify_decryption};

/// Hash-to-curve by try-and-increment (RFC 9381 TAI style): the first counter whose hash is the
/// x coordinate of a curve point. Deterministic and public; the discrete log of the result with
/// respect to the generator is unknown to everybody.
pub fn hash_to_point(input: &[u8]) -> Option<ProjectivePoint> {
    for counter in 0u8..=255 {
        let h = tagged_hash(tags::DRAW_POINT, &[input, &[counter]]);
        let mut enc = [0u8; 33];
        enc[0] = 0x02;
        enc[1..].copy_from_slice(&h);
        if let Some(p) = crate::codec::point_from_bytes(&enc) {
            return Some(p);
        }
    }
    None
}

fn as_ct(h: &ProjectivePoint) -> Ciphertext {
    Ciphertext {
        c1: *h,
        c2: ProjectivePoint::IDENTITY,
    }
}

/// One validator's share `d_i * H` of the draw, with its proof (bound to `ctx`).
pub fn draw_share<R: Rng + CryptoRng>(
    secret_share: &Scalar,
    h: &ProjectivePoint,
    ctx: &[u8],
    rng: &mut R,
) -> (ProjectivePoint, DecryptProof) {
    let ct = as_ct(h);
    (
        *h * *secret_share,
        prove_decryption(secret_share, &ct, ctx, rng),
    )
}

pub fn verify_draw_share(
    verification_key: &ProjectivePoint,
    h: &ProjectivePoint,
    share: &ProjectivePoint,
    proof: &DecryptProof,
    ctx: &[u8],
) -> bool {
    verify_decryption(verification_key, &as_ct(h), share, proof, ctx)
}

/// Combines `t` or more VERIFIED shares `(validator index from 1, d_i * H)` into `P = d * H`.
/// Indices must be distinct.
pub fn combine_draw(shares: &[(u32, ProjectivePoint)]) -> Option<ProjectivePoint> {
    let indices: Vec<u32> = shares.iter().map(|s| s.0).collect();
    let mut acc = ProjectivePoint::IDENTITY;
    for (i, d) in shares {
        acc += *d * lagrange_at_zero(&indices, *i)?;
    }
    Some(acc)
}

/// The 32-byte seed of the draw: the combined point, bound to the draw input.
pub fn draw_seed(p: &ProjectivePoint, input: &[u8]) -> [u8; 32] {
    tagged_hash(tags::DRAW_SEED, &[&crate::codec::point_to_bytes(p), input])
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::elliptic_curve::Field;
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    /// A (t, n) Shamir sharing of a random `d`: shares `d_i = f(i)`, verification keys `d_i * G`.
    fn sharing(t: usize, n: u32, seed: u64) -> (Scalar, Vec<Scalar>) {
        let mut rng = StdRng::seed_from_u64(seed);
        let coeffs: Vec<Scalar> = (0..t).map(|_| Scalar::random(&mut rng)).collect();
        let eval = |x: u32| {
            let x = Scalar::from(u64::from(x));
            coeffs
                .iter()
                .rev()
                .fold(Scalar::ZERO, |acc, c| acc * x + *c)
        };
        (coeffs[0], (1..=n).map(eval).collect())
    }

    #[test]
    fn the_point_is_valid_deterministic_and_domain_separated() {
        let a = hash_to_point(b"contest-1").unwrap();
        assert_eq!(a, hash_to_point(b"contest-1").unwrap());
        assert_ne!(a, hash_to_point(b"contest-2").unwrap());
        assert_ne!(a, ProjectivePoint::IDENTITY);
    }

    #[test]
    fn any_t_shares_combine_to_the_same_point_and_the_same_seed() {
        let (t, n) = (3usize, 4u32);
        let (d, shares) = sharing(t, n, 1);
        let h = hash_to_point(b"draw").unwrap();
        let mut rng = StdRng::seed_from_u64(2);
        let parts: Vec<(u32, ProjectivePoint)> = shares
            .iter()
            .enumerate()
            .map(|(i, s)| ((i + 1) as u32, draw_share(s, &h, b"ctx", &mut rng).0))
            .collect();
        let expected = h * d;
        for subset in [[0usize, 1, 2], [0, 1, 3], [0, 2, 3], [1, 2, 3]] {
            let picked: Vec<_> = subset.iter().map(|i| parts[*i]).collect();
            let p = combine_draw(&picked).unwrap();
            assert_eq!(p, expected, "subset {subset:?}");
            assert_eq!(draw_seed(&p, b"in"), draw_seed(&expected, b"in"));
        }
        // all four also agree; fewer than t give something else (no information, no agreement)
        assert_eq!(combine_draw(&parts).unwrap(), expected);
        assert_ne!(combine_draw(&parts[..2]).unwrap(), expected);
        assert_ne!(draw_seed(&expected, b"in"), draw_seed(&expected, b"other"));
    }

    #[test]
    fn a_wrong_share_a_wrong_key_or_another_context_is_refused() {
        let (_, shares) = sharing(3, 4, 5);
        let h = hash_to_point(b"draw").unwrap();
        let mut rng = StdRng::seed_from_u64(6);
        let vk = ProjectivePoint::GENERATOR * shares[0];
        let (pt, proof) = draw_share(&shares[0], &h, b"ctx", &mut rng);
        assert!(verify_draw_share(&vk, &h, &pt, &proof, b"ctx"));
        assert!(!verify_draw_share(&vk, &h, &pt, &proof, b"other"));
        let other_vk = ProjectivePoint::GENERATOR * shares[1];
        assert!(!verify_draw_share(&other_vk, &h, &pt, &proof, b"ctx"));
        let h2 = hash_to_point(b"draw-2").unwrap();
        assert!(!verify_draw_share(&vk, &h2, &pt, &proof, b"ctx"));
        let forged = pt + ProjectivePoint::GENERATOR;
        assert!(!verify_draw_share(&vk, &h, &forged, &proof, b"ctx"));
        // duplicated or zero indices do not combine
        assert!(combine_draw(&[(1, pt), (1, pt)]).is_none());
        assert!(combine_draw(&[(0, pt)]).is_none());
    }
}
