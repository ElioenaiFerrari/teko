use alloc::string::String;

/// Tunable device policy. Defaults follow the approved recommendations:
/// 8 attempts, 6–9 digit PIN, exponential delay from the 4th failure,
/// 5-minute unlock timeout.
#[derive(Debug, Clone)]
pub struct DeviceConfig {
    pub label: String,
    /// Wrong PINs tolerated before the device wipes itself.
    pub max_pin_attempts: u32,
    /// Failures with no delay.
    pub free_attempts: u32,
    pub backoff_base_ms: u64,
    pub backoff_max_ms: u64,
    pub pin_min_len: usize,
    pub pin_max_len: usize,
    pub unlock_timeout_ms: u64,
    /// KDF iterations for the PIN hash (placeholder for a secure-element KDF).
    pub pin_kdf_iterations: u32,
}

impl Default for DeviceConfig {
    fn default() -> Self {
        Self {
            label: String::from("Teko Emulator"),
            max_pin_attempts: 8,
            free_attempts: 3,
            backoff_base_ms: 1_000,
            backoff_max_ms: 300_000,
            pin_min_len: 6,
            pin_max_len: 9,
            unlock_timeout_ms: 300_000,
            pin_kdf_iterations: 20_000,
        }
    }
}

impl DeviceConfig {
    /// Delay required after `fails` consecutive wrong PINs.
    pub fn backoff_ms(&self, fails: u32) -> u64 {
        if fails <= self.free_attempts {
            return 0;
        }
        let exp = (fails - self.free_attempts - 1).min(30);
        self.backoff_base_ms
            .saturating_mul(1u64 << exp)
            .min(self.backoff_max_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_is_exponential_and_capped() {
        let c = DeviceConfig::default();
        assert_eq!(c.backoff_ms(3), 0);
        assert_eq!(c.backoff_ms(4), 1_000);
        assert_eq!(c.backoff_ms(5), 2_000);
        assert_eq!(c.backoff_ms(6), 4_000);
        assert_eq!(c.backoff_ms(60), 300_000);
    }
}
