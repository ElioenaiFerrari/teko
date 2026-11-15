use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use prost::Message;
use teko_bulletin::Entry;
use teko_chain::{Block, BlockEntry, Chain, NetworkProfile};

use crate::error::NodeError;
use crate::wire::{
    self, Ack, BlocksReply, EconStatusReply, GetBlocks, Msg, StatusReply, SubmitEntries, SubmitTxs,
};

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

/// Status of one node's economy chain.
#[derive(Clone, Debug)]
pub struct EconStatus {
    pub addr: SocketAddr,
    pub chain_id: String,
    pub height: u64,
    pub state_root: Vec<u8>,
    pub mempool: u32,
    pub founder_phase_active: bool,
    pub founder_target: u64,
    pub total_stake: u64,
    /// (consensus key, owner address, voting power)
    pub validators: Vec<(Vec<u8>, Vec<u8>, u64)>,
    pub is_validator: bool,
    pub round: u32,
    pub evidence: u32,
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

    // ───────────────────────── economy (ADR-0028) ─────────────────────────

    /// Queues signed transactions on the first reachable node (it gossips them).
    /// Returns how many were new.
    pub fn econ_submit(&self, txs: &[teko_ledger::Tx]) -> Result<u32, NodeError> {
        let m = wire::msg(
            wire::ECON_TX,
            &SubmitTxs {
                txs: txs.iter().map(teko_ledger::Tx::to_bytes).collect(),
            },
        );
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

    pub fn econ_status(&self, addr: SocketAddr) -> Result<EconStatus, NodeError> {
        let r = call(
            addr,
            &Msg {
                kind: wire::ECON_STATUS,
                body: Vec::new(),
            },
            TIMEOUT,
        )?;
        if r.kind != wire::ECON_STATUS_REPLY {
            return Err(NodeError::Protocol("this node has no economy chain".into()));
        }
        let s: EconStatusReply = wire::decode(&r)?;
        Ok(EconStatus {
            addr,
            chain_id: s.chain_id,
            height: s.height,
            state_root: s.state_root,
            mempool: s.mempool,
            founder_phase_active: s.founder_phase_active,
            founder_target: s.founder_target,
            total_stake: s.total_stake,
            validators: s
                .validators
                .into_iter()
                .map(|v| (v.key, v.owner, v.power))
                .collect(),
            is_validator: s.is_validator,
            round: s.round,
            evidence: s.evidence,
        })
    }

    /// Downloads and VERIFIES the economy chain: every certificate is checked against the
    /// validator set the replayed state implies. Trusts no node; returns the longest
    /// verified chain among the reachable nodes.
    pub fn econ_sync(
        &self,
        genesis: &teko_economy::EconomyGenesis,
    ) -> Result<teko_economy::EconomyChain, NodeError> {
        let mut best: Option<teko_economy::EconomyChain> = None;
        let mut last = NodeError::Unreachable;
        for a in &self.addrs {
            match Self::econ_pull(*a, genesis) {
                Ok(c) => {
                    if best.as_ref().is_none_or(|b| c.height() > b.height()) {
                        best = Some(c);
                    }
                }
                Err(e) => last = e,
            }
        }
        best.ok_or(last)
    }

    /// Brings an already verified chain up to date, downloading ONLY the new blocks
    /// (each one verified by `apply_decided`). Tries the nodes in order.
    pub fn econ_pull_into(&self, chain: &mut teko_economy::EconomyChain) -> Result<(), NodeError> {
        let mut last = NodeError::Unreachable;
        for a in &self.addrs {
            match Self::econ_pull_from(*a, chain) {
                Ok(()) => return Ok(()),
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    /// Downloads the final blocks from height `from` on from ONE node, WITHOUT verifying
    /// them: the caller (the explorer index) verifies every certificate itself.
    pub fn econ_fetch_from_node(
        addr: SocketAddr,
        from: u64,
    ) -> Result<Vec<teko_economy::DecidedBlock>, NodeError> {
        let mut out: Vec<teko_economy::DecidedBlock> = Vec::new();
        loop {
            let next = from.saturating_add(out.len() as u64);
            let req = wire::msg(
                wire::ECON_BLOCKS,
                &GetBlocks {
                    from: next,
                    max: 200,
                },
            );
            let r = call(addr, &req, Duration::from_secs(10))?;
            if r.kind != wire::BLOCKS_REPLY {
                return Err(NodeError::Protocol("unexpected reply".into()));
            }
            let reply: BlocksReply = wire::decode(&r)?;
            if reply.blocks.is_empty() {
                return Ok(out);
            }
            for raw in reply.blocks {
                out.push(
                    teko_economy::DecidedBlock::decode(raw.as_slice())
                        .map_err(|_| NodeError::Protocol("malformed block".into()))?,
                );
            }
        }
    }

    fn econ_pull(
        addr: SocketAddr,
        genesis: &teko_economy::EconomyGenesis,
    ) -> Result<teko_economy::EconomyChain, NodeError> {
        let mut chain = teko_economy::EconomyChain::new(genesis.clone())
            .map_err(|e| NodeError::Economy(format!("{e:?}")))?;
        Self::econ_pull_from(addr, &mut chain)?;
        Ok(chain)
    }

    fn econ_pull_from(
        addr: SocketAddr,
        chain: &mut teko_economy::EconomyChain,
    ) -> Result<(), NodeError> {
        loop {
            let req = wire::msg(
                wire::ECON_BLOCKS,
                &GetBlocks {
                    from: chain.height() + 1,
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
                let d = teko_economy::DecidedBlock::decode(raw.as_slice())
                    .map_err(|_| NodeError::Protocol("malformed block".into()))?;
                chain.apply_decided(d).map_err(|e| NodeError::replay(&e))?;
            }
        }
    }
}
