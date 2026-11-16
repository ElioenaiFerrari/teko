//! Non-interactive proofs (Fiat-Shamir, STRONG: the challenge hashes the whole
//! statement, not only the commitments).

use alloc::vec::Vec;

use p256::elliptic_curve::Field;
use p256::elliptic_curve::ops::Reduce;
use p256::{FieldBytes, ProjectivePoint, Scalar};
use rand_core::{CryptoRng, Rng};
use teko_crypto::{tagged_hash, tags};

use crate::codec::{point_to_bytes, scalar_from_bytes, scalar_to_bytes};
use crate::elgamal::Ciphertext;

const G: ProjectivePoint = ProjectivePoint::GENERATOR;

fn challenge(tag: &str, ctx: &[u8], points: &[&ProjectivePoint]) -> Scalar {
    let enc: Vec<[u8; 33]> = points.iter().map(|p| point_to_bytes(p)).collect();
    let mut parts: Vec<&[u8]> = Vec::with_capacity(enc.len() + 1);
    parts.push(ctx);
    for e in &enc {
        parts.push(e);
    }
    let h = tagged_hash(tag, &parts);
    Scalar::reduce(&FieldBytes::from(h))
}

/// Proof that a ciphertext encrypts 0 OR 1 (disjunctive Chaum-Pedersen).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BitProof {
    pub c0: Scalar,
    pub s0: Scalar,
    pub c1: Scalar,
    pub s1: Scalar,
}

impl BitProof {
    pub const LEN: usize = 128;

    pub fn to_bytes(&self) -> [u8; Self::LEN] {
        let mut out = [0u8; Self::LEN];
        for (chunk, s) in out
            .chunks_mut(32)
            .zip([&self.c0, &self.s0, &self.c1, &self.s1])
        {
            chunk.copy_from_slice(&scalar_to_bytes(s));
        }
        out
    }

    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        let mut it = b.chunks(32).map(scalar_from_bytes);
        let p = Self {
            c0: it.next()??,
            s0: it.next()??,
            c1: it.next()??,
            s1: it.next()??,
        };
        (b.len() == Self::LEN).then_some(p)
    }
}

/// `ctx` must identify the election, contest, option position and voter key.
pub fn prove_bit<R: Rng + CryptoRng>(
    pk: &ProjectivePoint,
    ct: &Ciphertext,
    bit: bool,
    r: &Scalar,
    ctx: &[u8],
    rng: &mut R,
) -> BitProof {
    // Branch j proves (G, Y, C1, C2 - jG) is a DDH tuple. We know branch `real`.
    let real = u64::from(bit);
    let w = Scalar::random(rng);
    let c_sim = Scalar::random(rng);
    let s_sim = Scalar::random(rng);
    let d_sim = ct.c2 - G * Scalar::from(1 - real);
    let t1_sim = G * s_sim - ct.c1 * c_sim;
    let t2_sim = *pk * s_sim - d_sim * c_sim;
    let (t1_real, t2_real) = (G * w, *pk * w);
    let (t10, t20, t11, t21) = if bit {
        (t1_sim, t2_sim, t1_real, t2_real)
    } else {
        (t1_real, t2_real, t1_sim, t2_sim)
    };
    let c = challenge(
        tags::BALLOT_BIT_PROOF,
        ctx,
        &[pk, &ct.c1, &ct.c2, &t10, &t20, &t11, &t21],
    );
    let c_real = c - c_sim;
    let s_real = w + c_real * *r;
    if bit {
        BitProof {
            c0: c_sim,
            s0: s_sim,
            c1: c_real,
            s1: s_real,
        }
    } else {
        BitProof {
            c0: c_real,
            s0: s_real,
            c1: c_sim,
            s1: s_sim,
        }
    }
}

pub fn verify_bit(pk: &ProjectivePoint, ct: &Ciphertext, p: &BitProof, ctx: &[u8]) -> bool {
    let d0 = ct.c2;
    let d1 = ct.c2 - G;
    let t10 = G * p.s0 - ct.c1 * p.c0;
    let t20 = *pk * p.s0 - d0 * p.c0;
    let t11 = G * p.s1 - ct.c1 * p.c1;
    let t21 = *pk * p.s1 - d1 * p.c1;
    let c = challenge(
        tags::BALLOT_BIT_PROOF,
        ctx,
        &[pk, &ct.c1, &ct.c2, &t10, &t20, &t11, &t21],
    );
    c == p.c0 + p.c1
}

