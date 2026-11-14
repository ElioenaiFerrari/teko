//! RSABSSA-SHA384-PSS-Randomized (RFC 9474), step by step.

use crypto_bigint::{BoxedUint, Odd};
use rand::TryRng;
use rand::rngs::SysRng;
use rsa::hazmat::{rsa_decrypt_and_check, rsa_encrypt};
use rsa::pkcs8::{DecodePrivateKey, DecodePublicKey, EncodePrivateKey, EncodePublicKey};
use rsa::pss::{Signature, VerifyingKey};
use rsa::signature::Verifier;
use rsa::traits::PublicKeyParts;
use rsa::{PrivateKey, PublicKey};
use sha2::{Digest, Sha384};
use thiserror::Error;

const HASH_LEN: usize = 48;
const SALT_LEN: usize = 48;
pub const PREFIX_LEN: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RegError {
    #[error("invalid RSA key")]
    InvalidKey,
    #[error("invalid input (length, range or not coprime)")]
    InvalidInput,
    #[error("signature does not verify")]
    InvalidSignature,
    #[error("randomness source failed")]
    Rng,
    #[error("internal error")]
    Internal,
}

/// The authority's secret key for ONE contest (ADR-0017: one key per contest, so
/// a registration for President can never be replayed in a mayoral contest).
pub struct IssuerKey {
    sk: PrivateKey,
}

/// Output of `blind`: what is sent to the authority, and what the voter keeps.
pub struct BlindResult {
    /// Send to the authority.
    pub blind_message: Vec<u8>,
    /// KEEP SECRET until `finalize`: `msg_prefix ‖ r⁻¹`.
    pub state: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlindSignature(pub Vec<u8>);

/// What the voter shows to the registry: verifiable by anyone, unlinkable to the issuance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub msg_prefix: [u8; PREFIX_LEN],
    pub signature: Vec<u8>,
}

impl Token {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = self.msg_prefix.to_vec();
        v.extend_from_slice(&self.signature);
        v
    }

    pub fn from_bytes(b: &[u8]) -> Result<Self, RegError> {
        let (p, s) = b
            .split_at_checked(PREFIX_LEN)
            .ok_or(RegError::InvalidInput)?;
        Ok(Self {
            msg_prefix: p.try_into().map_err(|_| RegError::InvalidInput)?,
            signature: s.to_vec(),
        })
    }
}

pub type Blinded = BlindResult;

fn rng_fill(buf: &mut [u8]) -> Result<(), RegError> {
    SysRng.try_fill_bytes(buf).map_err(|_| RegError::Rng)
}

fn load_public(spki: &[u8]) -> Result<PublicKey, RegError> {
    PublicKey::from_public_key_der(spki).map_err(|_| RegError::InvalidKey)
}

/// `k`: modulus length in bytes. Integers are exchanged as exactly `k` big-endian bytes.
fn to_k_bytes(x: &BoxedUint, k: usize) -> Vec<u8> {
    let full = x.to_be_bytes();
    let start = full.len().saturating_sub(k);
    let mut out = vec![0u8; k.saturating_sub(full.len())];
    out.extend_from_slice(&full[start..]);
    out
}

fn from_bytes(pk: &PublicKey, b: &[u8]) -> Result<BoxedUint, RegError> {
    BoxedUint::from_be_slice(b, pk.n_bits_precision()).map_err(|_| RegError::InvalidInput)
}

// ───────────────────────── RFC 8017 EMSA-PSS ─────────────────────────

fn mgf1(seed: &[u8], len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len + HASH_LEN);
    let mut counter = 0u32;
    while out.len() < len {
        let mut h = Sha384::new();
        h.update(seed);
        h.update(counter.to_be_bytes());
        out.extend_from_slice(&h.finalize());
        counter += 1;
    }
    out.truncate(len);
    out
}

/// EMSA-PSS-ENCODE (RFC 8017 §9.1.1) with SHA-384, MGF1-SHA-384, `salt` of 48 bytes.
#[doc(hidden)]
pub fn emsa_pss_encode(
    message: &[u8],
    em_bits: usize,
    salt: &[u8; SALT_LEN],
) -> Result<Vec<u8>, RegError> {
    let em_len = em_bits.div_ceil(8);
    if em_len < HASH_LEN + SALT_LEN + 2 {
        return Err(RegError::InvalidInput);
    }
    let m_hash = Sha384::digest(message);
    let mut m_prime = vec![0u8; 8];
    m_prime.extend_from_slice(&m_hash);
    m_prime.extend_from_slice(salt);
    let h = Sha384::digest(&m_prime);
    let ps_len = em_len - SALT_LEN - HASH_LEN - 2;
    let mut db = vec![0u8; ps_len];
    db.push(0x01);
    db.extend_from_slice(salt);
    let mask = mgf1(&h, em_len - HASH_LEN - 1);
    for (d, m) in db.iter_mut().zip(mask) {
        *d ^= m;
    }
    // zero the leftmost 8*emLen - emBits bits
    let unused = 8 * em_len - em_bits;
    if unused > 0 {
        db[0] &= 0xFFu8 >> unused;
    }
    let mut em = db;
    em.extend_from_slice(&h);
    em.push(0xbc);
    Ok(em)
}

