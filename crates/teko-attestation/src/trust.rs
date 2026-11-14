use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

use teko_crypto::hex;
use thiserror::Error;

/// Public trust anchors. Contains NO secrets.
///
/// Text format (one `key=value` per line, `#` comments):
/// ```text
/// authority_root=<hex SEC1 pubkey>
/// fw_threshold=2
/// fw_signer=<hex>   (N lines)
/// revoked_batch=<id>
/// gov_threshold=2   (optional: election governance, Phase 2)
/// gov_signer=<hex> (N lines)
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TrustPolicy {
    /// Authority root: signs batch certificates and citizen credentials.
    pub authority_root: Vec<u8>,
    pub fw_signers: Vec<Vec<u8>>,
    pub fw_threshold: usize,
    pub revoked_batches: Vec<String>,
    /// Institutions that govern ELECTIONS (Phase 2). Empty = elections disabled.
    pub gov_signers: Vec<Vec<u8>>,
    pub gov_threshold: usize,
    /// Network this device accepts elections for (`chain_id=`). Empty = any (dev).
    pub chain_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TrustError {
    #[error("invalid trust file: {0}")]
    Invalid(&'static str),
}

impl TrustPolicy {
    pub fn parse(text: &str) -> Result<Self, TrustError> {
        let mut p = TrustPolicy::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (k, v) = line
                .split_once('=')
                .ok_or(TrustError::Invalid("expected key=value"))?;
            let v = v.trim();
            match k.trim() {
                "authority_root" => {
                    p.authority_root =
                        hex::decode(v).map_err(|_| TrustError::Invalid("bad authority_root"))?
                }
                "fw_threshold" => {
                    p.fw_threshold = v
                        .parse()
                        .map_err(|_| TrustError::Invalid("bad fw_threshold"))?
                }
                "fw_signer" => p
                    .fw_signers
                    .push(hex::decode(v).map_err(|_| TrustError::Invalid("bad fw_signer"))?),
                "revoked_batch" => p.revoked_batches.push(String::from(v)),
                "chain_id" => {
                    if v.is_empty() || v.len() > 64 || !v.is_ascii() {
                        return Err(TrustError::Invalid("bad chain_id"));
                    }
                    p.chain_id = String::from(v);
                }
                "gov_threshold" => {
                    p.gov_threshold = v
                        .parse()
                        .map_err(|_| TrustError::Invalid("bad gov_threshold"))?
                }
                "gov_signer" => p
                    .gov_signers
                    .push(hex::decode(v).map_err(|_| TrustError::Invalid("bad gov_signer"))?),
                _ => return Err(TrustError::Invalid("unknown key")),
            }
        }
        if p.authority_root.is_empty() {
            return Err(TrustError::Invalid("missing authority_root"));
        }
        if p.fw_threshold == 0 || p.fw_threshold > p.fw_signers.len() {
            return Err(TrustError::Invalid("fw_threshold must be in 1..=N"));
        }
        if p.gov_signers.is_empty() != (p.gov_threshold == 0)
            || p.gov_threshold > p.gov_signers.len()
        {
            return Err(TrustError::Invalid(
                "gov_threshold must be in 1..=N when governance is set",
            ));
        }
        Ok(p)
    }

    pub fn to_text(&self) -> String {
        let mut s = String::new();
        let _ = writeln!(s, "# Teko trust anchors (public). Test keys only.");
        let _ = writeln!(s, "authority_root={}", hex::encode(&self.authority_root));
        let _ = writeln!(s, "fw_threshold={}", self.fw_threshold);
        for k in &self.fw_signers {
            let _ = writeln!(s, "fw_signer={}", hex::encode(k));
        }
        for b in &self.revoked_batches {
            let _ = writeln!(s, "revoked_batch={b}");
        }
        if !self.gov_signers.is_empty() {
            let _ = writeln!(s, "gov_threshold={}", self.gov_threshold);
            for k in &self.gov_signers {
                let _ = writeln!(s, "gov_signer={}", hex::encode(k));
            }
        }
        if !self.chain_id.is_empty() {
            let _ = writeln!(s, "chain_id={}", self.chain_id);
        }
        s
    }

    pub fn is_revoked(&self, batch_id: &str) -> bool {
        self.revoked_batches.iter().any(|b| b == batch_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_roundtrip() {
        let p = TrustPolicy {
            authority_root: alloc::vec![2; 33],
            fw_signers: alloc::vec![alloc::vec![2; 33], alloc::vec![3; 33], alloc::vec![4; 33]],
            fw_threshold: 2,
            revoked_batches: alloc::vec![String::from("batch-x")],
            gov_signers: alloc::vec![alloc::vec![5; 33], alloc::vec![6; 33]],
            gov_threshold: 2,
            chain_id: String::from("teko-devnet-1"),
        };
        assert_eq!(TrustPolicy::parse(&p.to_text()).unwrap(), p);
    }

    #[test]
    fn governance_is_all_or_nothing() {
        let base = "authority_root=02\nfw_threshold=1\nfw_signer=02\n";
        assert!(
            TrustPolicy::parse(base).is_ok(),
            "elections may be disabled"
        );
        assert!(
            TrustPolicy::parse(&alloc::format!("{base}gov_signer=03\n")).is_err(),
            "signers without threshold"
        );
        assert!(
            TrustPolicy::parse(&alloc::format!("{base}gov_threshold=1\n")).is_err(),
            "threshold without signers"
        );
        assert!(
            TrustPolicy::parse(&alloc::format!("{base}gov_threshold=2\ngov_signer=03\n")).is_err()
        );
        assert!(
            TrustPolicy::parse(&alloc::format!("{base}gov_threshold=1\ngov_signer=03\n")).is_ok()
        );
    }

    #[test]
    fn rejects_bad_threshold() {
        let t = "authority_root=02\nfw_threshold=3\nfw_signer=02\n";
        assert!(TrustPolicy::parse(t).is_err());
    }
}
