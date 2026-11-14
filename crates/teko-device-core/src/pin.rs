use subtle::ConstantTimeEq;
use teko_crypto::{tagged_hash, tags};
use zeroize::Zeroizing;

use crate::config::DeviceConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinPolicyError {
    Length,
    NotDigits,
    Trivial,
}

/// Iterated salted hash. Placeholder for a hardware-backed KDF; the real
/// protection is the attempt counter + wipe, not this stretching.
pub fn derive_pin_hash(pin: &[u8], salt: &[u8], iterations: u32) -> Zeroizing<[u8; 32]> {
    let mut h = Zeroizing::new(tagged_hash(tags::PIN, &[salt, pin]));
    for _ in 1..iterations.max(1) {
        h = Zeroizing::new(tagged_hash(tags::PIN, &[salt, h.as_slice()]));
    }
    h
}

/// Constant-time comparison (never `==`).
pub fn pin_matches(pin: &[u8], salt: &[u8], expected: &[u8], iterations: u32) -> bool {
    let got = derive_pin_hash(pin, salt, iterations);
    got.as_slice().ct_eq(expected).into()
}

pub fn validate_new_pin(pin: &[u8], cfg: &DeviceConfig) -> Result<(), PinPolicyError> {
    if pin.len() < cfg.pin_min_len || pin.len() > cfg.pin_max_len {
        return Err(PinPolicyError::Length);
    }
    if !pin.iter().all(u8::is_ascii_digit) {
        return Err(PinPolicyError::NotDigits);
    }
    let same = pin.windows(2).all(|w| w[0] == w[1]);
    let asc = pin.windows(2).all(|w| w[1] == w[0].wrapping_add(1));
    let desc = pin.windows(2).all(|w| w[0] == w[1].wrapping_add(1));
    if same || asc || desc {
        return Err(PinPolicyError::Trivial);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy() {
        let c = DeviceConfig::default();
        assert!(validate_new_pin(b"482913", &c).is_ok());
        assert_eq!(
            validate_new_pin(b"123456", &c),
            Err(PinPolicyError::Trivial)
        );
        assert_eq!(
            validate_new_pin(b"654321", &c),
            Err(PinPolicyError::Trivial)
        );
        assert_eq!(
            validate_new_pin(b"111111", &c),
            Err(PinPolicyError::Trivial)
        );
        assert_eq!(validate_new_pin(b"12345", &c), Err(PinPolicyError::Length));
        assert_eq!(
            validate_new_pin(b"1234567890", &c),
            Err(PinPolicyError::Length)
        );
        assert_eq!(
            validate_new_pin(b"48291a", &c),
            Err(PinPolicyError::NotDigits)
        );
    }

    #[test]
    fn hash_matches_only_correct_pin() {
        let h = derive_pin_hash(b"482913", b"salt", 10);
        assert!(pin_matches(b"482913", b"salt", h.as_slice(), 10));
        assert!(!pin_matches(b"482914", b"salt", h.as_slice(), 10));
        assert!(!pin_matches(b"482913", b"other", h.as_slice(), 10));
    }
}