/// Test hook for the RFC vector (EMSA-PSS-ENCODE with an explicit salt).
#[doc(hidden)]
pub fn emsa_pss_encode_for_tests(
    message: &[u8],
    em_bits: usize,
    salt: &[u8; SALT_LEN],
) -> Result<Vec<u8>, RegError> {
    emsa_pss_encode(message, em_bits, salt)
}

// ───────────────────────── Blind / Finalize / Verify (client side) ─────────────────────────

/// RFC 9474 Blind with explicit randomness: `prefix`, PSS `salt`, and the blinding
/// inverse `inv = r⁻¹ mod n` (as in the RFC's test vector). Production code uses
/// `blind`, which draws all three from the OS RNG.
#[doc(hidden)]
pub fn blind_with(
    public_spki: &[u8],
    message: &[u8],
    prefix: [u8; PREFIX_LEN],
    salt: &[u8; SALT_LEN],
    inv: &[u8],
) -> Result<BlindResult, RegError> {
    let pk = load_public(public_spki)?;
    let k = pk.size();
    if inv.len() != k {
        return Err(RegError::InvalidInput);
    }
    let n = pk.n();
    let n_odd = Odd::new(n.as_ref().clone())
        .into_option()
        .ok_or(RegError::InvalidKey)?;

    let mut input = prefix.to_vec();
    input.extend_from_slice(message);
    let em = emsa_pss_encode(&input, n.bit_width() as usize - 1, salt)?;
    let m = from_bytes(&pk, &em)?;
    // The encoded message must be invertible mod n.
    if m.invert_odd_mod(&n_odd).into_option().is_none() {
        return Err(RegError::InvalidInput);
    }
    let r_inv = from_bytes(&pk, inv)?;
    // r = (r⁻¹)⁻¹ mod n
    let r = r_inv
        .invert_odd_mod(&n_odd)
        .into_option()
        .ok_or(RegError::InvalidInput)?;
    let x = rsa_encrypt(&pk, &r).map_err(|_| RegError::Internal)?;
    let z = m.mul_mod(&x, n);

    let mut state = prefix.to_vec();
    state.extend_from_slice(inv);
    Ok(BlindResult {
        blind_message: to_k_bytes(&z, k),
        state,
    })
}

/// Like `blind`, but every random value is derived from `seed` (SHA-384 in
/// counter mode), so the same `(key, message, seed)` always gives the same
/// result. `seed` must be secret and unique per registration (the device derives
/// it from its voter secret and the contest); anyone who knows it can link.
pub fn blind_deterministic(
    public_spki: &[u8],
    message: &[u8],
    seed: &[u8; 32],
) -> Result<BlindResult, RegError> {
    let pk = load_public(public_spki)?;
    let k = pk.size();
    let expand = |label: &[u8], counter: u32, len: usize| -> Vec<u8> {
        let mut out = Vec::with_capacity(len + HASH_LEN);
        let mut block = 0u32;
        while out.len() < len {
            let mut h = Sha384::new();
            h.update(b"teko/v2/blind-expand");
            h.update(label);
            h.update(seed);
            h.update(counter.to_be_bytes());
            h.update(block.to_be_bytes());
            out.extend_from_slice(&h.finalize());
            block += 1;
        }
        out.truncate(len);
        out
    };
    let mut prefix = [0u8; PREFIX_LEN];
    prefix.copy_from_slice(&expand(b"prefix", 0, PREFIX_LEN));
    let mut salt = [0u8; SALT_LEN];
    salt.copy_from_slice(&expand(b"salt", 0, SALT_LEN));
    let n_odd = Odd::new(pk.n().as_ref().clone())
        .into_option()
        .ok_or(RegError::InvalidKey)?;
    for counter in 0u32..64 {
        let mut buf = expand(b"r", counter, k);
        // Clearing the top byte keeps r < n for any modulus with its top byte set.
        if let Some(b) = buf.first_mut() {
            *b = 0;
        }
        let r = from_bytes(&pk, &buf)?;
        if let Some(r_inv) = r.invert_odd_mod(&n_odd).into_option()
            && let Ok(res) = blind_with(public_spki, message, prefix, &salt, &to_k_bytes(&r_inv, k))
        {
            return Ok(res);
        }
    }
    Err(RegError::Internal)
}

