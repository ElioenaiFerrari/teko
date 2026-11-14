//! Distributed key generation: Feldman VSS with a Schnorr proof of possession per
//! participant (the FROST-style DKG). No single party ever holds the election
//! secret; any `threshold` of `n` shares decrypt.
//!
//! Everyone must see the SAME round-1 packages (Feldman needs a consistent view of the
//! commitments): the validators publish them on the chain, which is a consistent
//! broadcast. Round-2 shares are secret: this crate only builds and checks them;
//! the transport must be authenticated and encrypted in production (the local test
//! network uses loopback).
//!
//! Known, accepted limit: like Pedersen's DKG, an active adversary may BIAS the
//! distribution of the public key (it cannot learn or choose the secret). Helios
//! and Belenios accept this for ElGamal. The proof of possession stops rogue-key
//! attacks.

use alloc::vec::Vec;

use p256::elliptic_curve::Field;
use p256::elliptic_curve::ops::Reduce;
use p256::{FieldBytes, ProjectivePoint, Scalar};
use rand_core::{CryptoRng, Rng};
use teko_crypto::{tagged_hash, tags};
use zeroize::Zeroize;

use crate::codec::point_to_bytes;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DkgError {
    BadParameters,
    /// Round-1 package of this participant is malformed (commitments).
    BadCommitments(u32),
    BadProofOfPossession(u32),
    /// The secret share from this participant does not match its commitments.
    BadShare(u32),
    /// A package or share is missing.
    Missing(u32),
}

/// What a participant publishes in round 1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Round1 {
    /// 1-based participant index.
    pub index: u32,
    /// `a_k * G` for the polynomial's `threshold` coefficients; `[0]` is its public key part.
    pub commitments: Vec<ProjectivePoint>,
    /// Schnorr proof of possession of `a_0`: `(R, s)`.
    pub pop_r: ProjectivePoint,
    pub pop_s: Scalar,
}

/// A participant's secret state. Wiped on drop.
pub struct Participant {
    index: u32,
    n: u32,
    coeffs: Vec<Scalar>,
}

impl Drop for Participant {
    fn drop(&mut self) {
        self.coeffs.zeroize();
    }
}

/// The result: this participant's secret share and the PUBLIC data every verifier needs.
pub struct KeyShare {
    pub index: u32,
    /// SECRET. Zeroized on drop.
    pub secret: Scalar,
    /// The election public key `Y`.
    pub group_public_key: ProjectivePoint,
    /// `(j, s_j * G)` for every participant (to verify decryption shares).
    pub verification_keys: Vec<(u32, ProjectivePoint)>,
}

impl Drop for KeyShare {
    fn drop(&mut self) {
        self.secret.zeroize();
    }
}

fn pop_challenge(
    session: &[u8],
    index: u32,
    t: usize,
    n: u32,
    c0: &ProjectivePoint,
    r: &ProjectivePoint,
) -> Scalar {
    let meta = [
        index.to_be_bytes(),
        (t as u32).to_be_bytes(),
        n.to_be_bytes(),
    ]
    .concat();
    let (cb, rb) = (point_to_bytes(c0), point_to_bytes(r));
    let h = tagged_hash(tags::DKG_POP, &[session, &meta, &cb, &rb]);
    Scalar::reduce(&FieldBytes::from(h))
}

/// `sum_k C_k * x^k`, the public value of a participant's polynomial at `x`.
fn eval_commitments(commitments: &[ProjectivePoint], x: u32) -> ProjectivePoint {
    let xs = Scalar::from(u64::from(x));
    let mut pow = Scalar::ONE;
    let mut acc = ProjectivePoint::IDENTITY;
    for c in commitments {
        acc += *c * pow;
        pow *= xs;
    }
    acc
}

