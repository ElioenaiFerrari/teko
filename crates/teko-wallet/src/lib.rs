//! Wallet seed for Teko (ADR-0023): BIP-39 24-word phrase generated ON THE DEVICE,
//! SLIP-10 `nist256p1` key derivation, bech32m wallet addresses.
//!
//! The wallet is INDEPENDENT of the identity and voter roots: it has its own entropy,
//! so the phrase can never reveal who you are or how you voted (principle 5), and the
//! identity keeps its "no seed, in-person reissue" recovery (ADR-0003).
//!
//! The phrase leaves the device only through its SCREEN, to the human. This crate
//! keeps secrets in `Zeroizing` buffers and gives them redacted `Debug`.
//!
//! The derived keys are P-256 (like the rest of Teko), NOT secp256k1: the same words
//! in a Bitcoin wallet produce unrelated keys. NOT AUDITED.
#![cfg_attr(not(test), no_std)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

extern crate alloc;

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

use bip39::{Language, Mnemonic};
use bitcoin_hashes::{Hash, HashEngine, Hmac, HmacEngine, sha512};
use p256::elliptic_curve::PrimeField;
use p256::{FieldBytes, Scalar, ecdsa::SigningKey};
use teko_crypto::{ledger_address, public_key_bytes, wallet_address};
use zeroize::{Zeroize, Zeroizing};

pub const WORD_COUNT: usize = 24;
pub const ENTROPY_LEN: usize = 32;
/// Longest accepted passphrase (printable ASCII only, so no Unicode normalisation).
pub const MAX_PASSPHRASE: usize = 64;
/// Teko wallet purpose for the all-hardened path `m/7240'/0'/0'/0'/index'`.
pub const PURPOSE: u32 = 7240;
const HARDENED: u32 = 0x8000_0000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalletError {
    InvalidEntropy,
    /// Wrong word count, a word outside the list, or a bad checksum.
    InvalidPhrase,
    InvalidPassphrase,
    Derivation,
}

/// A 24-word recovery phrase. Never `Display`ed or logged; `Debug` is redacted.
pub struct Phrase {
    words: Zeroizing<Vec<String>>,
}

impl fmt::Debug for Phrase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Phrase(<redacted>)")
    }
}

fn mnemonic_of(words: &[String]) -> Result<Mnemonic, WalletError> {
    let joined = Zeroizing::new(words.join(" "));
    Mnemonic::parse_in_normalized(Language::English, joined.as_str())
        .map_err(|_| WalletError::InvalidPhrase)
}

impl Phrase {
    /// Builds the phrase of 32 bytes of device entropy.
    pub fn from_entropy(entropy: &[u8; ENTROPY_LEN]) -> Result<Phrase, WalletError> {
        let m = Mnemonic::from_entropy_in(Language::English, entropy)
            .map_err(|_| WalletError::InvalidEntropy)?;
        let words: Vec<String> = m.words().map(ToString::to_string).collect();
        if words.len() != WORD_COUNT {
            return Err(WalletError::InvalidEntropy);
        }
        Ok(Phrase {
            words: Zeroizing::new(words),
        })
    }

    /// Parses typed words: exactly 24, all in the BIP-39 English list, valid checksum.
    /// Case and surrounding spaces are forgiven.
    pub fn parse<S: AsRef<str>>(words: &[S]) -> Result<Phrase, WalletError> {
        if words.len() != WORD_COUNT {
            return Err(WalletError::InvalidPhrase);
        }
        let norm: Vec<String> = words.iter().map(|w| normalize_word(w.as_ref())).collect();
        mnemonic_of(&norm)?;
        Ok(Phrase {
            words: Zeroizing::new(norm),
        })
    }

    pub fn len(&self) -> usize {
        self.words.len()
    }

    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// The word at 0-based `index`.
    pub fn word(&self, index: usize) -> Option<&str> {
        self.words.get(index).map(String::as_str)
    }

    pub fn words(&self) -> &[String] {
        &self.words
    }

    /// Does `typed` equal the word at `index`? (Comparison is constant-time-ish: it
    /// always scans the full length; the phrase is not a login secret.)
    pub fn matches(&self, index: usize, typed: &str) -> bool {
        let t = normalize_word(typed);
        match self.word(index) {
            Some(w) => {
                w.len() == t.len()
                    && w.bytes().zip(t.bytes()).fold(0u8, |a, (x, y)| a | (x ^ y)) == 0
            }
            None => false,
        }
    }

    /// Wallet master key for this phrase and optional passphrase (BIP-39 seed, then
    /// SLIP-10 master for `nist256p1`).
    pub fn master_key(&self, passphrase: &str) -> Result<MasterKey, WalletError> {
        check_passphrase(passphrase)?;
        let m = mnemonic_of(&self.words)?;
        let seed = Zeroizing::new(m.to_seed_normalized(passphrase));
        MasterKey::from_seed(seed.as_slice())
    }
}

fn normalize_word(w: &str) -> String {
    w.trim().to_ascii_lowercase()
}

fn check_passphrase(p: &str) -> Result<(), WalletError> {
    if p.len() > MAX_PASSPHRASE || !p.bytes().all(|b| (0x20..=0x7e).contains(&b)) {
        return Err(WalletError::InvalidPassphrase);
    }
    Ok(())
}

