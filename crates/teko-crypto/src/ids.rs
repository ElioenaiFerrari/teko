use alloc::string::String;
use alloc::vec::Vec;

use bech32::primitives::decode::CheckedHrpstring;
use bech32::{Bech32m, Hrp};

use crate::error::CryptoError;
use crate::keys::tagged_hash;
use crate::tags;

pub const CIVIC_HRP: &str = "teko";
pub const PAIRWISE_HRP: &str = "tekop";
const ID_VERSION: u8 = 1;
const ID_BYTES: usize = 20; // 160 bits

fn encode_id(hrp: &str, digest: &[u8; 32]) -> Result<String, CryptoError> {
    let hrp = Hrp::parse(hrp).map_err(|_| CryptoError::InvalidId)?;
    let mut data = Vec::with_capacity(1 + ID_BYTES);
    data.push(ID_VERSION);
    data.extend_from_slice(digest.get(..ID_BYTES).ok_or(CryptoError::InvalidId)?);
    bech32::encode::<Bech32m>(hrp, &data).map_err(|_| CryptoError::InvalidId)
}

/// Civic ID = bech32m("teko", version ‖ trunc160(SHA3-256(tag ‖ version ‖ pk))).
pub fn civic_id(identity_public_key: &[u8]) -> Result<String, CryptoError> {
    let d = tagged_hash(tags::CIVIC_ID, &[&[ID_VERSION], identity_public_key]);
    encode_id(CIVIC_HRP, &d)
}

/// Validates the bech32m checksum, HRP and version; returns the 20-byte payload.
pub fn parse_civic_id(s: &str) -> Result<[u8; ID_BYTES], CryptoError> {
    let c = CheckedHrpstring::new::<Bech32m>(s).map_err(|_| CryptoError::InvalidId)?;
    if c.hrp().as_str() != CIVIC_HRP {
        return Err(CryptoError::InvalidId);
    }
    let data: Vec<u8> = c.byte_iter().collect();
    match data.split_first() {
        Some((&ID_VERSION, rest)) => rest.try_into().map_err(|_| CryptoError::InvalidId),
        _ => Err(CryptoError::InvalidId),
    }
}

/// Canonical form of a service identifier (so variants map to the same ID).
pub fn canonical_service_id(service: &str) -> Result<String, CryptoError> {
    let s = service.trim().to_ascii_lowercase();
    if s.is_empty() || s.len() > 128 || s.chars().any(|c| c.is_control()) {
        return Err(CryptoError::InvalidArgument);
    }
    Ok(s)
}

/// Pairwise ID = bech32m("tekop", version ‖ trunc160(H(tag, k_pairwise, service))).
pub fn pairwise_id(pairwise_key: &[u8; 32], service: &str) -> Result<String, CryptoError> {
    let svc = canonical_service_id(service)?;
    let d = tagged_hash(
        tags::PAIRWISE_ID,
        &[pairwise_key.as_slice(), svc.as_bytes()],
    );
    encode_id(PAIRWISE_HRP, &d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const CHARSET: &[u8] = b"qpzry9x8gf2tvdw0s3jn54khce6mua7l";

    proptest! {
        #[test]
        fn civic_id_valid_and_detects_single_typos(pk in proptest::collection::vec(any::<u8>(), 33), pos in any::<usize>(), repl in 0usize..32) {
            let id = civic_id(&pk).unwrap();
            prop_assert!(parse_civic_id(&id).is_ok());
            // mutate one data-part character
            let sep = id.rfind('1').unwrap();
            let idx = sep + 1 + pos % (id.len() - sep - 1);
            let mut bytes = id.clone().into_bytes();
            let new = CHARSET[repl];
            prop_assume!(bytes[idx] != new);
            bytes[idx] = new;
            let bad = String::from_utf8(bytes).unwrap();
            prop_assert!(parse_civic_id(&bad).is_err());
        }

        #[test]
        fn pairwise_ids_differ_across_services_and_keys(k1 in any::<[u8;32]>(), k2 in any::<[u8;32]>(), a in "[a-z]{1,20}", b in "[a-z]{1,20}") {
            prop_assume!(a != b);
            prop_assert_ne!(pairwise_id(&k1, &a).unwrap(), pairwise_id(&k1, &b).unwrap());
            prop_assume!(k1 != k2);
            prop_assert_ne!(pairwise_id(&k1, &a).unwrap(), pairwise_id(&k2, &a).unwrap());
        }

        #[test]
        fn pairwise_is_stable_and_canonical(k in any::<[u8;32]>(), a in "[a-z]{1,20}") {
            let upper = alloc::format!("  {}  ", a.to_ascii_uppercase());
            prop_assert_eq!(pairwise_id(&k, &a).unwrap(), pairwise_id(&k, &upper).unwrap());
        }
    }

    #[test]
    fn civic_id_has_hrp_and_is_not_pairwise() {
        let id = civic_id(&[2u8; 33]).unwrap();
        assert!(id.starts_with("teko1"));
        let p = pairwise_id(&[1u8; 32], "x").unwrap();
        assert!(p.starts_with("tekop1"));
        assert!(parse_civic_id(&p).is_err());
    }

    #[test]
    fn rejects_empty_or_control_service() {
        assert!(canonical_service_id("  ").is_err());
        assert!(canonical_service_id("a\nb").is_err());
    }
}
