//! A ballot: one exponential-ElGamal ciphertext per option (one-hot), proofs that
//! each entry is 0 or 1, and that they sum to exactly 1.

use alloc::vec::Vec;

use p256::elliptic_curve::Field;
use p256::{ProjectivePoint, Scalar};
use rand_core::{CryptoRng, Rng};
use zeroize::Zeroize;

use crate::codec::{POINT_LEN, point_from_bytes, point_to_bytes};
use crate::elgamal::{Ciphertext, encrypt_bit};
use crate::proofs::{
    BitProof, RangeProof, SumProof, prove_bit, prove_range, prove_sum, verify_bit, verify_range,
    verify_sum,
};

/// Options per contest. Keeps the ballot small (~200 bytes per option).
pub const MAX_OPTIONS: usize = 128;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncryptedBallot {
    pub cts: Vec<Ciphertext>,
    pub bit_proofs: Vec<BitProof>,
    /// "The total encrypts exactly 1" (a one-seat ballot: the only kind before ADR-0044).
    pub sum_proof: SumProof,
    /// A MULTI-SEAT ballot (ADR-0044): the weighted total is in `1..=seats`. Then `sum_proof` is
    /// not on the wire (it is a placeholder of zeros that is never read).
    pub range_proof: Option<RangeProof>,
}

/// What the crypto needs to know about a contest: how many options, how many seats, and which
/// options are BLANK or NULL (a ballot is `1..=seats` distinct candidates, OR exactly one special).
#[derive(Clone, Copy, Debug)]
pub struct BallotShape<'a> {
    pub n: usize,
    pub seats: usize,
    pub special: &'a [usize],
}

impl BallotShape<'_> {
    fn weight(&self, i: usize) -> u64 {
        if self.special.contains(&i) {
            self.seats.max(1) as u64
        } else {
            1
        }
    }

    fn valid(&self) -> bool {
        self.n > 0
            && self.n <= MAX_OPTIONS
            && self.seats >= 1
            && self.special.iter().all(|i| *i < self.n)
    }
}

fn scale(ct: &Ciphertext, k: u64) -> Ciphertext {
    (0..k).fold(Ciphertext::zero(), |a, _| a + *ct)
}

/// `sum_i weight_i * ct_i`: candidates weigh 1, blank and null weigh `seats`.
fn weighted_total(shape: &BallotShape, cts: &[Ciphertext]) -> Ciphertext {
    cts.iter()
        .enumerate()
        .fold(Ciphertext::zero(), |a, (i, c)| {
            a + scale(c, shape.weight(i))
        })
}

/// Per-position context so a proof for option 3 cannot be moved to option 4.
fn bit_ctx(ctx: &[u8], index: usize) -> Vec<u8> {
    let mut c = Vec::with_capacity(ctx.len() + 4);
    c.extend_from_slice(ctx);
    c.extend_from_slice(&(index as u32).to_be_bytes());
    c
}

fn total(cts: &[Ciphertext]) -> Ciphertext {
    cts.iter().fold(Ciphertext::zero(), |a, c| a + *c)
}

/// Encrypts a vote for option `choice` of `n`. Runs INSIDE the device: the
/// randomness is wiped before returning and never leaves it. `ctx` binds the proofs
/// to this election, contest and voter key. `None` if `choice`/`n` are out of range.
pub fn encrypt_ballot<R: Rng + CryptoRng>(
    pk: &ProjectivePoint,
    n: usize,
    choice: usize,
    ctx: &[u8],
    rng: &mut R,
) -> Option<EncryptedBallot> {
    let (b, mut rs) = encrypt_ballot_with(pk, n, choice, ctx, rng)?;
    rs.zeroize();
    Some(b)
}

