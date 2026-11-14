//! Checks the implementation against the RFC 9474 Appendix A.1 test vector
//! (RSABSSA-SHA384-PSS-Randomized, RSA-4096). Every intermediate value of the
//! RFC must be reproduced EXACTLY: EMSA-PSS encoding, the blinded message, the
//! blind signature and the final signature.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod rfc9474_a1;

use crypto_bigint::BoxedUint;
use rfc9474_a1 as v;
use rsa::pkcs8::EncodePublicKey;
use rsa::{PrivateKey, PublicKey};
use teko_registration::{BlindSignature, IssuerKey, Token, blind_with, finalize, verify};

fn h(s: &str) -> Vec<u8> {
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
        .collect()
}

fn big(s: &str, bits: u32) -> BoxedUint {
    BoxedUint::from_be_slice(&h(s), bits).unwrap()
}

fn keys() -> (IssuerKey, Vec<u8>) {
    let bits = 4096;
    let (n, e, d) = (big(v::N, bits), big(v::E, bits), big(v::D, bits));
    let primes = vec![big(v::P, bits / 2), big(v::Q, bits / 2)];
    let sk = PrivateKey::from_components(n.clone(), e.clone(), d, primes).unwrap();
    let spki = PublicKey::new(n, e)
        .unwrap()
        .to_public_key_der()
        .unwrap()
        .as_bytes()
        .to_vec();
    (IssuerKey::from_rsa(sk), spki)
}

#[test]
fn emsa_pss_encoding_matches_the_rfc() {
    let em = teko_registration::emsa_pss_encode_for_tests(
        &h(v::PREPARED_MSG),
        4095,
        &h(v::SALT).try_into().unwrap(),
    )
    .unwrap();
    assert_eq!(em, h(v::ENCODED_MSG));
}

#[test]
fn blinding_matches_the_rfc_vector() {
    let (_, spki) = keys();
    let prefix: [u8; 32] = h(v::MSG_PREFIX).try_into().unwrap();
    let salt: [u8; 48] = h(v::SALT).try_into().unwrap();
    let b = blind_with(&spki, &h(v::MSG), prefix, &salt, &h(v::INV)).unwrap();
    assert_eq!(b.blind_message, h(v::BLINDED_MSG));
}

#[test]
fn blind_signing_matches_the_rfc_vector() {
    let (sk, _) = keys();
    let sig = sk.blind_sign(&h(v::BLINDED_MSG)).unwrap();
    assert_eq!(sig.0, h(v::BLIND_SIG));
}

#[test]
fn finalizing_reproduces_the_rfc_signature_and_it_verifies() {
    let (_, spki) = keys();
    let prefix: [u8; 32] = h(v::MSG_PREFIX).try_into().unwrap();
    let salt: [u8; 48] = h(v::SALT).try_into().unwrap();
    let b = blind_with(&spki, &h(v::MSG), prefix, &salt, &h(v::INV)).unwrap();
    let token = finalize(
        &spki,
        &b.state,
        &BlindSignature(h(v::BLIND_SIG)),
        &h(v::MSG),
    )
    .unwrap();
    assert_eq!(token.signature, h(v::SIG));
    assert_eq!(token.msg_prefix.to_vec(), h(v::MSG_PREFIX));
    assert!(verify(&spki, &h(v::MSG), &token).is_ok());
}

#[test]
fn tampering_with_any_part_of_the_rfc_vector_is_rejected() {
    let (_, spki) = keys();
    let prefix: [u8; 32] = h(v::MSG_PREFIX).try_into().unwrap();
    let salt: [u8; 48] = h(v::SALT).try_into().unwrap();
    let b = blind_with(&spki, &h(v::MSG), prefix, &salt, &h(v::INV)).unwrap();
    // a corrupted blind signature never becomes a token
    let mut bad = h(v::BLIND_SIG);
    bad[100] ^= 1;
    assert!(finalize(&spki, &b.state, &BlindSignature(bad), &h(v::MSG)).is_err());
    // the genuine token does not verify for another message, prefix or signature
    let token = finalize(
        &spki,
        &b.state,
        &BlindSignature(h(v::BLIND_SIG)),
        &h(v::MSG),
    )
    .unwrap();
    assert!(verify(&spki, b"another message", &token).is_err());
    let mut t2 = Token {
        msg_prefix: token.msg_prefix,
        signature: token.signature.clone(),
    };
    t2.msg_prefix[0] ^= 1;
    assert!(verify(&spki, &h(v::MSG), &t2).is_err());
    t2 = Token {
        msg_prefix: token.msg_prefix,
        signature: token.signature,
    };
    t2.signature[7] ^= 1;
    assert!(verify(&spki, &h(v::MSG), &t2).is_err());
}