/// Is `word` in the BIP-39 English list?
pub fn is_word(word: &str) -> bool {
    Language::English.find_word(&normalize_word(word)).is_some()
}

/// Words that start with `prefix` (for on-device autocomplete).
pub fn complete(prefix: &str) -> Vec<&'static str> {
    let p = normalize_word(prefix);
    Language::English
        .word_list()
        .iter()
        .copied()
        .filter(|w| w.starts_with(p.as_str()))
        .collect()
}

/// SLIP-10 extended private key: 32-byte key + 32-byte chain code.
pub struct MasterKey {
    bytes: Zeroizing<[u8; 64]>,
}

impl fmt::Debug for MasterKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MasterKey(<redacted>)")
    }
}

fn hmac512(key: &[u8], data: &[&[u8]]) -> Zeroizing<[u8; 64]> {
    let mut e = HmacEngine::<sha512::Hash>::new(key);
    for d in data {
        e.input(d);
    }
    Zeroizing::new(Hmac::from_engine(e).to_byte_array())
}

fn parse_scalar(b: &[u8]) -> Option<Scalar> {
    let arr: [u8; 32] = b.try_into().ok()?;
    let fb = FieldBytes::from(arr);
    Option::<Scalar>::from(Scalar::from_repr(fb))
}

fn is_zero(s: &Scalar) -> bool {
    use p256::elliptic_curve::Field;
    bool::from(s.is_zero())
}

impl MasterKey {
    /// SLIP-10 master key generation for `nist256p1`.
    pub fn from_seed(seed: &[u8]) -> Result<MasterKey, WalletError> {
        let mut i = hmac512(b"Nist256p1 seed", &[seed]);
        for _ in 0..64 {
            if let Some(k) = i.get(..32).and_then(parse_scalar)
                && !is_zero(&k)
            {
                return Ok(MasterKey { bytes: i });
            }
            i = hmac512(b"Nist256p1 seed", &[i.as_slice()]);
        }
        Err(WalletError::Derivation)
    }

    pub fn from_bytes(b: &[u8]) -> Option<MasterKey> {
        let a: [u8; 64] = b.try_into().ok()?;
        let k = parse_scalar(&a[..32])?;
        if is_zero(&k) {
            return None;
        }
        Some(MasterKey {
            bytes: Zeroizing::new(a),
        })
    }

    /// The 64 bytes to persist on the device (key ‖ chain code).
    pub fn as_bytes(&self) -> &[u8; 64] {
        &self.bytes
    }

    /// SLIP-10 hardened child. Returns the 64-byte extended key.
    fn child(&self, index: u32) -> Result<MasterKey, WalletError> {
        let idx = (index | HARDENED).to_be_bytes();
        let (k_par, c_par) = self.bytes.split_at(32);
        let parent = parse_scalar(k_par).ok_or(WalletError::Derivation)?;
        let mut i = hmac512(c_par, &[&[0u8], k_par, &idx]);
        for _ in 0..64 {
            if let Some(il) = i.get(..32).and_then(parse_scalar) {
                let k = il + parent;
                if !is_zero(&k) {
                    let mut out = Zeroizing::new([0u8; 64]);
                    out[..32].copy_from_slice(k.to_repr().as_slice());
                    out[32..].copy_from_slice(&i[32..]);
                    return Ok(MasterKey { bytes: out });
                }
            }
            let ir: Zeroizing<[u8; 32]> = Zeroizing::new(
                <[u8; 32]>::try_from(&i[32..]).map_err(|_| WalletError::Derivation)?,
            );
            i = hmac512(c_par, &[&[1u8], ir.as_slice(), &idx]);
        }
        Err(WalletError::Derivation)
    }

    /// The signing key at `m/7240'/0'/0'/0'/index'`.
    pub fn signing_key(&self, index: u32) -> Result<SigningKey, WalletError> {
        let mut k = self.child(PURPOSE)?;
        for part in [0u32, 0, 0, index] {
            k = k.child(part)?;
        }
        SigningKey::from_slice(&k.bytes[..32]).map_err(|_| WalletError::Derivation)
    }

    /// Public key (SEC1) of address `index`.
    pub fn public_key(&self, index: u32) -> Result<Vec<u8>, WalletError> {
        Ok(public_key_bytes(&self.signing_key(index)?))
    }

    /// 32-byte ledger address of `index`.
    pub fn address(&self, index: u32) -> Result<[u8; 32], WalletError> {
        Ok(ledger_address(&self.public_key(index)?))
    }

    /// bech32m `tekow1…` string of address `index`.
    pub fn address_string(&self, index: u32) -> Result<String, WalletError> {
        wallet_address(&self.address(index)?).map_err(|_| WalletError::Derivation)
    }

    #[cfg(test)]
    fn child_for_test(&self, index: u32) -> Result<MasterKey, WalletError> {
        self.child(index)
    }
}

impl Drop for MasterKey {
    fn drop(&mut self) {
        self.bytes.zeroize();
    }
}

/// SLIP-10 `nist256p1` derivation along `path` (hardened indices), for testing
/// against the published vectors.
#[cfg(test)]
fn slip10(seed: &[u8], path: &[u32]) -> MasterKey {
    let mut k = MasterKey::from_seed(seed).unwrap();
    for p in path {
        k = k.child_for_test(*p).unwrap();
    }
    k
}

#[cfg(test)]
mod tests;