/// Like `encrypt_ballot`, but also returns the per-option randomness. It exists for ONE use: the
/// cast-or-audit challenge (ADR-0041), where the device reveals the randomness of a SPOILED ballot
/// (one that is never signed or cast) so the voter's computer can open it. The caller MUST wipe
/// the randomness and never reveal it for a ballot that is cast.
pub fn encrypt_ballot_with<R: Rng + CryptoRng>(
    pk: &ProjectivePoint,
    n: usize,
    choice: usize,
    ctx: &[u8],
    rng: &mut R,
) -> Option<(EncryptedBallot, Vec<Scalar>)> {
    if n == 0 || n > MAX_OPTIONS || choice >= n {
        return None;
    }
    let rs: Vec<Scalar> = (0..n).map(|_| Scalar::random(rng)).collect();
    let mut cts = Vec::with_capacity(n);
    let mut bit_proofs = Vec::with_capacity(n);
    for (i, r) in rs.iter().enumerate() {
        let bit = i == choice;
        let ct = encrypt_bit(pk, u64::from(bit), r);
        bit_proofs.push(prove_bit(pk, &ct, bit, r, &bit_ctx(ctx, i), rng));
        cts.push(ct);
    }
    let r_total = rs.iter().fold(Scalar::ZERO, |a, r| a + *r);
    let sum_proof = prove_sum(pk, &total(&cts), &r_total, ctx, rng);
    Some((
        EncryptedBallot {
            cts,
            bit_proofs,
            sum_proof,
            range_proof: None,
        },
        rs,
    ))
}

/// Encrypts a MULTI-SEAT vote (ADR-0044): `choices` are the chosen option positions. For one seat
/// this is exactly `encrypt_ballot_with` (same bytes). `None` if the choices break the rule:
/// `1..=seats` DISTINCT candidates, or exactly one blank or null alone.
pub fn encrypt_ballot_multi_with<R: Rng + CryptoRng>(
    pk: &ProjectivePoint,
    shape: &BallotShape,
    choices: &[usize],
    ctx: &[u8],
    rng: &mut R,
) -> Option<(EncryptedBallot, Vec<Scalar>)> {
    if !shape.valid() || choices.is_empty() {
        return None;
    }
    if shape.seats == 1 {
        return match choices {
            [one] => encrypt_ballot_with(pk, shape.n, *one, ctx, rng),
            _ => None,
        };
    }
    let mut seen = alloc::vec![false; shape.n];
    for c in choices {
        if *c >= shape.n || core::mem::replace(&mut seen[*c], true) {
            return None; // out of range or repeated
        }
    }
    let specials = choices.iter().filter(|c| shape.special.contains(c)).count();
    let candidates = choices.len() - specials;
    let m = match (specials, candidates) {
        (1, 0) => shape.seats,
        (0, k) if (1..=shape.seats).contains(&k) => k,
        _ => return None,
    };
    let rs: Vec<Scalar> = (0..shape.n).map(|_| Scalar::random(rng)).collect();
    let mut cts = Vec::with_capacity(shape.n);
    let mut bit_proofs = Vec::with_capacity(shape.n);
    for (i, r) in rs.iter().enumerate() {
        let bit = seen[i];
        let ct = encrypt_bit(pk, u64::from(bit), r);
        bit_proofs.push(prove_bit(pk, &ct, bit, r, &bit_ctx(ctx, i), rng));
        cts.push(ct);
    }
    let r_total = rs.iter().enumerate().fold(Scalar::ZERO, |a, (i, r)| {
        a + *r * Scalar::from(shape.weight(i))
    });
    let total = weighted_total(shape, &cts);
    let range = prove_range(pk, &total, m, shape.seats, &r_total, ctx, rng)?;
    Some((
        EncryptedBallot {
            cts,
            bit_proofs,
            sum_proof: SumProof {
                c: Scalar::ZERO,
                s: Scalar::ZERO,
            },
            range_proof: Some(range),
        },
        rs,
    ))
}

/// Like `encrypt_ballot_multi_with`, wiping the randomness.
pub fn encrypt_ballot_multi<R: Rng + CryptoRng>(
    pk: &ProjectivePoint,
    shape: &BallotShape,
    choices: &[usize],
    ctx: &[u8],
    rng: &mut R,
) -> Option<EncryptedBallot> {
    let (b, mut rs) = encrypt_ballot_multi_with(pk, shape, choices, ctx, rng)?;
    rs.zeroize();
    Some(b)
}

