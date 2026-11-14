//! Credentials with selective disclosure (simplified SD-JWT model, ADR-0005).
//!
//! The issuer signs the *digests* of salted claims. The holder keeps the claims
//! and discloses only the chosen ones. Predicates such as "age >= 18" use
//! issuer-derived boolean claims (`age_over_18`), as ADR-0005 recommends.
//! Known limitation: presentations of the same credential are linkable.

use alloc::string::String;
use alloc::vec::Vec;

use p256::ecdsa::SigningKey;
use prost::Message;
use rand_core::{CryptoRng, Rng};

use crate::error::CryptoError;
use crate::keys::{public_key_bytes, tagged_hash};
use crate::sign::{sign_tagged, verify_tagged};
use crate::tags;

#[derive(Clone, PartialEq, Message)]
pub struct Credential {
    #[prost(string, tag = "1")]
    pub issuer: String,
    #[prost(bytes = "vec", tag = "2")]
    pub subject_key: Vec<u8>,
    #[prost(uint64, tag = "3")]
    pub issued_at: u64,
    #[prost(uint64, tag = "4")]
    pub expires_at: u64,
    #[prost(bytes = "vec", tag = "5")]
    pub credential_id: Vec<u8>,
    /// Sorted digests of the claims (order hides claim positions).
    #[prost(bytes = "vec", repeated, tag = "6")]
    pub claim_digests: Vec<Vec<u8>>,
    #[prost(bytes = "vec", tag = "7")]
    pub signature: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
pub struct Claim {
    #[prost(string, tag = "1")]
    pub name: String,
    #[prost(string, tag = "2")]
    pub value: String,
    #[prost(bytes = "vec", tag = "3")]
    pub salt: Vec<u8>,
}

/// What the issuer hands to the holder (device): credential + all claims.
#[derive(Clone, PartialEq, Message)]
pub struct IssuedCredential {
    #[prost(message, optional, tag = "1")]
    pub credential: Option<Credential>,
    #[prost(message, repeated, tag = "2")]
    pub claims: Vec<Claim>,
}

#[derive(Clone, PartialEq, Message)]
pub struct Presentation {
    #[prost(message, optional, tag = "1")]
    pub credential: Option<Credential>,
    #[prost(message, repeated, tag = "2")]
    pub disclosed: Vec<Claim>,
    #[prost(bytes = "vec", tag = "3")]
    pub nonce: Vec<u8>,
    #[prost(string, tag = "4")]
    pub verifier: String,
    #[prost(bytes = "vec", tag = "5")]
    pub holder_signature: Vec<u8>,
}

pub fn claim_digest(c: &Claim) -> [u8; 32] {
    tagged_hash(
        tags::CLAIM,
        &[c.salt.as_slice(), c.name.as_bytes(), c.value.as_bytes()],
    )
}

fn credential_signing_bytes(c: &Credential) -> Vec<u8> {
    let mut unsigned = c.clone();
    unsigned.signature = Vec::new();
    unsigned.encode_to_vec()
}

fn presentation_signing_bytes(p: &Presentation) -> Vec<u8> {
    let mut unsigned = p.clone();
    unsigned.holder_signature = Vec::new();
    unsigned.encode_to_vec()
}

#[allow(clippy::too_many_arguments)]
pub fn issue<R: Rng + CryptoRng>(
    issuer_key: &SigningKey,
    issuer_name: &str,
    subject_key: &[u8],
    issued_at: u64,
    expires_at: u64,
    credential_id: [u8; 16],
    attributes: &[(String, String)],
    rng: &mut R,
) -> IssuedCredential {
    let claims: Vec<Claim> = attributes
        .iter()
        .map(|(name, value)| {
            let mut salt = [0u8; 16];
            rng.fill_bytes(&mut salt);
            Claim {
                name: name.clone(),
                value: value.clone(),
                salt: salt.to_vec(),
            }
        })
        .collect();
    let mut digests: Vec<Vec<u8>> = claims.iter().map(|c| claim_digest(c).to_vec()).collect();
    digests.sort();
    let mut cred = Credential {
        issuer: String::from(issuer_name),
        subject_key: subject_key.to_vec(),
        issued_at,
        expires_at,
        credential_id: credential_id.to_vec(),
        claim_digests: digests,
        signature: Vec::new(),
    };
    cred.signature = sign_tagged(
        issuer_key,
        tags::CREDENTIAL,
        &credential_signing_bytes(&cred),
    );
    IssuedCredential {
        credential: Some(cred),
        claims,
    }
}

fn verify_credential_signature(issuer_pk: &[u8], c: &Credential) -> Result<(), CryptoError> {
    verify_tagged(
        issuer_pk,
        tags::CREDENTIAL,
        &credential_signing_bytes(c),
        &c.signature,
    )
}

/// Full check of what a holder receives: signature + every claim matches a digest.
pub fn verify_issued(issuer_pk: &[u8], issued: &IssuedCredential) -> Result<(), CryptoError> {
    let cred = issued
        .credential
        .as_ref()
        .ok_or(CryptoError::MalformedCredential)?;
    verify_credential_signature(issuer_pk, cred)?;
    for claim in &issued.claims {
        let d = claim_digest(claim);
        if !cred.claim_digests.iter().any(|x| x.as_slice() == d) {
            return Err(CryptoError::MalformedCredential);
        }
    }
    Ok(())
}

/// Builds a presentation disclosing only `disclose` claims, signed by the
/// holder over the verifier's nonce.
pub fn present(
    holder_key: &SigningKey,
    issued: &IssuedCredential,
    disclose: &[&str],
    nonce: &[u8],
    verifier: &str,
) -> Result<Presentation, CryptoError> {
    let cred = issued
        .credential
        .clone()
        .ok_or(CryptoError::MalformedCredential)?;
    if cred.subject_key != public_key_bytes(holder_key) {
        return Err(CryptoError::InvalidKey);
    }
    let mut disclosed = Vec::new();
    for name in disclose {
        let claim = issued
            .claims
            .iter()
            .find(|c| c.name == *name)
            .ok_or(CryptoError::InvalidArgument)?;
        disclosed.push(claim.clone());
    }
    let mut p = Presentation {
        credential: Some(cred),
        disclosed,
        nonce: nonce.to_vec(),
        verifier: String::from(verifier),
        holder_signature: Vec::new(),
    };
    p.holder_signature = sign_tagged(
        holder_key,
        tags::ATTR_PROOF,
        &presentation_signing_bytes(&p),
    );
    Ok(p)
}

/// Verifier side. `now` is unix seconds (0 skips the expiry check).
/// Returns the disclosed `(name, value)` pairs.
pub fn verify_presentation(
    issuer_pk: &[u8],
    p: &Presentation,
    expected_nonce: &[u8],
    now: u64,
) -> Result<Vec<(String, String)>, CryptoError> {
    let cred = p
        .credential
        .as_ref()
        .ok_or(CryptoError::MalformedCredential)?;
    verify_credential_signature(issuer_pk, cred)?;
    if now != 0 && cred.expires_at != 0 && now > cred.expires_at {
        return Err(CryptoError::Expired);
    }
    if p.nonce != expected_nonce {
        return Err(CryptoError::NonceMismatch);
    }
    verify_tagged(
        &cred.subject_key,
        tags::ATTR_PROOF,
        &presentation_signing_bytes(p),
        &p.holder_signature,
    )?;
    let mut out = Vec::new();
    for claim in &p.disclosed {
        let d = claim_digest(claim);
        if !cred.claim_digests.iter().any(|x| x.as_slice() == d) {
            return Err(CryptoError::MalformedCredential);
        }
        out.push((claim.name.clone(), claim.value.clone()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::generate_signing_key;
    use alloc::string::ToString;
    use proptest::prelude::*;
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    fn attrs() -> Vec<(String, String)> {
        [
            ("name", "Maria da Silva"),
            ("birth_date", "1990-05-17"),
            ("age_over_18", "true"),
        ]
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect()
    }

    proptest! {
        #[test]
        fn selective_disclosure_roundtrip(seed in any::<u64>(), nonce in proptest::collection::vec(any::<u8>(), 8..32)) {
            let mut rng = StdRng::seed_from_u64(seed);
            let issuer = generate_signing_key(&mut rng);
            let holder = generate_signing_key(&mut rng);
            let issued = issue(&issuer, "Autoridade", &public_key_bytes(&holder), 1, 0, [7; 16], &attrs(), &mut rng);
            prop_assert!(verify_issued(&public_key_bytes(&issuer), &issued).is_ok());

            let p = present(&holder, &issued, &["age_over_18"], &nonce, "loja").unwrap();
            let got = verify_presentation(&public_key_bytes(&issuer), &p, &nonce, 0).unwrap();
            prop_assert_eq!(got, alloc::vec![("age_over_18".to_string(), "true".to_string())]);
            // birth date is not in the presentation
            let bytes = p.encode_to_vec();
            prop_assert!(!bytes.windows(10).any(|w| w == b"1990-05-17"));
        }

        #[test]
        fn wrong_nonce_forged_claim_or_other_issuer_fails(seed in any::<u64>()) {
            let mut rng = StdRng::seed_from_u64(seed);
            let issuer = generate_signing_key(&mut rng);
            let other = generate_signing_key(&mut rng);
            let holder = generate_signing_key(&mut rng);
            let issued = issue(&issuer, "A", &public_key_bytes(&holder), 1, 0, [1; 16], &attrs(), &mut rng);
            let p = present(&holder, &issued, &["age_over_18"], b"nonce-1", "v").unwrap();
            let ipk = public_key_bytes(&issuer);
            prop_assert_eq!(verify_presentation(&ipk, &p, b"nonce-2", 0), Err(CryptoError::NonceMismatch));
            prop_assert!(verify_presentation(&public_key_bytes(&other), &p, b"nonce-1", 0).is_err());
            let mut forged = p.clone();
            forged.disclosed[0].value = "false-but-forged".to_string();
            prop_assert!(verify_presentation(&ipk, &forged, b"nonce-1", 0).is_err());
        }
    }

    #[test]
    fn credential_bound_to_holder_key() {
        let mut rng = StdRng::seed_from_u64(9);
        let issuer = generate_signing_key(&mut rng);
        let holder = generate_signing_key(&mut rng);
        let thief = generate_signing_key(&mut rng);
        let issued = issue(
            &issuer,
            "A",
            &public_key_bytes(&holder),
            1,
            0,
            [1; 16],
            &attrs(),
            &mut rng,
        );
        assert!(present(&thief, &issued, &["name"], b"n", "v").is_err());
    }

    #[test]
    fn expiry_is_enforced() {
        let mut rng = StdRng::seed_from_u64(3);
        let issuer = generate_signing_key(&mut rng);
        let holder = generate_signing_key(&mut rng);
        let issued = issue(
            &issuer,
            "A",
            &public_key_bytes(&holder),
            1,
            100,
            [1; 16],
            &attrs(),
            &mut rng,
        );
        let p = present(&holder, &issued, &["name"], b"n", "v").unwrap();
        let ipk = public_key_bytes(&issuer);
        assert!(verify_presentation(&ipk, &p, b"n", 50).is_ok());
        assert_eq!(
            verify_presentation(&ipk, &p, b"n", 101),
            Err(CryptoError::Expired)
        );
    }
}
