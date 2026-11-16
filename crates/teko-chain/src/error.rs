use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ChainError {
    #[error("the mainnet profile is locked in this phase")]
    MainnetLocked,
    #[error("invalid network profile: {0}")]
    Profile(&'static str),
    #[error("block rejected: {0}")]
    Block(&'static str),
    #[error("the commit certificate is missing or invalid")]
    Commit,
    #[error("the block time does not move forward (it must be after the previous block's)")]
    BadTime,
    #[error("log error: {0}")]
    Log(String),
}
