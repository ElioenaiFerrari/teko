use thiserror::Error;

#[derive(Debug, Error)]
pub enum NodeError {
    #[error("i/o: {0}")]
    Io(#[from] std::io::Error),
    #[error("chain: {0}")]
    Chain(#[from] teko_chain::ChainError),
    #[error("protocol: {0}")]
    Protocol(String),
    #[error("refused: {0}")]
    Refused(String),
    #[error("no node reachable")]
    Unreachable,
}
