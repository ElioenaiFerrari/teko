//! Network profiles: `devnet` (your machine), `testnet` (several machines, fake
//! data), `mainnet` (LOCKED). The `chain_id` is bound into every block, so
//! nothing signed on one network is valid on another.

use teko_crypto::hex;
use teko_election::GovernancePolicy;

use crate::error::ChainError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkKind {
    Devnet,
    Testnet,
    Mainnet,
}

impl NetworkKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Devnet => "devnet",
            Self::Testnet => "testnet",
            Self::Mainnet => "mainnet",
        }
    }
}

/// Public parameters of a network. Contains NO secrets.
///
/// Text format (`network.txt`, one `key=value` per line, `#` comments):
/// ```text
/// network=devnet
/// chain_id=teko-devnet-1
/// block_interval_ms=2000
/// validator_threshold=2
/// validator=<hex SEC1 pubkey>   (N lines)
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkProfile {
    pub kind: NetworkKind,
    pub chain_id: String,
    pub validators: Vec<Vec<u8>>,
    pub threshold: usize,
    pub block_interval_ms: u64,
}

impl NetworkProfile {
    /// Builds a profile; `Mainnet` is refused.
    pub fn new(
        kind: NetworkKind,
        validators: Vec<Vec<u8>>,
        threshold: usize,
    ) -> Result<Self, ChainError> {
        let (chain_id, interval) = match kind {
            NetworkKind::Devnet => ("teko-devnet-1", 2_000),
            NetworkKind::Testnet => ("teko-testnet-1", 6_000),
            NetworkKind::Mainnet => return Err(ChainError::MainnetLocked),
        };
        Self::checked(kind, chain_id.to_string(), validators, threshold, interval)
    }

    fn checked(
        kind: NetworkKind,
        chain_id: String,
        validators: Vec<Vec<u8>>,
        threshold: usize,
        block_interval_ms: u64,
    ) -> Result<Self, ChainError> {
        if chain_id.is_empty() || chain_id.len() > 64 || !chain_id.is_ascii() {
            return Err(ChainError::Profile("chain_id must be 1..=64 ASCII chars"));
        }
        if validators.len() < 2 {
            return Err(ChainError::Profile("at least 2 validators"));
        }
        // 1 <= M <= N, valid and distinct keys (reuses the governance checks).
        GovernancePolicy::new(validators.clone(), threshold)
            .map_err(|_| ChainError::Profile("invalid validator set or threshold"))?;
        if block_interval_ms < 20 {
            return Err(ChainError::Profile("block_interval_ms must be >= 20"));
        }
        Ok(Self {
            kind,
            chain_id,
            validators,
            threshold,
            block_interval_ms,
        })
    }

    /// Same network with another block interval (tests and tuning).
    pub fn with_block_interval_ms(mut self, ms: u64) -> Result<Self, ChainError> {
        if ms < 20 {
            return Err(ChainError::Profile("block_interval_ms must be >= 20"));
        }
        self.block_interval_ms = ms;
        Ok(self)
    }

    pub fn policy(&self) -> Result<GovernancePolicy, ChainError> {
        GovernancePolicy::new(self.validators.clone(), self.threshold)
            .map_err(|_| ChainError::Profile("invalid validator set or threshold"))
    }

    pub fn parse(text: &str) -> Result<Self, ChainError> {
        let (mut kind, mut chain_id, mut interval, mut threshold) = (None, None, None, None);
        let mut validators = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (k, v) = line
                .split_once('=')
                .ok_or(ChainError::Profile("expected key=value"))?;
            let v = v.trim();
            match k.trim() {
                "network" => {
                    kind = Some(match v {
                        "devnet" => NetworkKind::Devnet,
                        "testnet" => NetworkKind::Testnet,
                        "mainnet" => return Err(ChainError::MainnetLocked),
                        _ => return Err(ChainError::Profile("unknown network")),
                    })
                }
                "chain_id" => chain_id = Some(v.to_string()),
                "block_interval_ms" => {
                    interval = Some(
                        v.parse::<u64>()
                            .map_err(|_| ChainError::Profile("bad block_interval_ms"))?,
                    )
                }
                "validator_threshold" => {
                    threshold = Some(
                        v.parse::<usize>()
                            .map_err(|_| ChainError::Profile("bad validator_threshold"))?,
                    )
                }
                "validator" => validators
                    .push(hex::decode(v).map_err(|_| ChainError::Profile("bad validator key"))?),
                _ => return Err(ChainError::Profile("unknown key")),
            }
        }
        Self::checked(
            kind.ok_or(ChainError::Profile("missing network"))?,
            chain_id.ok_or(ChainError::Profile("missing chain_id"))?,
            validators,
            threshold.ok_or(ChainError::Profile("missing validator_threshold"))?,
            interval.ok_or(ChainError::Profile("missing block_interval_ms"))?,
        )
    }

    pub fn to_text(&self) -> String {
        let mut s = format!(
            "# Teko network profile (public). Test keys only.\nnetwork={}\nchain_id={}\nblock_interval_ms={}\nvalidator_threshold={}\n",
            self.kind.name(),
            self.chain_id,
            self.block_interval_ms,
            self.threshold
        );
        for v in &self.validators {
            s.push_str(&format!("validator={}\n", hex::encode(v)));
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand::rngs::StdRng;
    use teko_crypto::{generate_signing_key, public_key_bytes};

    fn keys(n: usize) -> Vec<Vec<u8>> {
        let mut rng = StdRng::seed_from_u64(1);
        (0..n)
            .map(|_| public_key_bytes(&generate_signing_key(&mut rng)))
            .collect()
    }

    #[test]
    fn mainnet_is_locked_everywhere() {
        assert_eq!(
            NetworkProfile::new(NetworkKind::Mainnet, keys(3), 2),
            Err(ChainError::MainnetLocked)
        );
        let t = "network=mainnet\nchain_id=teko-main-1\nblock_interval_ms=2000\nvalidator_threshold=1\n";
        assert_eq!(NetworkProfile::parse(t), Err(ChainError::MainnetLocked));
    }

    #[test]
    fn text_roundtrip_and_validation() {
        let p = NetworkProfile::new(NetworkKind::Devnet, keys(3), 2).unwrap();
        assert_eq!(p.chain_id, "teko-devnet-1");
        assert_eq!(NetworkProfile::parse(&p.to_text()).unwrap(), p);
        assert_ne!(
            p.chain_id,
            NetworkProfile::new(NetworkKind::Testnet, keys(3), 2)
                .unwrap()
                .chain_id
        );
        assert!(NetworkProfile::new(NetworkKind::Devnet, keys(3), 0).is_err());
        assert!(NetworkProfile::new(NetworkKind::Devnet, keys(3), 4).is_err());
        assert!(NetworkProfile::new(NetworkKind::Devnet, keys(1), 1).is_err());
        let mut dup = keys(2);
        dup.push(dup[0].clone());
        assert!(NetworkProfile::new(NetworkKind::Devnet, dup, 2).is_err());
        assert!(NetworkProfile::parse("network=devnet\nbogus=1\n").is_err());
    }
}
