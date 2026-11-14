use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use prost::Message;
use teko_bulletin::Entry;
use teko_chain::{Block, BlockEntry, Chain, NetworkProfile};

use crate::error::NodeError;
use crate::wire::{self, Ack, BlocksReply, GetBlocks, Msg, StatusReply, SubmitEntries};

const TIMEOUT: Duration = Duration::from_secs(3);

pub(crate) fn call(addr: SocketAddr, m: &Msg, timeout: Duration) -> Result<Msg, NodeError> {
    let mut s = TcpStream::connect_timeout(&addr, timeout)?;
    s.set_read_timeout(Some(timeout))?;
    s.set_write_timeout(Some(timeout))?;
    let _ = s.set_nodelay(true);
    wire::write_msg(&mut s, m)?;
    wire::read_msg(&mut s)
}

#[derive(Clone, Debug)]
pub struct RemoteStatus {
    pub addr: SocketAddr,
    pub chain_id: String,
    pub height: u64,
    pub log_size: u64,
    pub log_root: Vec<u8>,
    pub mempool: u32,
    pub peers_online: u32,
    pub peers_total: u32,
    pub equivocations: u32,
}

/// Talks to any node of a network. It trusts NONE of them: `sync_chain` re-verifies
/// every block (M-of-N signatures, hash chain, Merkle state) before returning.
pub struct NodeClient {
    addrs: Vec<SocketAddr>,
}

impl NodeClient {
    pub fn new(addrs: Vec<SocketAddr>) -> Self {
        Self { addrs }
    }

    pub fn addrs(&self) -> &[SocketAddr] {
        &self.addrs
    }

    /// Queues entries on the first reachable node (it gossips them to the rest).
    /// Returns how many were new (the node drops exact duplicates).
    pub fn submit(&self, entries: &[Entry]) -> Result<u32, NodeError> {
        let body = SubmitEntries {
            entries: entries.iter().map(BlockEntry::from).collect(),
        };
        let m = wire::msg(wire::SUBMIT, &body);
        let mut last = NodeError::Unreachable;
        for a in &self.addrs {
            match call(*a, &m, TIMEOUT) {
                Ok(r) if r.kind == wire::ACK => {
                    let ack: Ack = wire::decode(&r)?;
                    if ack.ok {
                        return Ok(ack.accepted);
                    }
                    last = NodeError::Refused(ack.info);
                }
                Ok(_) => last = NodeError::Protocol("unexpected reply".into()),
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    pub fn status(&self, addr: SocketAddr) -> Result<RemoteStatus, NodeError> {
        let r = call(
            addr,
            &Msg {
                kind: wire::STATUS,
                body: Vec::new(),
            },
            TIMEOUT,
        )?;
        if r.kind != wire::STATUS_REPLY {
            return Err(NodeError::Protocol("unexpected reply".into()));
        }
        let s: StatusReply = wire::decode(&r)?;
        Ok(RemoteStatus {
            addr,
            chain_id: s.chain_id,
            height: s.height,
            log_size: s.log_size,
            log_root: s.log_root,
            mempool: s.mempool,
            peers_online: s.peers_online,
            peers_total: s.peers_total,
            equivocations: s.equivocations,
        })
    }

    /// Status of every node (unreachable ones are `None`).
    pub fn statuses(&self) -> Vec<(SocketAddr, Option<RemoteStatus>)> {
        self.addrs
            .iter()
            .map(|a| (*a, self.status(*a).ok()))
            .collect()
    }

    /// Downloads and VERIFIES the whole chain from the first node that answers.
    pub fn sync_chain(&self, profile: &NetworkProfile) -> Result<Chain, NodeError> {
        let mut last = NodeError::Unreachable;
        for a in &self.addrs {
            match self.sync_from(*a, profile) {
                Ok(c) => return Ok(c),
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    /// Brings `chain` up to date from the first node that answers, downloading
    /// ONLY the missing blocks (each one fully verified by `Chain::apply`).
    pub fn sync_into(&self, chain: &mut Chain) -> Result<(), NodeError> {
        let mut last = NodeError::Unreachable;
        for a in &self.addrs {
            match Self::pull(*a, chain) {
                Ok(()) => return Ok(()),
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    fn sync_from(&self, addr: SocketAddr, profile: &NetworkProfile) -> Result<Chain, NodeError> {
        let mut chain = Chain::new(profile.clone());
        Self::pull(addr, &mut chain)?;
        Ok(chain)
    }

    fn pull(addr: SocketAddr, chain: &mut Chain) -> Result<(), NodeError> {
        loop {
            let req = wire::msg(
                wire::GET_BLOCKS,
                &GetBlocks {
                    from: chain.height(),
                    max: 200,
                },
            );
            let r = call(addr, &req, Duration::from_secs(10))?;
            if r.kind != wire::BLOCKS_REPLY {
                return Err(NodeError::Protocol("unexpected reply".into()));
            }
            let reply: BlocksReply = wire::decode(&r)?;
            if reply.blocks.is_empty() {
                return Ok(());
            }
            for raw in reply.blocks {
                let b = Block::decode(raw.as_slice())
                    .map_err(|_| NodeError::Protocol("malformed block".into()))?;
                chain.apply(b)?;
            }
        }
    }
}
