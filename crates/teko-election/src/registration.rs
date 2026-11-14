//! Anonymous voter registration, election side (ADR-0008).
//!
//! The authority signs a BLINDED message, so it never sees the voter key. The
//! voter unblinds the signature into a token and publishes
//! `(voter_key, token)`. The tally accepts a ballot only if its voter key has a
//! valid token for THAT contest. The RSA math lives in `teko-registration`
//! (std); this crate stays `no_std` and sees it only through the trait below.

use alloc::string::String;
use alloc::vec::Vec;

use prost::Message;
use teko_crypto::{tagged_hash, tags};

/// Checks a blind-RSA token against a contest's registration key.
pub trait RegistrationVerifier: Send + Sync {
    fn verify(&self, registration_key: &[u8], message: &[u8], token: &[u8]) -> bool;
}

/// Voter-side blinding, DETERMINISTIC from a device-secret `seed`: asking twice
/// yields the same blinded message and token, so the device needs no pending
/// state and a lost reply can simply be retried (the authority is idempotent).
pub trait RegistrationBlinder {
    /// The blinded message to send to the authority.
    fn blind(&self, registration_key: &[u8], message: &[u8], seed: &[u8; 32]) -> Option<Vec<u8>>;
    /// Unblinds the authority's reply into a token, CHECKING it first.
    fn unblind(
        &self,
        registration_key: &[u8],
        message: &[u8],
        seed: &[u8; 32],
        blind_signature: &[u8],
    ) -> Option<Vec<u8>>;
}

/// What the authority blind-signs. Binds election, contest and voter key, so a
/// token is useless in another contest, another election or for another key.
pub fn registration_message(election_id: &str, contest_id: &str, voter_key: &[u8]) -> Vec<u8> {
    tagged_hash(
        tags::REGISTRATION,
        &[election_id.as_bytes(), contest_id.as_bytes(), voter_key],
    )
    .to_vec()
}

/// What the device's identity key signs when asking for a registration.
pub fn registration_request_message(
    election_id: &str,
    contest_id: &str,
    blind_message: &[u8],
) -> Vec<u8> {
    tagged_hash(
        tags::REGISTRATION_REQUEST,
        &[election_id.as_bytes(), contest_id.as_bytes(), blind_message],
    )
    .to_vec()
}

/// A registration as published on the board. Carries NO identity.
#[derive(Clone, PartialEq, Message)]
pub struct SubmittedRegistration {
    #[prost(string, tag = "1")]
    pub election_id: String,
    #[prost(string, tag = "2")]
    pub contest_id: String,
    #[prost(bytes = "vec", tag = "3")]
    pub voter_key: Vec<u8>,
    #[prost(bytes = "vec", tag = "4")]
    pub token: Vec<u8>,
}