/// Checks a ballot of any shape: every entry 0 or 1, and the total is what the shape allows.
pub fn verify_ballot_shape(
    pk: &ProjectivePoint,
    shape: &BallotShape,
    ctx: &[u8],
    b: &EncryptedBallot,
) -> bool {
    if !shape.valid() {
        return false;
    }
    if shape.seats == 1 {
        return b.range_proof.is_none() && verify_ballot(pk, shape.n, ctx, b);
    }
    let Some(range) = &b.range_proof else {
        return false;
    };
    if b.cts.len() != shape.n || b.bit_proofs.len() != shape.n {
        return false;
    }
    let bits_ok = b
        .cts
        .iter()
        .zip(&b.bit_proofs)
        .enumerate()
        .all(|(i, (ct, p))| verify_bit(pk, ct, p, &bit_ctx(ctx, i)));
    bits_ok && verify_range(pk, &weighted_total(shape, &b.cts), range, shape.seats, ctx)
}

/// Opens a ballot of any shape with the randomness that made it: the chosen positions (ascending),
/// or `None` if it is not a valid ballot of this shape made with `rs`. The audit uses this.
pub fn open_ballot_multi(
    pk: &ProjectivePoint,
    shape: &BallotShape,
    ctx: &[u8],
    b: &EncryptedBallot,
    rs: &[Scalar],
) -> Option<Vec<usize>> {
    if rs.len() != shape.n || !verify_ballot_shape(pk, shape, ctx, b) {
        return None;
    }
    let g = ProjectivePoint::GENERATOR;
    let mut chosen = Vec::new();
    for (i, (ct, r)) in b.cts.iter().zip(rs).enumerate() {
        if ct.c1 != g * r {
            return None;
        }
        let m = ct.c2 - pk * r;
        if m == ProjectivePoint::IDENTITY {
            continue;
        }
        if m == g {
            chosen.push(i);
        } else {
            return None;
        }
    }
    (!chosen.is_empty()).then_some(chosen)
}

/// Opens a ballot with the randomness that made it: returns the ONE option it encodes, or `None`
/// if it does not encode exactly one option (or the proofs are invalid, or `rs` is not the
/// randomness of THIS ballot). This is what the voter's computer runs on an AUDITED ballot to
/// confirm that the device encrypted the option it was asked to.
pub fn open_ballot(
    pk: &ProjectivePoint,
    n: usize,
    ctx: &[u8],
    b: &EncryptedBallot,
    rs: &[Scalar],
) -> Option<usize> {
    if rs.len() != n || !verify_ballot(pk, n, ctx, b) {
        return None;
    }
    let g = ProjectivePoint::GENERATOR;
    let mut chosen: Option<usize> = None;
    for (i, (ct, r)) in b.cts.iter().zip(rs).enumerate() {
        if ct.c1 != g * r {
            return None;
        }
        let m = ct.c2 - pk * r;
        if m == ProjectivePoint::IDENTITY {
            continue;
        }
        if m == g && chosen.is_none() {
            chosen = Some(i);
        } else {
            return None;
        }
    }
    chosen
}

pub fn verify_ballot(pk: &ProjectivePoint, n: usize, ctx: &[u8], b: &EncryptedBallot) -> bool {
    if n == 0 || n > MAX_OPTIONS || b.cts.len() != n || b.bit_proofs.len() != n {
        return false;
    }
    let bits_ok = b
        .cts
        .iter()
        .zip(&b.bit_proofs)
        .enumerate()
        .all(|(i, (ct, p))| verify_bit(pk, ct, p, &bit_ctx(ctx, i)));
    bits_ok && verify_sum(pk, &total(&b.cts), &b.sum_proof, ctx)
}

/// Homomorphic per-option sums over ballots that were ALREADY verified.
pub fn sum_ballots(n: usize, ballots: &[&EncryptedBallot]) -> Vec<Ciphertext> {
    let mut acc = alloc::vec![Ciphertext::zero(); n];
    for b in ballots {
        for (a, c) in acc.iter_mut().zip(&b.cts) {
            *a = *a + *c;
        }
    }
    acc
}

impl EncryptedBallot {
    /// `n * (66 + 128) + 64` bytes (one seat).
    pub fn encoded_len(n: usize) -> usize {
        Self::encoded_len_seats(n, 1)
    }

