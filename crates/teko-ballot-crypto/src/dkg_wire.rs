//! Wire/on-chain encodings of the DKG public data. The record published on the
//! bulletin board lets ANYONE re-verify the ceremony (proofs of possession) and
//! recompute the election key and every validator's verification key.

use alloc::vec::Vec;

use prost::Message;

use crate::codec::{point_from_bytes, point_to_bytes, scalar_from_bytes, scalar_to_bytes};
use crate::dkg::{DkgError, Round1, public_outputs, verify_packages};
use p256::ProjectivePoint;

#[derive(Clone, PartialEq, Message)]
pub struct Round1Msg {
    #[prost(uint32, tag = "1")]
    pub index: u32,
    #[prost(bytes = "vec", repeated, tag = "2")]
    pub commitments: Vec<Vec<u8>>,
    #[prost(bytes = "vec", tag = "3")]
    pub pop_r: Vec<u8>,
    #[prost(bytes = "vec", tag = "4")]
    pub pop_s: Vec<u8>,
}

impl From<&Round1> for Round1Msg {
    fn from(r: &Round1) -> Self {
        Self {
            index: r.index,
            commitments: r
                .commitments
                .iter()
                .map(|c| point_to_bytes(c).to_vec())
                .collect(),
            pop_r: point_to_bytes(&r.pop_r).to_vec(),
            pop_s: scalar_to_bytes(&r.pop_s).to_vec(),
        }
    }
}

impl Round1Msg {
    pub fn to_round1(&self) -> Option<Round1> {
        Some(Round1 {
            index: self.index,
            commitments: self
                .commitments
                .iter()
                .map(|c| point_from_bytes(c))
                .collect::<Option<Vec<_>>>()?,
            pop_r: point_from_bytes(&self.pop_r)?,
            pop_s: scalar_from_bytes(&self.pop_s)?,
        })
    }
}

/// What the key ceremony leaves on the chain.
#[derive(Clone, PartialEq, Message)]
pub struct DkgRecord {
    #[prost(bytes = "vec", tag = "1")]
    pub session: Vec<u8>,
    #[prost(uint32, tag = "2")]
    pub threshold: u32,
    #[prost(uint32, tag = "3")]
    pub n: u32,
    #[prost(message, repeated, tag = "4")]
    pub packages: Vec<Round1Msg>,
    /// The election public key `Y` (33 bytes), recomputable from `packages`.
    #[prost(bytes = "vec", tag = "5")]
    pub group_key: Vec<u8>,
}

/// What a verifier learns from a RE-CHECKED record.
pub struct VerifiedCeremony {
    pub group_key: ProjectivePoint,
    pub verification_keys: Vec<(u32, ProjectivePoint)>,
    pub threshold: usize,
}

impl DkgRecord {
    /// Re-verifies everything (shapes, every proof of possession) and recomputes the
    /// public outputs; refuses a record whose `group_key` is not what its packages give.
    pub fn verify(&self) -> Result<VerifiedCeremony, DkgError> {
        let packages: Vec<Round1> = self
            .packages
            .iter()
            .map(Round1Msg::to_round1)
            .collect::<Option<Vec<_>>>()
            .ok_or(DkgError::BadCommitments(0))?;
        let t = self.threshold as usize;
        verify_packages(&self.session, self.n, t, &packages)?;
        let (y, vks) = public_outputs(self.n, &packages)?;
        if point_to_bytes(&y).as_slice() != self.group_key.as_slice() {
            return Err(DkgError::BadCommitments(0));
        }
        Ok(VerifiedCeremony {
            group_key: y,
            verification_keys: vks,
            threshold: t,
        })
    }
}
