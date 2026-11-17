//! The vote chain of a node (ADR-0014, ADR-0034): bulletin entries ordered by BFT among the
//! institutions (equal power, no coin), hosted by the generic `Lane`. This module is what is
//! specific to votes: the application glue, entry submission and the status reply.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use prost::Message;
use teko_bft::{Commit, ValidatorSet};
use teko_bulletin::BulletinBoard;
use teko_chain::{Chain, DecidedBlock, NetworkProfile, VoteApp};
use teko_crypto::SigningKey;

use crate::error::NodeError;
use crate::lane::{self, Kinds, Lane, LaneApp, Setup, ack, lock_lane, send_all};
use crate::net::Net;
use crate::node::Shared;
use crate::setup::PeerLink;
use crate::wire::{self, Msg, StatusReply, SubmitEntries};

pub(crate) type Votes = Lane<VoteApp>;

pub(crate) const KINDS: Kinds = Kinds {
    bft: wire::VOTE_BFT,
    commit: wire::COMMIT,
    blocks: wire::GET_BLOCKS,
};

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl LaneApp for VoteApp {
    type Decided = DecidedBlock;
    const NAME: &'static str = "votes";

    fn height(&self) -> u64 {
        self.chain.height()
    }

    fn validator_set(&self) -> Option<ValidatorSet> {
        self.chain.validator_set().ok()
    }

    /// The institutions in force and those of a change that is still pending (ADR-0045): a newcomer
    /// may connect before its set takes effect.
    fn peer_keys(&self) -> Vec<Vec<u8>> {
        let now = self.chain.epoch_at(self.chain.height() + 1).epoch;
        self.chain
            .epochs()
            .iter()
            .filter(|e| e.epoch >= now)
            .flat_map(|e| e.set.clone())
            .collect()
    }

    fn pending(&self) -> usize {
        self.mempool_len()
    }

    fn apply(&mut self, d: DecidedBlock) -> Result<(), NodeError> {
        Ok(self.on_decided(d)?)
    }

    fn parts(d: &DecidedBlock) -> Option<(u64, Vec<u8>, Commit)> {
        let b = d.block().ok()?;
        Some((b.height, d.payload.clone(), d.commit.clone()?))
    }

    fn make(payload: Vec<u8>, commit: Commit) -> DecidedBlock {
        DecidedBlock {
            payload,
            commit: Some(commit),
        }
    }

    fn blocks_from(&self, from: u64, max: usize) -> Vec<Vec<u8>> {
        self.chain
            .blocks_after(from.saturating_sub(1))
            .iter()
            .take(max)
            .map(Message::encode_to_vec)
            .collect()
    }
    /// The vote chain has no coin to burn: the evidence is kept for the node to take it to the
    /// ECONOMY, where the institution's bond is (ADR-0035). It is also counted in the status.
    fn on_evidence(&mut self, ev: &teko_bft::Evidence, _key: &teko_crypto::SigningKey) -> Vec<Msg> {
        self.note_evidence(ev.clone());
        Vec::new()
    }
}

/// Opens the vote lane of a node: the chain is rebuilt from `votes.blocks` (every block
/// re-verified) and the engine starts idle.
pub(crate) fn open(
    dir: &Path,
    profile: &NetworkProfile,
    key: &SigningKey,
    index: usize,
    peers: Vec<PeerLink>,
    net: Arc<Net>,
) -> Result<Votes, NodeError> {
    let app = VoteApp::new(Chain::new(profile.clone()), key, Box::new(now_secs));
    Lane::open(
        dir,
        app,
        Setup {
            chain_id: &profile.chain_id,
            key,
            index,
            peers,
            net,
            kinds: KINDS,
            batch: Duration::from_millis(profile.block_interval_ms),
        },
    )
}

/// Queues entries from a client (`from_client`: gossip them to the other institutions) or from
/// a peer (never re-forwarded).
pub(crate) fn submit(s: &Arc<Shared>, b: &SubmitEntries, from_client: bool) -> Msg {
    let mut entries = Vec::new();
    for e in &b.entries {
        match e.to_entry() {
            Ok(x) => entries.push(x),
            Err(_) => return ack(false, "invalid entry", 0),
        }
    }
    let mut accepted = 0u32;
    let (net, peers) = {
        let mut v = lock_lane(&s.votes);
        for e in entries {
            match v.app_mut().submit(e) {
                Ok(true) => accepted += 1,
                Ok(false) => {}
                Err(why) => return ack(false, &why, accepted),
            }
        }
        (v.net.clone(), v.peers.clone())
    };
    if from_client && accepted > 0 {
        // Gossip to the other institutions (they do not re-forward).
        send_all(&net, &peers, vec![wire::msg(wire::FORWARD, b)]);
    }
    ack(true, "queued", accepted)
}

fn status(s: &Arc<Shared>) -> Msg {
    let v = lock_lane(&s.votes);
    let chain = &v.app().chain;
    wire::msg(
        wire::STATUS_REPLY,
        &StatusReply {
            chain_id: chain.profile().chain_id.clone(),
            height: chain.height(),
            tip_hash: chain.tip_hash().to_vec(),
            log_size: chain.board().len(),
            log_root: chain.board().root().to_vec(),
            mempool: v.app().mempool_len() as u32,
            peers_online: v.peers_online,
            peers_total: s.cfg.links.len().saturating_sub(1) as u32,
            validator_index: s.cfg.index as u32,
            equivocations: v.evidence,
        },
    )
}

/// Handles the vote-chain wire messages. Returns None for kinds that are not ours.
pub(crate) fn handle(s: &Arc<Shared>, req: &Msg) -> Option<Msg> {
    if let Some(r) = lane::handle(&s.votes, req) {
        return Some(r);
    }
    match req.kind {
        wire::SUBMIT | wire::FORWARD => Some(match wire::decode::<SubmitEntries>(req) {
            Ok(b) => submit(s, &b, req.kind == wire::SUBMIT),
            Err(_) => ack(false, "malformed", 0),
        }),
        wire::STATUS => Some(status(s)),
        _ => None,
    }
}