impl Participant {
    /// Starts round 1. `session` binds the ceremony (e.g. chain id + election id) so a
    /// proof from one ceremony cannot be replayed in another.
    pub fn new<R: Rng + CryptoRng>(
        rng: &mut R,
        session: &[u8],
        index: u32,
        n: u32,
        threshold: usize,
    ) -> Result<(Self, Round1), DkgError> {
        if index == 0 || index > n || threshold == 0 || threshold > n as usize {
            return Err(DkgError::BadParameters);
        }
        let coeffs: Vec<Scalar> = (0..threshold).map(|_| Scalar::random(rng)).collect();
        let commitments: Vec<ProjectivePoint> = coeffs
            .iter()
            .map(|a| ProjectivePoint::GENERATOR * *a)
            .collect();
        let a0 = *coeffs.first().ok_or(DkgError::BadParameters)?;
        let c0 = *commitments.first().ok_or(DkgError::BadParameters)?;
        let mut k = Scalar::random(rng);
        let pop_r = ProjectivePoint::GENERATOR * k;
        let c = pop_challenge(session, index, threshold, n, &c0, &pop_r);
        let pop_s = k + c * a0;
        k.zeroize();
        Ok((
            Self { index, n, coeffs },
            Round1 {
                index,
                commitments,
                pop_r,
                pop_s,
            },
        ))
    }

    pub fn index(&self) -> u32 {
        self.index
    }

    /// The secret share this participant sends to participant `j`: `f(j)`.
    pub fn share_for(&self, j: u32) -> Option<Scalar> {
        if j == 0 || j > self.n {
            return None;
        }
        let x = Scalar::from(u64::from(j));
        // Horner
        Some(
            self.coeffs
                .iter()
                .rev()
                .fold(Scalar::ZERO, |acc, a| acc * x + *a),
        )
    }

    /// Finishes with every participant's round-1 package (including this one's) and
    /// the secret shares received from each participant (including its own `share_for(self)`).
    /// Verifies EVERYTHING; any failure names the culprit.
    pub fn finish(
        &self,
        session: &[u8],
        threshold: usize,
        packages: &[Round1],
        received: &[(u32, Scalar)],
    ) -> Result<KeyShare, DkgError> {
        verify_packages(session, self.n, threshold, packages)?;
        let mut secret = Scalar::ZERO;
        for p in packages {
            let (_, share) = received
                .iter()
                .find(|(from, _)| *from == p.index)
                .ok_or(DkgError::Missing(p.index))?;
            if !verify_share(p, self.index, share) {
                return Err(DkgError::BadShare(p.index));
            }
            secret += *share;
        }
        let (group_public_key, verification_keys) = public_outputs(self.n, packages)?;
        Ok(KeyShare {
            index: self.index,
            secret,
            group_public_key,
            verification_keys,
        })
    }
}

/// Checks every round-1 package: one per participant `1..=n`, `threshold`
/// commitments (none the identity at `[0]`), and a valid proof of possession.
pub fn verify_packages(
    session: &[u8],
    n: u32,
    threshold: usize,
    packages: &[Round1],
) -> Result<(), DkgError> {
    for want in 1..=n {
        let p = packages
            .iter()
            .find(|p| p.index == want)
            .ok_or(DkgError::Missing(want))?;
        if packages.iter().filter(|q| q.index == want).count() != 1 {
            return Err(DkgError::BadCommitments(want));
        }
        if p.commitments.len() != threshold {
            return Err(DkgError::BadCommitments(want));
        }
        let c0 = p
            .commitments
            .first()
            .ok_or(DkgError::BadCommitments(want))?;
        if *c0 == ProjectivePoint::IDENTITY {
            return Err(DkgError::BadCommitments(want));
        }
        let c = pop_challenge(session, p.index, threshold, n, c0, &p.pop_r);
        if ProjectivePoint::GENERATOR * p.pop_s != p.pop_r + *c0 * c {
            return Err(DkgError::BadProofOfPossession(want));
        }
    }
    if packages.len() != n as usize {
        return Err(DkgError::BadCommitments(0));
    }
    Ok(())
}

/// Feldman check: `G * share == sum_k C_k * j^k`.
pub fn verify_share(from: &Round1, to_index: u32, share: &Scalar) -> bool {
    ProjectivePoint::GENERATOR * *share == eval_commitments(&from.commitments, to_index)
}