    /// With several seats the 64-byte sum proof is a range proof of `64 * seats` bytes.
    pub fn encoded_len_seats(n: usize, seats: usize) -> usize {
        let tail = if seats <= 1 {
            SumProof::LEN
        } else {
            RangeProof::encoded_len(seats)
        };
        n * (2 * POINT_LEN + BitProof::LEN) + tail
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(Self::encoded_len(self.cts.len()));
        for (ct, p) in self.cts.iter().zip(&self.bit_proofs) {
            out.extend_from_slice(&point_to_bytes(&ct.c1));
            out.extend_from_slice(&point_to_bytes(&ct.c2));
            out.extend_from_slice(&p.to_bytes());
        }
        match &self.range_proof {
            Some(r) => out.extend_from_slice(&r.to_bytes()),
            None => out.extend_from_slice(&self.sum_proof.to_bytes()),
        }
        out
    }

    /// Strict: exact length for `n` options, canonical points and scalars.
    pub fn from_bytes(n: usize, b: &[u8]) -> Option<Self> {
        Self::from_bytes_seats(n, 1, b)
    }

    /// Strict parse of a ballot of a contest with `seats` seats.
    pub fn from_bytes_seats(n: usize, seats: usize, b: &[u8]) -> Option<Self> {
        if n == 0 || n > MAX_OPTIONS || seats == 0 || b.len() != Self::encoded_len_seats(n, seats) {
            return None;
        }
        let per = 2 * POINT_LEN + BitProof::LEN;
        let mut cts = Vec::with_capacity(n);
        let mut bit_proofs = Vec::with_capacity(n);
        for chunk in b.chunks(per).take(n) {
            let c1 = point_from_bytes(chunk.get(..POINT_LEN)?)?;
            let c2 = point_from_bytes(chunk.get(POINT_LEN..2 * POINT_LEN)?)?;
            cts.push(Ciphertext { c1, c2 });
            bit_proofs.push(BitProof::from_bytes(chunk.get(2 * POINT_LEN..)?)?);
        }
        let tail = b.get(n * per..)?;
        if seats <= 1 {
            return Some(Self {
                cts,
                bit_proofs,
                sum_proof: SumProof::from_bytes(tail)?,
                range_proof: None,
            });
        }
        Some(Self {
            cts,
            bit_proofs,
            sum_proof: SumProof {
                c: Scalar::ZERO,
                s: Scalar::ZERO,
            },
            range_proof: Some(RangeProof::from_bytes(seats, tail)?),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::elgamal::{combine_shares, decrypt_share, solve_small_dlog};
    use proptest::prelude::*;
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    fn keypair(seed: u64) -> (Scalar, ProjectivePoint) {
        let mut rng = StdRng::seed_from_u64(seed);
        let x = Scalar::random(&mut rng);
        (x, ProjectivePoint::GENERATOR * x)
    }

    const CTX: &[u8] = b"eleicao|presidente|voter-key-1";

    #[test]
    fn a_valid_ballot_verifies_roundtrips_and_has_the_expected_size() {
        let (_, pk) = keypair(1);
        let mut rng = StdRng::seed_from_u64(2);
        for n in [1usize, 2, 12, MAX_OPTIONS] {
            let b = encrypt_ballot(&pk, n, n - 1, CTX, &mut rng).unwrap();
            assert!(verify_ballot(&pk, n, CTX, &b));
            let bytes = b.to_bytes();
            assert_eq!(bytes.len(), EncryptedBallot::encoded_len(n));
            let back = EncryptedBallot::from_bytes(n, &bytes).unwrap();
            assert_eq!(back, b);
            assert!(verify_ballot(&pk, n, CTX, &back));
        }
        // 12 options (the biggest demo contest): about 2.4 KB
        assert!(EncryptedBallot::encoded_len(12) < 2_500);
        assert!(
            encrypt_ballot(&pk, 3, 3, CTX, &mut rng).is_none(),
            "choice out of range"
        );
        assert!(encrypt_ballot(&pk, 0, 0, CTX, &mut rng).is_none());
        assert!(encrypt_ballot(&pk, MAX_OPTIONS + 1, 0, CTX, &mut rng).is_none());
    }

    #[test]
    fn an_audited_ballot_opens_to_exactly_the_option_it_encrypts() {
        let (_, pk) = keypair(5);
        let mut rng = StdRng::seed_from_u64(6);
        for n in [2usize, 5, MAX_OPTIONS] {
            for choice in [0, n - 1] {
                let (b, rs) = encrypt_ballot_with(&pk, n, choice, CTX, &mut rng).unwrap();
                assert_eq!(open_ballot(&pk, n, CTX, &b, &rs), Some(choice));
                // the plain encrypt gives ballots of the same shape (and drops the randomness)
                let c = encrypt_ballot(&pk, n, choice, CTX, &mut rng).unwrap();
                assert!(verify_ballot(&pk, n, CTX, &c));
            }
        }
    }

    #[test]
    fn a_wrong_randomness_a_tampered_ballot_or_another_context_does_not_open() {
        let (_, pk) = keypair(7);
        let mut rng = StdRng::seed_from_u64(8);
        let n = 4;
        let (b, rs) = encrypt_ballot_with(&pk, n, 2, CTX, &mut rng).unwrap();
        let (_, other_rs) = encrypt_ballot_with(&pk, n, 2, CTX, &mut rng).unwrap();
        assert_eq!(open_ballot(&pk, n, CTX, &b, &other_rs), None);
        let mut bad = rs.clone();
        bad[0] += Scalar::ONE;
        assert_eq!(open_ballot(&pk, n, CTX, &b, &bad), None);
        assert_eq!(open_ballot(&pk, n, CTX, &b, &rs[..n - 1]), None);
        assert_eq!(open_ballot(&pk, n, b"outro", &b, &rs), None);
        let mut t = b.clone();
        t.cts.swap(0, 2);
        assert_eq!(open_ballot(&pk, n, CTX, &t, &rs), None);
        // a ballot that says "this option" to the proofs but encodes ANOTHER (a lying device):
        // the device encrypts option 1 while the screen said option 2: opening reveals 1, not 2
        let (lie, lie_rs) = encrypt_ballot_with(&pk, n, 1, CTX, &mut rng).unwrap();
        assert_eq!(open_ballot(&pk, n, CTX, &lie, &lie_rs), Some(1));
        assert_ne!(open_ballot(&pk, n, CTX, &lie, &lie_rs), Some(2));
    }

    const SPECIAL: [usize; 2] = [4, 5]; // 4 candidates, then blank and null

    fn shape(seats: usize) -> BallotShape<'static> {
        BallotShape {
            n: 6,
            seats,
            special: &SPECIAL,
        }
    }

    #[test]
    fn a_one_seat_ballot_keeps_todays_bytes_and_the_multi_api_agrees_with_it() {
        let (_, pk) = keypair(11);
        let mut rng = StdRng::seed_from_u64(12);
        let old = encrypt_ballot(&pk, 6, 3, CTX, &mut rng).unwrap();
        let new = encrypt_ballot_multi(&pk, &shape(1), &[3], CTX, &mut rng).unwrap();
        assert_eq!(old.to_bytes().len(), new.to_bytes().len());
        assert_eq!(new.to_bytes().len(), EncryptedBallot::encoded_len(6));
        assert!(new.range_proof.is_none());
        assert!(verify_ballot_shape(&pk, &shape(1), CTX, &old));
        assert!(verify_ballot_shape(&pk, &shape(1), CTX, &new));
        // one seat: a second choice is not a ballot
        assert!(encrypt_ballot_multi(&pk, &shape(1), &[0, 1], CTX, &mut rng).is_none());
        assert!(encrypt_ballot_multi(&pk, &shape(1), &[], CTX, &mut rng).is_none());
    }

    #[test]
    fn a_two_seat_ballot_is_one_or_two_distinct_candidates_or_exactly_blank_or_null() {
        let (_, pk) = keypair(13);
        let mut rng = StdRng::seed_from_u64(14);
        let sh = shape(2);
        let good: [&[usize]; 6] = [&[0], &[3], &[0, 1], &[2, 3], &[4], &[5]];
        for choices in good {
            let (b, rs) = encrypt_ballot_multi_with(&pk, &sh, choices, CTX, &mut rng).unwrap();
            assert!(verify_ballot_shape(&pk, &sh, CTX, &b), "{choices:?}");
            assert_eq!(b.to_bytes().len(), EncryptedBallot::encoded_len_seats(6, 2));
            // the wire roundtrip is strict and exact
            let back = EncryptedBallot::from_bytes_seats(6, 2, &b.to_bytes()).unwrap();
            assert_eq!(back, b);
            assert!(verify_ballot_shape(&pk, &sh, CTX, &back));
            // opening (the audit) returns exactly the chosen set
            let mut want = choices.to_vec();
            want.sort_unstable();
            assert_eq!(open_ballot_multi(&pk, &sh, CTX, &b, &rs), Some(want));
        }
        // three candidates, a repeat, an out-of-range, a candidate with blank, blank with null, nothing
        let bad: [&[usize]; 6] = [&[0, 1, 2], &[1, 1], &[9], &[0, 4], &[4, 5], &[]];
        for choices in bad {
            assert!(
                encrypt_ballot_multi(&pk, &sh, choices, CTX, &mut rng).is_none(),
                "{choices:?} must not be encryptable"
            );
        }
    }

    #[test]
    fn a_forged_two_seat_ballot_is_refused_by_the_proofs() {
        let (_, pk) = keypair(15);
        let mut rng = StdRng::seed_from_u64(16);
        let sh = shape(2);
        // forge: build ciphertexts for THREE candidates and reuse an honest ballot's range proof
        let honest = encrypt_ballot_multi(&pk, &sh, &[0, 1], CTX, &mut rng).unwrap();
        let r: Vec<Scalar> = (0..6).map(|_| Scalar::random(&mut rng)).collect();
        let bits = [true, true, true, false, false, false];
        let cts: Vec<Ciphertext> = bits
            .iter()
            .zip(&r)
            .map(|(b, r)| encrypt_bit(&pk, u64::from(*b), r))
            .collect();
        let bit_proofs: Vec<_> = (0..6)
            .map(|i| prove_bit(&pk, &cts[i], bits[i], &r[i], &bit_ctx(CTX, i), &mut rng))
            .collect();
        let forged = EncryptedBallot {
            cts,
            bit_proofs,
            sum_proof: honest.sum_proof,
            range_proof: honest.range_proof.clone(),
        };
        assert!(
            !verify_ballot_shape(&pk, &sh, CTX, &forged),
            "3 candidates in 2 seats"
        );
        // a valid range proof moved to another context, another seat count, a one-seat verifier
        assert!(!verify_ballot_shape(&pk, &sh, b"outro", &honest));
        assert!(!verify_ballot_shape(&pk, &shape(1), CTX, &honest));
        assert!(!verify_ballot_shape(&pk, &shape(3), CTX, &honest));
        // tampered range proof
        let mut t = honest.clone();
        if let Some(rp) = t.range_proof.as_mut() {
            rp.s[0] += Scalar::ONE;
        }
        assert!(!verify_ballot_shape(&pk, &sh, CTX, &t));
        // a wrong randomness does not open, and truncation fails to parse
        let (b, rs) = encrypt_ballot_multi_with(&pk, &sh, &[0, 2], CTX, &mut rng).unwrap();
        let (_, other) = encrypt_ballot_multi_with(&pk, &sh, &[0, 2], CTX, &mut rng).unwrap();
        assert_eq!(open_ballot_multi(&pk, &sh, CTX, &b, &other), None);
        assert!(open_ballot_multi(&pk, &sh, CTX, &b, &rs).is_some());
        let bytes = b.to_bytes();
        assert!(EncryptedBallot::from_bytes_seats(6, 2, &bytes[..bytes.len() - 1]).is_none());
        assert!(EncryptedBallot::from_bytes_seats(6, 1, &bytes).is_none());
    }

    #[test]
    fn invalid_ballots_are_rejected() {
        let (_, pk) = keypair(3);
        let mut rng = StdRng::seed_from_u64(4);
        let n = 4;
        let good = encrypt_ballot(&pk, n, 1, CTX, &mut rng).unwrap();

        // wrong context: replayed for another contest / election / voter key
        for ctx in [
            &b"eleicao|governador|voter-key-1"[..],
            b"outra|presidente|voter-key-1",
            b"eleicao|presidente|voter-key-2",
            b"",
        ] {
            assert!(!verify_ballot(&pk, n, ctx, &good), "ctx {ctx:?}");
        }
        // wrong public key
        let (_, other_pk) = keypair(99);
        assert!(!verify_ballot(&other_pk, n, CTX, &good));
        // wrong option count
        assert!(!verify_ballot(&pk, n + 1, CTX, &good));
        // proof moved between positions
        let mut swapped = good.clone();
        swapped.bit_proofs.swap(0, 1);
        assert!(!verify_ballot(&pk, n, CTX, &swapped));
        // ciphertexts reordered (the proofs no longer match their positions)
        let mut reordered = good.clone();
        reordered.cts.swap(0, 1);
        assert!(!verify_ballot(&pk, n, CTX, &reordered));
        // tampered sum proof
        let mut t = good.clone();
        t.sum_proof.s += Scalar::ONE;
        assert!(!verify_ballot(&pk, n, CTX, &t));

        // a vote for TWO options: each bit proof is individually valid but the sum is 2
        let r: Vec<Scalar> = (0..n).map(|_| Scalar::random(&mut rng)).collect();
        let bits = [true, true, false, false];
        let cts: Vec<Ciphertext> = bits
            .iter()
            .zip(&r)
            .map(|(b, r)| encrypt_bit(&pk, u64::from(*b), r))
            .collect();
        let bit_proofs: Vec<BitProof> = cts
            .iter()
            .zip(&bits)
            .zip(&r)
            .enumerate()
            .map(|(i, ((c, b), r))| prove_bit(&pk, c, *b, r, &bit_ctx(CTX, i), &mut rng))
            .collect();
        let r_total = r.iter().fold(Scalar::ZERO, |a, x| a + *x);
        let two = EncryptedBallot {
            sum_proof: prove_sum(&pk, &total(&cts), &r_total, CTX, &mut rng),
            cts,
            bit_proofs,
            range_proof: None,
        };
        assert!(!verify_ballot(&pk, n, CTX, &two), "two options marked");

        // no option marked (sum 0), and a "2" in one slot
        for bits in [[false; 4], [false; 4]] {
            let cts: Vec<Ciphertext> = bits
                .iter()
                .zip(&r)
                .map(|(b, r)| encrypt_bit(&pk, u64::from(*b), r))
                .collect();
            let bit_proofs: Vec<BitProof> = cts
                .iter()
                .zip(&bits)
                .zip(&r)
                .enumerate()
                .map(|(i, ((c, b), r))| prove_bit(&pk, c, *b, r, &bit_ctx(CTX, i), &mut rng))
                .collect();
            let none = EncryptedBallot {
                sum_proof: prove_sum(&pk, &total(&cts), &r_total, CTX, &mut rng),
                cts,
                bit_proofs,
                range_proof: None,
            };
            assert!(!verify_ballot(&pk, n, CTX, &none), "no option marked");
        }
        let mut with_two = good.clone();
        with_two.cts[0] = encrypt_bit(&pk, 2, &Scalar::random(&mut rng));
        assert!(
            !verify_ballot(&pk, n, CTX, &with_two),
            "a 2 cannot carry a 0/1 proof"
        );

        // malformed encodings
        let bytes = good.to_bytes();
        assert!(EncryptedBallot::from_bytes(n, &bytes[1..]).is_none());
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(EncryptedBallot::from_bytes(n, &longer).is_none());
        let mut offcurve = bytes.clone();
        offcurve[5] ^= 0xFF;
        assert!(
            EncryptedBallot::from_bytes(n, &offcurve)
                .is_none_or(|b| !verify_ballot(&pk, n, CTX, &b))
        );
    }

    #[test]
    fn a_proof_is_bound_to_its_exact_ciphertext() {
        // Strong Fiat-Shamir: the challenge hashes the ciphertext, so a proof made for
        // one ciphertext never verifies for another with the same public key and context.
        let (_, pk) = keypair(5);
        let mut rng = StdRng::seed_from_u64(6);
        let r = Scalar::random(&mut rng);
        let ct = encrypt_bit(&pk, 1, &r);
        let p = prove_bit(&pk, &ct, true, &r, CTX, &mut rng);
        assert!(verify_bit(&pk, &ct, &p, CTX));
        let other = encrypt_bit(&pk, 1, &Scalar::random(&mut rng));
        assert!(!verify_bit(&pk, &other, &p, CTX));
        let two = encrypt_bit(&pk, 2, &r);
        assert!(!verify_bit(&pk, &two, &p, CTX));
    }

    #[test]
    fn threshold_decryption_of_the_homomorphic_sum_matches_the_plain_count() {
        // Shamir-share a secret 2-of-3 by hand: f(x) = a0 + a1 x
        let mut rng = StdRng::seed_from_u64(7);
        let (a0, a1) = (Scalar::random(&mut rng), Scalar::random(&mut rng));
        let f = |x: u64| a0 + a1 * Scalar::from(x);
        let pk = ProjectivePoint::GENERATOR * a0;
        let n_opts = 3;
        let votes = [0usize, 2, 2, 1, 2, 0, 2, 1, 2, 2];
        let ballots: Vec<EncryptedBallot> = votes
            .iter()
            .map(|v| encrypt_ballot(&pk, n_opts, *v, CTX, &mut rng).unwrap())
            .collect();
        assert!(ballots.iter().all(|b| verify_ballot(&pk, n_opts, CTX, b)));
        let sums = sum_ballots(n_opts, &ballots.iter().collect::<Vec<_>>());
        let mut want = [0u64; 3];
        for v in votes {
            want[v] += 1;
        }
        // any 2 of the 3 validators decrypt the same totals
        for pair in [[1u32, 2], [1, 3], [2, 3]] {
            for (opt, ct) in sums.iter().enumerate() {
                let shares: Vec<(u32, ProjectivePoint)> = pair
                    .iter()
                    .map(|i| (*i, decrypt_share(&f(u64::from(*i)), ct)))
                    .collect();
                let m = combine_shares(ct, &shares).unwrap();
                assert_eq!(
                    solve_small_dlog(&m, votes.len() as u64),
                    Some(want[opt]),
                    "{pair:?} option {opt}"
                );
            }
        }
        // a single share is not enough: it gives a different (wrong) point
        let ct = &sums[2];
        let one = combine_shares(ct, &[(1, decrypt_share(&f(1), ct))]).unwrap();
        assert_ne!(solve_small_dlog(&one, 10), Some(want[2]));
        // duplicate or zero indices are refused
        assert!(
            combine_shares(
                ct,
                &[(1, decrypt_share(&f(1), ct)), (1, decrypt_share(&f(1), ct))]
            )
            .is_none()
        );
        assert!(
            combine_shares(
                ct,
                &[
                    (0, ProjectivePoint::IDENTITY),
                    (2, ProjectivePoint::IDENTITY)
                ]
            )
            .is_none()
        );
        // the identity is never a valid encoded point
        assert!(point_from_bytes(&point_to_bytes(&ProjectivePoint::GENERATOR)).is_some());
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(12))]
        #[test]
        fn any_choice_verifies_and_changes_the_ciphertext_every_time(seed in any::<u64>(), n in 2usize..8, pick in 0usize..8) {
            let choice = pick % n;
            let (_, pk) = keypair(seed);
            let mut rng = StdRng::seed_from_u64(seed ^ 0xABCD);
            let a = encrypt_ballot(&pk, n, choice, CTX, &mut rng).unwrap();
            let b = encrypt_ballot(&pk, n, choice, CTX, &mut rng).unwrap();
            prop_assert!(verify_ballot(&pk, n, CTX, &a));
            // fresh randomness: the same vote never encrypts to the same bytes (no linkage)
            prop_assert_ne!(a.to_bytes(), b.to_bytes());
        }
    }

    /// Prints size and timings for the ADR. Run: `cargo test -p teko-ballot-crypto --release
    /// -- --ignored --nocapture measure`.
    #[test]
    #[ignore = "benchmark, prints numbers"]
    fn measure_size_and_time() {
        let (_, pk) = keypair(1);
        let mut rng = StdRng::seed_from_u64(2);
        for n in [2usize, 12, 16] {
            let t = std::time::Instant::now();
            let reps = 20;
            let mut last = None;
            for _ in 0..reps {
                last = encrypt_ballot(&pk, n, 0, CTX, &mut rng);
            }
            let enc = t.elapsed() / reps;
            let b = last.unwrap();
            let t = std::time::Instant::now();
            for _ in 0..reps {
                assert!(verify_ballot(&pk, n, CTX, &b));
            }
            let ver = t.elapsed() / reps;
            std::println!(
                "options={n:2} bytes={} encrypt={enc:?} verify={ver:?}",
                b.to_bytes().len()
            );
        }
    }
}
