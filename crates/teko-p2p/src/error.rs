use thiserror::Error;

#[derive(Debug, Error)]
pub enum NetError {
    #[error("i/o: {0}")]
    Io(#[from] std::io::Error),
    /// The handshake failed: bad message, bad proof, wrong chain or genesis, key not accepted.
    #[error("handshake refused: {0}")]
    Handshake(String),
    /// A frame did not decrypt (tampered, replayed, reordered).
    #[error("bad frame")]
    BadFrame,
    #[error("message too large")]
    TooLarge,
    #[error("timed out")]
    Timeout,
    #[error("refused: {0}")]
    Refused(String),
}
