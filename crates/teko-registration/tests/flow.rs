//! End to end with a real 2048-bit key: blind → sign → unblind → verify.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use teko_election::{RegistrationBlinder, RegistrationVerifier, registration_message};
use teko_registration::{IssuerKey, RsaRegistration};

fn issuer() -> (IssuerKey, Vec<u8>) {
    let k = IssuerKey::generate(2048).unwrap();
    let spki = k.public_spki().unwrap();
    (k, spki)
}

#[test]
fn full_flow_with_the_adapter_and_deterministic_retry() {
    let (issuer, spki) = issuer();
    let msg = registration_message("e", "presidente", &[2u8; 33]);
    let seed = [7u8; 32];
    let b = RsaRegistration;

    let blinded = b.blind(&spki, &msg, &seed).unwrap();
    // Deterministic: a lost reply can be retried with the SAME blinded message.
    assert_eq!(blinded, b.blind(&spki, &msg, &seed).unwrap());
    let sig = issuer.blind_sign(&blinded).unwrap();
    let token = b.unblind(&spki, &msg, &seed, &sig.0).unwrap();
    assert_eq!(token, b.unblind(&spki, &msg, &seed, &sig.0).unwrap());
    assert!(b.verify(&spki, &msg, &token));

    // Bound to the message, to the key and to the token bytes.
    let other = registration_message("e", "governador-sp", &[2u8; 33]);
    assert!(!b.verify(&spki, &other, &token), "other contest");
    let (_, spki2) = issuer_pair();
    assert!(!b.verify(&spki2, &msg, &token), "other authority key");
    let mut bad = token.clone();
    bad[40] ^= 1;
    assert!(!b.verify(&spki, &msg, &bad), "tampered token");
    assert!(!b.verify(&spki, &msg, &[]), "empty token");
}

fn issuer_pair() -> (IssuerKey, Vec<u8>) {
    issuer()
}

#[test]
fn different_seeds_blind_differently_and_the_authority_cannot_link() {
    let (issuer, spki) = issuer();
    let msg = registration_message("e", "c", &[3u8; 33]);
    let b = RsaRegistration;
    let b1 = b.blind(&spki, &msg, &[1u8; 32]).unwrap();
    let b2 = b.blind(&spki, &msg, &[2u8; 32]).unwrap();
    assert_ne!(
        b1, b2,
        "same message, different seed => unrelated blinded values"
    );
    // The signature the authority sees is NOT the token it will later see.
    let sig = issuer.blind_sign(&b1).unwrap();
    let token = b.unblind(&spki, &msg, &[1u8; 32], &sig.0).unwrap();
    assert!(!token.windows(sig.0.len()).any(|w| w == sig.0.as_slice()));
}

#[test]
fn a_wrong_or_forged_blind_signature_never_becomes_a_token() {
    let (_, spki) = issuer();
    let (other_issuer, _) = issuer_pair();
    let msg = registration_message("e", "c", &[4u8; 33]);
    let seed = [9u8; 32];
    let b = RsaRegistration;
    let blinded = b.blind(&spki, &msg, &seed).unwrap();
    // Signed by a different authority key:
    // (a different modulus may refuse a larger input outright: use junk then)
    let forged = other_issuer
        .blind_sign(&blinded)
        .map_or_else(|_| vec![0x55u8; 256], |s| s.0);
    assert!(b.unblind(&spki, &msg, &seed, &forged).is_none());
    assert!(b.unblind(&spki, &msg, &seed, &[0u8; 5]).is_none());
}