/// Public data anyone can compute from the round-1 packages: the election public
/// key and every participant's verification key.
pub fn public_outputs(
    n: u32,
    packages: &[Round1],
) -> Result<(ProjectivePoint, Vec<(u32, ProjectivePoint)>), DkgError> {
    let mut y = ProjectivePoint::IDENTITY;
    for p in packages {
        y += *p
            .commitments
            .first()
            .ok_or(DkgError::BadCommitments(p.index))?;
    }
    let vks = (1..=n)
        .map(|j| {
            let vk = packages.iter().fold(ProjectivePoint::IDENTITY, |acc, p| {
                acc + eval_commitments(&p.commitments, j)
            });
            (j, vk)
        })
        .collect();
    Ok((y, vks))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::elgamal::{
        Ciphertext, combine_shares, decrypt_share, encrypt_bit, lagrange_at_zero, solve_small_dlog,
    };
    use crate::proofs::{prove_decryption, verify_decryption};
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    const SESSION: &[u8] = b"teko-devnet-1|eleicoes";

    struct Ceremony {
        shares: Vec<KeyShare>,
        packages: Vec<Round1>,
    }

    fn run(seed: u64, n: u32, t: usize) -> Ceremony {
        let mut rng = StdRng::seed_from_u64(seed);
        let parts: Vec<(Participant, Round1)> = (1..=n)
            .map(|i| Participant::new(&mut rng, SESSION, i, n, t).unwrap())
            .collect();
        let packages: Vec<Round1> = parts.iter().map(|p| p.1.clone()).collect();
        let shares = parts
            .iter()
            .map(|(me, _)| {
                let received: Vec<(u32, Scalar)> = parts
                    .iter()
                    .map(|(other, pkg)| (pkg.index, other.share_for(me.index).unwrap()))
                    .collect();
                me.finish(SESSION, t, &packages, &received).unwrap()
            })
            .collect();
        Ceremony { shares, packages }
    }

    #[test]
    fn everyone_agrees_on_the_key_and_any_threshold_of_shares_decrypts() {
        let c = run(1, 3, 2);
        let y = c.shares[0].group_public_key;
        assert!(
            c.shares.iter().all(|s| s.group_public_key == y),
            "same election key for all"
        );
        // the public outputs equal what each participant derived
        let (y2, vks) = public_outputs(3, &c.packages).unwrap();
        assert_eq!(y2, y);
        for s in &c.shares {
            assert_eq!(
                vks.iter().find(|(j, _)| *j == s.index).unwrap().1,
                ProjectivePoint::GENERATOR * s.secret
            );
        }
        // reconstruct the secret from any 2 shares: it matches Y (tests only: never done in production)
        for pair in [[0usize, 1], [0, 2], [1, 2]] {
            let idx: Vec<u32> = pair.iter().map(|i| c.shares[*i].index).collect();
            let secret = pair.iter().fold(Scalar::ZERO, |a, i| {
                a + c.shares[*i].secret * lagrange_at_zero(&idx, c.shares[*i].index).unwrap()
            });
            assert_eq!(ProjectivePoint::GENERATOR * secret, y);
        }
        // end to end: encrypt a count, decrypt with proofs from validators 1 and 3
        let mut rng = StdRng::seed_from_u64(9);
        let ct: Ciphertext = [1u64, 1, 1, 0, 1]
            .iter()
            .map(|m| encrypt_bit(&y, *m, &Scalar::random(&mut rng)))
            .fold(Ciphertext::zero(), |a, c| a + c);
        let mut shares = Vec::new();
        for i in [0usize, 2] {
            let s = &c.shares[i];
            let d = decrypt_share(&s.secret, &ct);
            let proof = prove_decryption(&s.secret, &ct, b"contest-1", &mut rng);
            let vk = vks.iter().find(|(j, _)| *j == s.index).unwrap().1;
            assert!(verify_decryption(&vk, &ct, &d, &proof, b"contest-1"));
            // a wrong share, another contest's context, or another validator's key fail
            assert!(!verify_decryption(
                &vk,
                &ct,
                &(d + ProjectivePoint::GENERATOR),
                &proof,
                b"contest-1"
            ));
            assert!(!verify_decryption(&vk, &ct, &d, &proof, b"contest-2"));
            assert!(!verify_decryption(&vks[1].1, &ct, &d, &proof, b"contest-1"));
            shares.push((s.index, d));
        }
        let m = combine_shares(&ct, &shares).unwrap();
        assert_eq!(solve_small_dlog(&m, 10), Some(4));
    }

    #[test]
    fn a_tampered_commitment_share_or_proof_of_possession_is_detected() {
        let mut rng = StdRng::seed_from_u64(2);
        let (n, t) = (3u32, 2usize);
        let parts: Vec<(Participant, Round1)> = (1..=n)
            .map(|i| Participant::new(&mut rng, SESSION, i, n, t).unwrap())
            .collect();
        let packages: Vec<Round1> = parts.iter().map(|p| p.1.clone()).collect();
        assert!(verify_packages(SESSION, n, t, &packages).is_ok());

        // another ceremony's session: proofs of possession do not verify
        assert_eq!(
            verify_packages(b"outra", n, t, &packages),
            Err(DkgError::BadProofOfPossession(1))
        );
        // wrong threshold
        assert_eq!(
            verify_packages(SESSION, n, 3, &packages),
            Err(DkgError::BadCommitments(1))
        );
        // a missing participant
        assert_eq!(
            verify_packages(SESSION, n, t, &packages[..2]),
            Err(DkgError::Missing(3))
        );
        // tampered commitment: the PoP no longer matches
        let mut bad = packages.clone();
        bad[1].commitments[0] += ProjectivePoint::GENERATOR;
        assert_eq!(
            verify_packages(SESSION, n, t, &bad),
            Err(DkgError::BadProofOfPossession(2))
        );
        // ROGUE KEY: participant 3 claims C0' = G*x - (C0_1 + C0_2) to bias Y to a key it knows.
        // It cannot prove possession of the discrete log of C0', so the package is refused.
        let x = Scalar::random(&mut rng);
        let rogue_c0 = ProjectivePoint::GENERATOR * x
            - packages[0].commitments[0]
            - packages[1].commitments[0];
        let mut rogue = packages.clone();
        rogue[2].commitments[0] = rogue_c0;
        assert_eq!(
            verify_packages(SESSION, n, t, &rogue),
            Err(DkgError::BadProofOfPossession(3))
        );
        // identity as the public part
        let mut ident = packages.clone();
        ident[0].commitments[0] = ProjectivePoint::IDENTITY;
        assert_eq!(
            verify_packages(SESSION, n, t, &ident),
            Err(DkgError::BadCommitments(1))
        );

        // a wrong secret share is caught against the commitments and names the culprit
        let me = &parts[0].0;
        let mut received: Vec<(u32, Scalar)> = parts
            .iter()
            .map(|(o, p)| (p.index, o.share_for(1).unwrap()))
            .collect();
        received[1].1 += Scalar::ONE;
        assert!(matches!(
            me.finish(SESSION, t, &packages, &received),
            Err(DkgError::BadShare(2))
        ));
        // a missing share (restore the tampered one first)
        received[1].1 -= Scalar::ONE;
        received.remove(2);
        assert!(matches!(
            me.finish(SESSION, t, &packages, &received),
            Err(DkgError::Missing(3))
        ));
        // parameters
        assert!(Participant::new(&mut rng, SESSION, 0, 3, 2).is_err());
        assert!(Participant::new(&mut rng, SESSION, 4, 3, 2).is_err());
        assert!(Participant::new(&mut rng, SESSION, 1, 3, 4).is_err());
        assert!(Participant::new(&mut rng, SESSION, 1, 3, 0).is_err());
        assert!(me.share_for(0).is_none() && me.share_for(4).is_none());
    }

    #[test]
    fn works_for_other_sizes_too() {
        for (n, t) in [(2u32, 2usize), (3, 3), (5, 3), (4, 1)] {
            let c = run(u64::from(n) * 10 + t as u64, n, t);
            let y = c.shares[0].group_public_key;
            let idx: Vec<u32> = (1..=t as u32).collect();
            let secret = c.shares.iter().take(t).fold(Scalar::ZERO, |a, s| {
                a + s.secret * lagrange_at_zero(&idx, s.index).unwrap()
            });
            assert_eq!(ProjectivePoint::GENERATOR * secret, y, "n={n} t={t}");
        }
    }
}