/// Proof that `ct` (the SUM of a ballot's ciphertexts) encrypts exactly 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SumProof {
    pub c: Scalar,
    pub s: Scalar,
}

impl SumProof {
    pub const LEN: usize = 64;

    pub fn to_bytes(&self) -> [u8; Self::LEN] {
        let mut out = [0u8; Self::LEN];
        let (a, b) = out.split_at_mut(32);
        a.copy_from_slice(&scalar_to_bytes(&self.c));
        b.copy_from_slice(&scalar_to_bytes(&self.s));
        out
    }

    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        if b.len() != Self::LEN {
            return None;
        }
        let (a, c) = b.split_at(32);
        Some(Self {
            c: scalar_from_bytes(a)?,
            s: scalar_from_bytes(c)?,
        })
    }
}

pub fn prove_sum<R: Rng + CryptoRng>(
    pk: &ProjectivePoint,
    total: &Ciphertext,
    r_total: &Scalar,
    ctx: &[u8],
    rng: &mut R,
) -> SumProof {
    let w = Scalar::random(rng);
    let (t1, t2) = (G * w, *pk * w);
    let c = challenge(
        tags::BALLOT_SUM_PROOF,
        ctx,
        &[pk, &total.c1, &total.c2, &t1, &t2],
    );
    SumProof {
        c,
        s: w + c * *r_total,
    }
}

pub fn verify_sum(pk: &ProjectivePoint, total: &Ciphertext, p: &SumProof, ctx: &[u8]) -> bool {
    let d = total.c2 - G;
    let t1 = G * p.s - total.c1 * p.c;
    let t2 = *pk * p.s - d * p.c;
    challenge(
        tags::BALLOT_SUM_PROOF,
        ctx,
        &[pk, &total.c1, &total.c2, &t1, &t2],
    ) == p.c
}

/// Proof that `ct` (the WEIGHTED total of a multi-seat ballot) encrypts a value `m` in `1..=n`
/// (ADR-0044): a disjunction with one Chaum-Pedersen branch per value, so it says nothing about
/// WHICH value. Branch `m` proves `(G, Y, C1, C2 - m*G)` is a DDH tuple.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RangeProof {
    pub c: Vec<Scalar>,
    pub s: Vec<Scalar>,
}

impl RangeProof {
    pub fn encoded_len(branches: usize) -> usize {
        branches * SumProof::LEN
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(Self::encoded_len(self.c.len()));
        for (c, s) in self.c.iter().zip(&self.s) {
            out.extend_from_slice(&scalar_to_bytes(c));
            out.extend_from_slice(&scalar_to_bytes(s));
        }
        out
    }

    /// Strict: exactly `branches` pairs of canonical scalars.
    pub fn from_bytes(branches: usize, b: &[u8]) -> Option<Self> {
        if branches == 0 || b.len() != Self::encoded_len(branches) {
            return None;
        }
        let mut c = Vec::with_capacity(branches);
        let mut s = Vec::with_capacity(branches);
        for chunk in b.chunks(SumProof::LEN) {
            c.push(scalar_from_bytes(chunk.get(..32)?)?);
            s.push(scalar_from_bytes(chunk.get(32..)?)?);
        }
        Some(Self { c, s })
    }
}

fn range_ctx(ctx: &[u8], n: usize) -> Vec<u8> {
    let mut c = Vec::with_capacity(ctx.len() + 4);
    c.extend_from_slice(ctx);
    c.extend_from_slice(&(n as u32).to_be_bytes());
    c
}

