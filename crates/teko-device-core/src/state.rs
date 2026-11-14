use alloc::vec::Vec;

use prost::Message;
use teko_protocol::DeviceState;

/// Everything the device persists. SECRET: never leaves the device and is
/// deliberately not part of any protocol message.
#[derive(Clone, PartialEq, Message)]
pub struct Persisted {
    #[prost(enumeration = "DeviceState", tag = "1")]
    pub state: i32,
    /// Installed firmware version; doubles as the monotonic anti-rollback
    /// counter (survives wipe).
    #[prost(uint32, tag = "2")]
    pub fw_version: u32,
    #[prost(bytes = "vec", tag = "3")]
    pub pin_salt: Vec<u8>,
    #[prost(bytes = "vec", tag = "4")]
    pub pin_hash: Vec<u8>,
    #[prost(uint32, tag = "5")]
    pub pin_fail_count: u32,
    #[prost(uint64, tag = "6")]
    pub last_fail_ms: u64,
    #[prost(bytes = "vec", tag = "7")]
    pub k_root: Vec<u8>,
    /// Voter secret `s`: separate entropy, NOT derived from `k_root`.
    #[prost(bytes = "vec", tag = "8")]
    pub voter_secret: Vec<u8>,
    /// Factory attestation material (batch key + cert); survives wipe.
    #[prost(bytes = "vec", tag = "9")]
    pub batch_key: Vec<u8>,
    #[prost(bytes = "vec", tag = "10")]
    pub batch_cert: Vec<u8>,
    /// Encoded `IssuedCredential`.
    #[prost(bytes = "vec", tag = "11")]
    pub credential: Vec<u8>,
}
