use thiserror::Error;

/// Errors from framing and message decoding. Decoding never panics.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProtocolError {
    #[error("frame too short")]
    Truncated,
    #[error("bad magic")]
    BadMagic,
    #[error("unsupported protocol version {0}")]
    UnsupportedVersion(u8),
    #[error("reserved flag bits set")]
    ReservedFlags,
    #[error("payload length {0} exceeds limit")]
    PayloadTooLarge(u32),
    #[error("frame length does not match header")]
    LengthMismatch,
    #[error("crc mismatch")]
    BadCrc,
    #[error("unknown message type {0:#06x}")]
    UnknownMessage(u16),
    #[error("malformed payload")]
    MalformedPayload,
}