/// `m` in `1..=n` is the REAL value the total encrypts (with randomness `r_total`).
pub fn prove_range<R: Rng + CryptoRng>(
    pk: &ProjectivePoint,
    total: &Ciphertext,
    m: usize,
    n: usize,
    r_total: &Scalar,
    ctx: &[u8],
    rng: &mut R,
) -> Option<RangeProof> {
    if n == 0 || m == 0 || m > n {
        return None;
    }
    let real = m - 1;
    let w = Scalar::random(rng);
    let mut cs = Vec::with_capacity(n);
    let mut ss = Vec::with_capacity(n);
    let mut ts: Vec<ProjectivePoint> = Vec::with_capacity(2 * n);
    for j in 0..n {
        if j == real {
            cs.push(Scalar::ZERO);
            ss.push(Scalar::ZERO);
            ts.push(G * w);
            ts.push(*pk * w);
        } else {
            let (cj, sj) = (Scalar::random(rng), Scalar::random(rng));
            let d = total.c2 - G * Scalar::from((j + 1) as u64);
            ts.push(G * sj - total.c1 * cj);
            ts.push(*pk * sj - d * cj);
            cs.push(cj);
            ss.push(sj);
        }
    }
    let mut pts: Vec<&ProjectivePoint> = alloc::vec![pk, &total.c1, &total.c2];
    pts.extend(ts.iter());
    let c = challenge(tags::BALLOT_RANGE_PROOF, &range_ctx(ctx, n), &pts);
    let sim_sum = cs
        .iter()
        .enumerate()
        .filter(|(j, _)| *j != real)
        .fold(Scalar::ZERO, |a, (_, c)| a + *c);
    let c_real = c - sim_sum;
    cs[real] = c_real;
    ss[real] = w + c_real * *r_total;
    Some(RangeProof { c: cs, s: ss })
}

pub fn verify_range(
    pk: &ProjectivePoint,
    total: &Ciphertext,
    p: &RangeProof,
    n: usize,
    ctx: &[u8],
) -> bool {
    if n == 0 || p.c.len() != n || p.s.len() != n {
        return false;
    }
    let mut ts: Vec<ProjectivePoint> = Vec::with_capacity(2 * n);
    for j in 0..n {
        let d = total.c2 - G * Scalar::from((j + 1) as u64);
        ts.push(G * p.s[j] - total.c1 * p.c[j]);
        ts.push(*pk * p.s[j] - d * p.c[j]);
    }
    let mut pts: Vec<&ProjectivePoint> = alloc::vec![pk, &total.c1, &total.c2];
    pts.extend(ts.iter());
    let c = challenge(tags::BALLOT_RANGE_PROOF, &range_ctx(ctx, n), &pts);
    c == p.c.iter().fold(Scalar::ZERO, |a, x| a + *x)
}

/// Proof that a validator's decryption share `D = d*C1` uses the same secret `d`
/// as its public verification key `V = d*G` (Chaum-Pedersen).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecryptProof {
    pub c: Scalar,
    pub s: Scalar,
}

impl DecryptProof {
    pub const LEN: usize = 64;

    pub fn to_bytes(&self) -> [u8; Self::LEN] {
        SumProof {
            c: self.c,
            s: self.s,
        }
        .to_bytes()
    }

    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        SumProof::from_bytes(b).map(|p| Self { c: p.c, s: p.s })
    }
}

pub fn prove_decryption<R: Rng + CryptoRng>(
    secret_share: &Scalar,
    ct: &Ciphertext,
    ctx: &[u8],
    rng: &mut R,
) -> DecryptProof {
    let v = G * *secret_share;
    let d = ct.c1 * *secret_share;
    let w = Scalar::random(rng);
    let (t1, t2) = (G * w, ct.c1 * w);
    let c = challenge(tags::DECRYPT_PROOF, ctx, &[&v, &ct.c1, &d, &t1, &t2]);
    DecryptProof {
        c,
        s: w + c * *secret_share,
    }
}

pub fn verify_decryption(
    verification_key: &ProjectivePoint,
    ct: &Ciphertext,
    share: &ProjectivePoint,
    p: &DecryptProof,
    ctx: &[u8],
) -> bool {
    let t1 = G * p.s - *verification_key * p.c;
    let t2 = ct.c1 * p.s - *share * p.c;
    challenge(
        tags::DECRYPT_PROOF,
        ctx,
        &[verification_key, &ct.c1, share, &t1, &t2],
    ) == p.c
}
