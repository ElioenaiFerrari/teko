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
    #[error("economy: {0}")]
    Economy(String),
    #[error("no node reachable")]
    Unreachable,
    #[error("network: {0}")]
    Net(#[from] teko_p2p::NetError),
}

impl NodeError {
    /// A block that was CERTIFIED by the validators fails to replay. Software that agrees
    /// with the network never does that, so it almost always means the saved chain was
    /// made by an older version whose rules differ (for example ADR-0032 changed the
    /// assets format): the test network must be recreated.
    pub fn replay(e: &teko_economy::EconError) -> NodeError {
        let hint = if matches!(e, teko_economy::EconError::BadTx(_)) {
            " — a cadeia gravada não é compatível com esta versão do software (o formato mudou, por exemplo ADR-0032). Recrie a economia de teste: `make devnet-stop`, `make devnet-founder FOUNDER=<seu tekow1…>` e `make devnet`"
        } else {
            ""
        };
        NodeError::Economy(format!("{e:?}{hint}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use teko_economy::EconError;

    #[test]
    fn a_certified_block_that_does_not_replay_points_to_the_recreate_steps() {
        let e = NodeError::replay(&EconError::BadTx("metadata hash must be 32 bytes".into()));
        let t = e.to_string();
        assert!(
            t.contains("make devnet-founder") && t.contains("ADR-0032"),
            "{t}"
        );
        // other errors stay plain
        let plain = NodeError::replay(&EconError::StateRoot).to_string();
        assert!(!plain.contains("make devnet-founder"), "{plain}");
    }
}
