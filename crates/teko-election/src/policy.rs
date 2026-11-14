use alloc::vec::Vec;

use teko_crypto::verifying_key_from_bytes;

use crate::error::ElectionError;

/// The N institutions that govern elections and the threshold M.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GovernancePolicy {
    signers: Vec<Vec<u8>>,
    threshold: usize,
}

impl GovernancePolicy {
    /// `1 <= threshold <= signers.len()`, all keys valid and distinct.
    pub fn new(signers: Vec<Vec<u8>>, threshold: usize) -> Result<Self, ElectionError> {
        if threshold == 0 || threshold > signers.len() {
            return Err(ElectionError::InvalidPolicy("threshold must be in 1..=N"));
        }
        for (i, k) in signers.iter().enumerate() {
            if verifying_key_from_bytes(k).is_err() {
                return Err(ElectionError::InvalidPolicy("invalid signer key"));
            }
            if signers.iter().skip(i + 1).any(|o| o == k) {
                return Err(ElectionError::InvalidPolicy("duplicate signer"));
            }
        }
        Ok(Self { signers, threshold })
    }

    pub fn signers(&self) -> &[Vec<u8>] {
        &self.signers
    }

    pub fn threshold(&self) -> usize {
        self.threshold
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand::rngs::StdRng;
    use teko_crypto::{generate_signing_key, public_key_bytes};

    fn keys(n: usize) -> Vec<Vec<u8>> {
        let mut rng = StdRng::seed_from_u64(5);
        (0..n)
            .map(|_| public_key_bytes(&generate_signing_key(&mut rng)))
            .collect()
    }

    #[test]
    fn validates_threshold_keys_and_duplicates() {
        let k = keys(3);
        assert!(GovernancePolicy::new(k.clone(), 2).is_ok());
        assert!(GovernancePolicy::new(k.clone(), 0).is_err());
        assert!(GovernancePolicy::new(k.clone(), 4).is_err());
        assert!(GovernancePolicy::new(vec![vec![1, 2, 3]], 1).is_err());
        assert!(GovernancePolicy::new(vec![k[0].clone(), k[0].clone()], 1).is_err());
    }
}
