//! What a voter's device signs: one option in one contest of one election.

use alloc::string::String;
use alloc::vec::Vec;

use prost::Message;
use teko_crypto::{SigningKey, sign_tagged, tags, verify_tagged};

use crate::error::ElectionError;

/// Canonical bytes of a ballot. Binding the `contest_hash` ties the signature to
/// those exact candidates, jurisdiction and dates: it cannot be replayed on a
/// contest that was altered, and `option_index` is checked against the manifest.
#[derive(Clone, PartialEq, Message)]
pub struct BallotBody {
    #[prost(string, tag = "1")]
    pub election_id: String,
    #[prost(string, tag = "2")]
    pub contest_id: String,
    #[prost(bytes = "vec", tag = "3")]
    pub contest_hash: Vec<u8>,
    #[prost(uint32, tag = "4")]
    pub option_index: u32,
}

pub fn sign_ballot(voter_key: &SigningKey, body: &BallotBody) -> Vec<u8> {
    sign_tagged(voter_key, tags::BALLOT, &body.encode_to_vec())
}

pub fn verify_ballot(
    voter_public_key: &[u8],
    body: &BallotBody,
    signature: &[u8],
) -> Result<(), ElectionError> {
    verify_tagged(
        voter_public_key,
        tags::BALLOT,
        &body.encode_to_vec(),
        signature,
    )
    .map_err(|_| ElectionError::InvalidManifest("ballot signature does not verify"))
}

/// A manifest plus the governance transitions seen so far, as one file/payload.
#[derive(Clone, PartialEq, Message)]
pub struct ElectionBundle {
    #[prost(bytes = "vec", tag = "1")]
    pub manifest: Vec<u8>,
    #[prost(bytes = "vec", repeated, tag = "2")]
    pub transitions: Vec<Vec<u8>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand::rngs::StdRng;
    use teko_crypto::{generate_signing_key, public_key_bytes};

    fn body(i: u32) -> BallotBody {
        BallotBody {
            election_id: "eleicoes-2026".into(),
            contest_id: "presidente".into(),
            contest_hash: vec![7; 32],
            option_index: i,
        }
    }

    #[test]
    fn signature_binds_every_field() {
        let k = generate_signing_key(&mut StdRng::seed_from_u64(3));
        let pk = public_key_bytes(&k);
        let sig = sign_ballot(&k, &body(1));
        assert!(verify_ballot(&pk, &body(1), &sig).is_ok());
        assert!(verify_ballot(&pk, &body(0), &sig).is_err(), "other option");
        let mut b = body(1);
        b.contest_id = "governador-sp".into();
        assert!(verify_ballot(&pk, &b, &sig).is_err(), "other contest");
        let mut b = body(1);
        b.contest_hash = vec![8; 32];
        assert!(
            verify_ballot(&pk, &b, &sig).is_err(),
            "other contest version"
        );
        let mut b = body(1);
        b.election_id = "outra".into();
        assert!(verify_ballot(&pk, &b, &sig).is_err(), "other election");
        let other = public_key_bytes(&generate_signing_key(&mut StdRng::seed_from_u64(4)));
        assert!(
            verify_ballot(&other, &body(1), &sig).is_err(),
            "other voter"
        );
    }
}
