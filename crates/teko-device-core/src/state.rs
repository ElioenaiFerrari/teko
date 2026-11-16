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
    /// Encoded `ElectionBundle` (manifest + governance transitions), re-verified on use.
    #[prost(bytes = "vec", tag = "12")]
    pub election: Vec<u8>,
    /// `contest_hash` of every contest this device already signed a ballot for.
    #[prost(bytes = "vec", repeated, tag = "13")]
    pub ballots_signed: Vec<Vec<u8>>,
    /// Wallet master key (SLIP-10 key ‖ chain code, 64 bytes). INDEPENDENT of `k_root`
    /// and `voter_secret`; the recovery phrase itself is never stored (ADR-0023).
    #[prost(bytes = "vec", tag = "14")]
    pub wallet_master: Vec<u8>,
    /// The wallet was derived with a BIP-39 passphrase (the passphrase is NOT stored).
    #[prost(bool, tag = "15")]
    pub wallet_passphrase: bool,
    /// What the device keeps about the ballots it cast, to check later that each is recorded
    /// (ADR-0041): ONLY public hashes. Never the option, never the randomness.
    #[prost(message, repeated, tag = "16")]
    pub vote_records: Vec<VoteRecord>,
    /// The highest epoch of the vote chain's institutions the device has verified, and the hash of
    /// that epoch's set (ADR-0045). A ratchet: a chain of changes older than this, or on another
    /// branch, is refused, so a retired supermajority cannot forge a different history.
    #[prost(uint64, tag = "17")]
    pub vote_epoch: u64,
    #[prost(bytes = "vec", tag = "18")]
    pub vote_set_hash: Vec<u8>,
}

/// A ballot this device cast. Nothing here reveals the vote: the leaf hash is of an entry the
/// board already shows (the ciphertext is public).
#[derive(Clone, PartialEq, Message)]
pub struct VoteRecord {
    #[prost(string, tag = "1")]
    pub election_id: alloc::string::String,
    #[prost(string, tag = "2")]
    pub contest_id: alloc::string::String,
    #[prost(bytes = "vec", tag = "3")]
    pub contest_hash: Vec<u8>,
    /// Merkle leaf hash of the `BALLOT` entry on the board.
    #[prost(bytes = "vec", tag = "4")]
    pub leaf_hash: Vec<u8>,
}
