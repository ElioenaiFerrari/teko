use thiserror::Error;

#[derive(Debug, Error)]
pub enum BoardError {
    #[error("entry index out of range")]
    OutOfRange,
    #[error("old size is larger than the current size")]
    BadSize,
    #[error("payload too large (max 1 MiB)")]
    TooLarge,
    #[error("log file is corrupt or truncated: {0}")]
    Corrupt(&'static str),
    #[error("i/o: {0}")]
    Io(#[from] std::io::Error),
}