/// RFC 9474 Prepare + Blind with fresh randomness.
pub fn blind(public_spki: &[u8], message: &[u8]) -> Result<BlindResult, RegError> {
    let pk = load_public(public_spki)?;
    let k = pk.size();
    let n_odd = Odd::new(pk.n().as_ref().clone())
        .into_option()
        .ok_or(RegError::InvalidKey)?;
    let mut prefix = [0u8; PREFIX_LEN];
    rng_fill(&mut prefix)?;
    let mut salt = [0u8; SALT_LEN];
    rng_fill(&mut salt)?;
    // r uniform in [1, n) and invertible: rejection sampling.
    loop {
        let mut buf = vec![0u8; k];
        rng_fill(&mut buf)?;
        let r = from_bytes(&pk, &buf)?;
        if r >= *pk.n().as_ref() {
            continue;
        }
        if let Some(r_inv) = r.invert_odd_mod(&n_odd).into_option() {
            return blind_with(public_spki, message, prefix, &salt, &to_k_bytes(&r_inv, k));
        }
    }
}

/// RFC 9474 Finalize: unblind, then CHECK the signature (so a wrong or
/// malicious blind signature is rejected here, never handed out as a token).
pub fn finalize(
    public_spki: &[u8],
    state: &[u8],
    blind_sig: &BlindSignature,
    message: &[u8],
) -> Result<Token, RegError> {
    let pk = load_public(public_spki)?;
    let k = pk.size();
    if state.len() != PREFIX_LEN + k || blind_sig.0.len() != k {
        return Err(RegError::InvalidInput);
    }
    let (prefix, inv) = state.split_at(PREFIX_LEN);
    let z = from_bytes(&pk, &blind_sig.0)?;
    let r_inv = from_bytes(&pk, inv)?;
    let s = z.mul_mod(&r_inv, pk.n());
    let token = Token {
        msg_prefix: prefix.try_into().map_err(|_| RegError::InvalidInput)?,
        signature: to_k_bytes(&s, k),
    };
    verify(public_spki, message, &token)?;
    Ok(token)
}

/// RSASSA-PSS-VERIFY (RFC 8017) of `msg_prefix ‖ message`.
pub fn verify(public_spki: &[u8], message: &[u8], token: &Token) -> Result<(), RegError> {
    let pk = load_public(public_spki)?;
    if token.signature.len() != pk.size() {
        return Err(RegError::InvalidSignature);
    }
    let mut input = token.msg_prefix.to_vec();
    input.extend_from_slice(message);
    let sig =
        Signature::try_from(token.signature.as_slice()).map_err(|_| RegError::InvalidSignature)?;
    VerifyingKey::<Sha384>::new_with_salt_len(pk, SALT_LEN)
        .verify(&input, &sig)
        .map_err(|_| RegError::InvalidSignature)
}

// ───────────────────────── Authority side ─────────────────────────

impl IssuerKey {
    /// RSA key for ONE contest. 2048 bits is the minimum worth using.
    pub fn generate(bits: usize) -> Result<Self, RegError> {
        let sk = PrivateKey::new(&mut SysRng, bits).map_err(|_| RegError::InvalidKey)?;
        Ok(Self { sk })
    }

    #[doc(hidden)]
    pub fn from_rsa(sk: PrivateKey) -> Self {
        Self { sk }
    }

    pub fn from_pkcs8_der(der: &[u8]) -> Result<Self, RegError> {
        Ok(Self {
            sk: PrivateKey::from_pkcs8_der(der).map_err(|_| RegError::InvalidKey)?,
        })
    }

    /// SECRET.
    pub fn to_pkcs8_der(&self) -> Result<Vec<u8>, RegError> {
        self.sk
            .to_pkcs8_der()
            .map(|d| d.as_bytes().to_vec())
            .map_err(|_| RegError::InvalidKey)
    }

    /// Public key (SPKI DER): this is what the signed manifest carries.
    pub fn public_spki(&self) -> Result<Vec<u8>, RegError> {
        self.sk
            .to_public_key()
            .to_public_key_der()
            .map(|d| d.as_bytes().to_vec())
            .map_err(|_| RegError::InvalidKey)
    }

    /// RFC 9474 BlindSign. Uses RSA blinding and a CRT fault check
    /// (`rsa_decrypt_and_check`: the result is re-encrypted and compared).
    pub fn blind_sign(&self, blind_message: &[u8]) -> Result<BlindSignature, RegError> {
        let pk = self.sk.to_public_key();
        let k = pk.size();
        if blind_message.len() != k {
            return Err(RegError::InvalidInput);
        }
        let m = from_bytes(&pk, blind_message)?;
        if m >= *pk.n().as_ref() {
            return Err(RegError::InvalidInput);
        }
        let s = rsa_decrypt_and_check(&self.sk, Some(&mut SysRng), &m)
            .map_err(|_| RegError::Internal)?;
        Ok(BlindSignature(to_k_bytes(&s, k)))
    }
}
