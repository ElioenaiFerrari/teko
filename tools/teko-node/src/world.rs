//! The economy of a running node network, as the Suite sees it: a chain VERIFIED in
//! this process (every certificate checked against the validator set the replayed state
//! implies), refreshed from the nodes, with transactions submitted to them.

use std::path::Path;
use std::time::{Duration, Instant};

use prost::Message;
use teko_assets::devnet::World;
use teko_coin::devnet::{TransferRecord, history_from_txs};
use teko_economy::{EconBlock, EconomyChain, EconomyGenesis};
use teko_ledger::{Address, Ledger, Tx};

use crate::client::{EconStatus, NodeClient};
use crate::error::NodeError;
use crate::setup::{load_economy, load_network};

/// How long `submit` waits for a transaction to land in a block.
const INCLUSION_TIMEOUT: Duration = Duration::from_secs(30);

pub struct NodeWorld {
    client: NodeClient,
    genesis: EconomyGenesis,
    chain: EconomyChain,
}

impl NodeWorld {
    /// Connects to the network in `net_dir` and verifies its whole economy chain.
    /// Fails when the network has no economy or no node answers.
    pub fn open(net_dir: &Path) -> Result<Self, NodeError> {
        let net = load_network(net_dir)?;
        let ec = load_economy(net_dir)?
            .ok_or_else(|| NodeError::Economy("this network has no economy".into()))?;
        let client = NodeClient::new(net.peers);
        let chain = client.econ_sync(&ec.genesis)?;
        Ok(Self {
            client,
            genesis: ec.genesis,
            chain,
        })
    }

    pub fn genesis(&self) -> &EconomyGenesis {
        &self.genesis
    }

    /// The verified ledger (same as `World::ledger`, without importing the trait).
    pub fn ledger_ref(&self) -> &Ledger {
        self.chain.ledger()
    }

    pub fn chain(&self) -> &EconomyChain {
        &self.chain
    }

    /// Downloads and verifies the blocks decided since the last refresh.
    pub fn refresh(&mut self) -> Result<(), NodeError> {
        self.client.econ_pull_into(&mut self.chain)
    }

    /// Live status of every node (validator set, founder phase, stake).
    pub fn statuses(&self) -> Vec<Option<EconStatus>> {
        self.client
            .addrs()
            .iter()
            .map(|a| self.client.econ_status(*a).ok())
            .collect()
    }
}

impl World for NodeWorld {
    fn chain_id(&self) -> &str {
        &self.genesis.chain_id
    }

    fn ledger(&self) -> &Ledger {
        self.chain.ledger()
    }

    fn submit(&mut self, tx: Tx) -> Result<(), String> {
        let sender = tx.address();
        let nonce = tx.nonce;
        self.client
            .econ_submit(std::slice::from_ref(&tx))
            .map_err(|e| match e {
                NodeError::Refused(why) => describe(&why),
                other => other.to_string(),
            })?;
        let t = Instant::now();
        while t.elapsed() < INCLUSION_TIMEOUT {
            self.refresh().map_err(|e| e.to_string())?;
            if self.chain.ledger().nonce_of(&sender) > nonce {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(150));
        }
        Err("a transação foi aceita pela rede mas ainda não entrou em um bloco".into())
    }

    fn history(&self, who: &Address) -> Vec<TransferRecord> {
        let txs: Vec<(u64, Tx)> = self
            .chain
            .blocks()
            .iter()
            .filter_map(|d| {
                let b = EconBlock::decode(d.payload.as_slice()).ok()?;
                Some(
                    b.txs
                        .iter()
                        .filter_map(|raw| Tx::from_bytes(raw).ok().map(|t| (b.height, t)))
                        .collect::<Vec<_>>(),
                )
            })
            .flatten()
            .collect();
        history_from_txs(txs.iter().map(|(h, t)| (*h, t)), who)
    }
}

fn describe(why: &str) -> String {
    if why.contains("insufficient funds") {
        "saldo insuficiente".into()
    } else if why.contains("BadNonce") {
        "transação fora de ordem (nonce): atualize e tente de novo".into()
    } else if why.contains("FeeTooLow") {
        "taxa abaixo do mínimo".into()
    } else {
        why.to_string()
    }
}
