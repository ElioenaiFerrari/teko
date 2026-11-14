//! Glue to `teko-election`: the voter-side blinder (used by the device) and the
//! token verifier (used by the tally).

use teko_election::{RegistrationBlinder, RegistrationVerifier};

use crate::bsrsa::{BlindSignature, Token, blind_deterministic, finalize, verify};

/// Implements both roles with RFC 9474.
#[derive(Clone, Copy, Debug, Default)]
pub struct RsaRegistration;

impl RegistrationBlinder for RsaRegistration {
    fn blind(&self, registration_key: &[u8], message: &[u8], seed: &[u8; 32]) -> Option<Vec<u8>> {
        blind_deterministic(registration_key, message, seed)
            .ok()
            .map(|r| r.blind_message)
    }

    fn unblind(
        &self,
        registration_key: &[u8],
        message: &[u8],
        seed: &[u8; 32],
        blind_signature: &[u8],
    ) -> Option<Vec<u8>> {
        let state = blind_deterministic(registration_key, message, seed)
            .ok()?
            .state;
        finalize(
            registration_key,
            &state,
            &BlindSignature(blind_signature.to_vec()),
            message,
        )
        .ok()
        .map(|t| t.to_bytes())
    }
}

impl RegistrationVerifier for RsaRegistration {
    fn verify(&self, registration_key: &[u8], message: &[u8], token: &[u8]) -> bool {
        Token::from_bytes(token)
            .ok()
            .is_some_and(|t| verify(registration_key, message, &t).is_ok())
    }
}
