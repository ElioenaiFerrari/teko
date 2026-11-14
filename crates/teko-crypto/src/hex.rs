use alloc::string::String;
use alloc::vec::Vec;

use crate::error::CryptoError;

const DIGITS: &[u8; 16] = b"0123456789abcdef";

pub fn encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(char::from(DIGITS[usize::from(b >> 4)]));
        s.push(char::from(DIGITS[usize::from(b & 0x0F)]));
    }
    s
}

fn nibble(c: u8) -> Result<u8, CryptoError> {
    match c {
        b'0'..=b'9' => Ok(c - b'0'),
        b'a'..=b'f' => Ok(c - b'a' + 10),
        b'A'..=b'F' => Ok(c - b'A' + 10),
        _ => Err(CryptoError::InvalidHex),
    }
}

pub fn decode(s: &str) -> Result<Vec<u8>, CryptoError> {
    let b = s.trim().as_bytes();
    if !b.len().is_multiple_of(2) {
        return Err(CryptoError::InvalidHex);
    }
    b.chunks(2)
        .map(|p| match p {
            [h, l] => Ok((nibble(*h)? << 4) | nibble(*l)?),
            _ => Err(CryptoError::InvalidHex),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn roundtrip(v in proptest::collection::vec(any::<u8>(), 0..64)) {
            prop_assert_eq!(decode(&encode(&v)).unwrap(), v);
        }
    }
}
