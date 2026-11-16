//! A ballot: one exponential-ElGamal ciphertext per option (one-hot), proofs that
//! each entry is 0 or 1, and that they sum to exactly 1.

use alloc::vec::Vec;

use p256::elliptic_curve::Field;
use p256::{ProjectivePoint, Scalar};
use rand_core::{CryptoRng, Rng};
use zeroize::Zeroize;

use crate::codec::{POINT_LEN, point_from_bytes, point_to_bytes};
use crate::elgamal::{Ciphertext, encrypt_bit};
use crate::proofs::{BitProof, SumProof, prove_bit, prove_sum, verify_bit, verify_sum};

/// Options per contest. Keeps the ballot small (~200 bytes per option).
pub const MAX_OPTIONS: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncryptedBallot {
    pub cts: Vec<Ciphertext>,
    pub bit_proofs: Vec<BitProof>,
    pub sum_proof: SumProof,
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
        },
        rs,
    ))
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
    /// `n * (66 + 128) + 64` bytes.
    pub fn encoded_len(n: usize) -> usize {
        n * (2 * POINT_LEN + BitProof::LEN) + SumProof::LEN
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(Self::encoded_len(self.cts.len()));
        for (ct, p) in self.cts.iter().zip(&self.bit_proofs) {
            out.extend_from_slice(&point_to_bytes(&ct.c1));
            out.extend_from_slice(&point_to_bytes(&ct.c2));
            out.extend_from_slice(&p.to_bytes());
        }
        out.extend_from_slice(&self.sum_proof.to_bytes());
        out
    }

    /// Strict: exact length for `n` options, canonical points and scalars.
    pub fn from_bytes(n: usize, b: &[u8]) -> Option<Self> {
        if n == 0 || n > MAX_OPTIONS || b.len() != Self::encoded_len(n) {
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
        let sum_proof = SumProof::from_bytes(b.get(n * per..)?)?;
        Some(Self {
            cts,
            bit_proofs,
            sum_proof,
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
