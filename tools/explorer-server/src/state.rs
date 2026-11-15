use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use teko_chain::NetworkProfile;
use teko_explorer::Index;

use crate::votes::{VotesSnapshot, snapshot};
use teko_node::NodeClient;

#[derive(Debug, Clone, Serialize)]
pub struct NodeHealth {
    pub addr: String,
    pub up: bool,
    pub height: Option<u64>,
    pub mempool: Option<u32>,
    pub is_validator: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Meta {
    pub last_sync_unix: u64,
    /// Last problem found while syncing (unreachable node, block that failed
    /// verification). `None` when the last sync was clean.
    pub last_error: Option<String>,
    pub nodes: Vec<NodeHealth>,
}

pub struct Shared {
    pub index: Mutex<Index>,
    pub meta: Mutex<Meta>,
    /// `devnet`, `testnet` or `mainnet`.
    pub network: String,
    /// Profile of the vote ledger (PoA), when this network has one to show.
    pub profile: Option<NetworkProfile>,
    pub votes: Mutex<VotesSnapshot>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl Shared {
    pub fn new(index: Index, network: &str) -> Self {
        Self {
            index: Mutex::new(index),
            meta: Mutex::new(Meta::default()),
            network: network.to_string(),
            profile: None,
            votes: Mutex::new(VotesSnapshot::default()),
        }
    }

    /// Also follows the vote ledger described by `profile`.
    pub fn with_votes(mut self, profile: NetworkProfile) -> Self {
        self.profile = Some(profile);
        self
    }

    pub fn votes(&self) -> MutexGuard<'_, VotesSnapshot> {
        lock(&self.votes)
    }

    pub fn index(&self) -> MutexGuard<'_, Index> {
        lock(&self.index)
    }

    pub fn meta(&self) -> MutexGuard<'_, Meta> {
        lock(&self.meta)
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// One sync round: downloads the new blocks from the first node that serves a chain
/// the index accepts (every certificate is verified here, not trusted) and refreshes
/// the health of every node.
pub fn sync_once(shared: &Shared, client: &NodeClient) {
    let from = shared.index().height() + 1;
    let mut error: Option<String> = None;
    let mut done = false;
    for addr in client.addrs() {
        match NodeClient::econ_fetch_from_node(*addr, from) {
            Ok(blocks) => {
                let r = shared.index().extend(&blocks);
                match r {
                    Ok(_) => {
                        done = true;
                        break;
                    }
                    Err(e) => error = Some(format!("nó {addr}: {e}")),
                }
            }
            Err(e) => error = Some(format!("nó {addr}: {e}")),
        }
    }
    if let Some(profile) = &shared.profile {
        // verified by `sync_chain` (signatures, linkage, Merkle log); an unreachable
        // network keeps the last snapshot
        if let Ok(chain) = client.sync_chain(profile) {
            *shared.votes() = snapshot(&chain);
        }
    }
    let nodes = client
        .addrs()
        .iter()
        .map(|a: &SocketAddr| match client.econ_status(*a) {
            Ok(s) => NodeHealth {
                addr: a.to_string(),
                up: true,
                height: Some(s.height),
                mempool: Some(s.mempool),
                is_validator: Some(s.is_validator),
            },
            Err(_) => NodeHealth {
                addr: a.to_string(),
                up: false,
                height: None,
                mempool: None,
                is_validator: None,
            },
        })
        .collect();
    let mut m = shared.meta();
    m.nodes = nodes;
    m.last_sync_unix = now();
    // a node being down is not an error while another one served the chain
    m.last_error = if done { None } else { error };
}

/// Syncs forever, every `every` (a thread of its own).
pub fn spawn_sync(shared: Arc<Shared>, client: NodeClient, every: Duration) {
    thread::spawn(move || {
        loop {
            sync_once(&shared, &client);
            thread::sleep(every);
        }
    });
}
