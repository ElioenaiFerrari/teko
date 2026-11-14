//! Exponential ElGamal on P-256: `Enc(m; r) = (rG, mG + rY)`. Adding ciphertexts
//! adds plaintexts, so a tally can be summed while encrypted and only the TOTAL
//! decrypted.

use alloc::vec::Vec;
use core::ops::Add;

use p256::{ProjectivePoint, Scalar};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ciphertext {
    pub c1: ProjectivePoint,
    pub c2: ProjectivePoint,
}

impl Add for Ciphertext {
    type Output = Ciphertext;
    fn add(self, o: Ciphertext) -> Ciphertext {
        Ciphertext {
            c1: self.c1 + o.c1,
            c2: self.c2 + o.c2,
        }
    }
}

impl Ciphertext {
    pub fn zero() -> Self {
        Self {
            c1: ProjectivePoint::IDENTITY,
            c2: ProjectivePoint::IDENTITY,
        }
    }
}

/// `m` is a small integer (0/1 for a ballot, a count for a total).
pub fn encrypt_bit(pk: &ProjectivePoint, m: u64, r: &Scalar) -> Ciphertext {
    Ciphertext {
        c1: ProjectivePoint::GENERATOR * *r,
        c2: ProjectivePoint::GENERATOR * Scalar::from(m) + *pk * *r,
    }
}

/// One validator's decryption share of `ct`: `d_i * C1`.
pub fn decrypt_share(secret_share: &Scalar, ct: &Ciphertext) -> ProjectivePoint {
    ct.c1 * *secret_share
}

/// Lagrange coefficient at 0 for index `i` among `indices` (1-based, distinct, non-zero).
pub fn lagrange_at_zero(indices: &[u32], i: u32) -> Option<Scalar> {
    let xi = Scalar::from(u64::from(i));
    let mut num = Scalar::ONE;
    let mut den = Scalar::ONE;
    let mut seen_self = false;
    for &j in indices {
        if j == 0 {
            return None;
        }
        if j == i {
            if seen_self {
                return None; // duplicate
            }
            seen_self = true;
            continue;
        }
        let xj = Scalar::from(u64::from(j));
        num *= xj;
        den *= xj - xi;
    }
    if !seen_self {
        return None;
    }
    Option::<Scalar>::from(den.invert()).map(|inv| num * inv)
}

/// Combines `threshold` or more shares into `m*G` (the plaintext point).
/// `shares`: `(validator index starting at 1, d_i * C1)`; indices must be distinct.
pub fn combine_shares(
    ct: &Ciphertext,
    shares: &[(u32, ProjectivePoint)],
) -> Option<ProjectivePoint> {
    let indices: Vec<u32> = shares.iter().map(|s| s.0).collect();
    let mut acc = ProjectivePoint::IDENTITY;
    for (i, d) in shares {
        let l = lagrange_at_zero(&indices, *i)?;
        acc += *d * l;
    }
    Some(ct.c2 - acc)
}

/// Finds `m` in `0..=max` with `m*G == target`. Variable-time, on PUBLIC data
/// (the decrypted total), so no secret is involved.
pub fn solve_small_dlog(target: &ProjectivePoint, max: u64) -> Option<u64> {
    let mut acc = ProjectivePoint::IDENTITY;
    for m in 0..=max {
        if acc == *target {
            return Some(m);
        }
        acc += ProjectivePoint::GENERATOR;
    }
    None
}
