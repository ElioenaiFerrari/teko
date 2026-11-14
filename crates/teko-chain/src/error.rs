use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ChainError {
    #[error("the mainnet profile is locked in this phase")]
    MainnetLocked,
    #[error("invalid network profile: {0}")]
    Profile(&'static str),
    #[error("block rejected: {0}")]
    Block(&'static str),
    #[error("not enough valid validator signatures ({got} of {need})")]
    Signatures { got: usize, need: usize },
    #[error("log error: {0}")]
    Log(String),
}
