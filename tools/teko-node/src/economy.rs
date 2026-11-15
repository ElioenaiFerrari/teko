//! The economy service of a node: a BFT `Engine` over an `EconomyChain` (ADR-0028).
//!
//! It runs beside the PoA vote chain, with its own mempool, files and wire messages.
//! The engine is SANS-I/O; this module is its host: timers, gossip over the loopback
//! wire, persistence, catch-up and pacing of empty blocks.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use prost::Message;
use teko_bft::{Action, Engine, Message as BftMsg, Proposal, Step, Vote};
use teko_crypto::SigningKey;
use teko_economy::{DecidedBlock, EconApp, EconomyChain, EconomyGenesis};
use teko_ledger::Tx;

use crate::client::call;
use crate::error::NodeError;
use crate::node::Shared;
use crate::wire::{
    self, Ack, BftEnvelope, BlocksReply, EconStatusReply, EconValidator, GetBlocks, Msg, SubmitTxs,
};

const PEER_TIMEOUT: Duration = Duration::from_millis(1_500);
const RESEND_EVERY: Duration = Duration::from_millis(1_000);
const SYNC_EVERY: Duration = Duration::from_millis(1_000);
const MAX_FUTURE: usize = 4_000;

/// Public configuration of the economy chain of this network.
#[derive(Clone)]
pub struct EconomyConfig {
    pub genesis: EconomyGenesis,
    /// An empty block is proposed when nothing else happens for this long (it is what
    /// pays the founder's rewards).
    pub empty_block_interval_ms: u64,
}

struct Timer {
    at: Instant,
    height: u64,
    round: u32,
    step: Step,
}

pub(crate) struct Econ {
    engine: Engine<EconApp>,
    peers: Vec<SocketAddr>,
    index: usize,
    blocks_file: File,
    wal_path: PathBuf,
    timers: Vec<Timer>,
    /// The engine is active at height `chain.height() + 1`.
    started: bool,
    last_decide: Instant,
    future: Vec<(u64, BftMsg)>,
    evidence: u32,
    last_resend: Instant,
    last_sync: Instant,
    interval: Duration,
    me: Vec<u8>,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn persist(file: &mut File, d: &DecidedBlock) -> Result<(), NodeError> {
    let bytes = d.encode_to_vec();
    file.write_all(&(bytes.len() as u32).to_be_bytes())?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(())
}

fn load_blocks(path: &Path, chain: &mut EconomyChain) -> Result<(), NodeError> {
    let Ok(mut f) = File::open(path) else {
        return Ok(());
    };
    let mut data = Vec::new();
    f.read_to_end(&mut data)?;
    let mut at = 0usize;
    while let Some(head) = data.get(at..at + 4) {
        let n = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
        let Some(raw) = data.get(at + 4..at + 4 + n) else {
            break; // torn last write
        };
        let d = DecidedBlock::decode(raw)
            .map_err(|_| NodeError::Economy("corrupt economy.blocks".into()))?;
        // every block is RE-VERIFIED at startup (certificate, proposer, execution, root)
        chain
            .apply_decided(d)
            .map_err(|e| NodeError::Economy(format!("economy.blocks: {e:?}")))?;
        at += 4 + n;
    }
    Ok(())
}

/// Last `(height, round)` this validator took part in (so a restart does not vote twice
/// in the same round).
fn read_wal(path: &Path) -> Option<(u64, u32)> {
    let text = std::fs::read_to_string(path).ok()?;
    let line = text.lines().last()?;
    let (h, r) = line.split_once(' ')?;
    Some((h.parse().ok()?, r.parse().ok()?))
}

fn write_wal(path: &Path, height: u64, round: u32) {
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{height} {round}");
        let _ = f.sync_all();
    }
}

impl Econ {
    pub(crate) fn open(
        dir: &Path,
        cfg: &EconomyConfig,
        key: &SigningKey,
        index: usize,
        peers: Vec<SocketAddr>,
    ) -> Result<Self, NodeError> {
        let mut chain = EconomyChain::new(cfg.genesis.clone())
            .map_err(|e| NodeError::Economy(format!("genesis: {e:?}")))?;
        let blocks_path = dir.join("economy.blocks");
        load_blocks(&blocks_path, &mut chain)?;
        let blocks_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&blocks_path)?;
        let chain_id = cfg.genesis.chain_id.clone();
        let app = EconApp::new(chain, key, Box::new(now_secs));
        let engine = Engine::new(&chain_id, key.clone(), app)
            .map_err(|e| NodeError::Economy(format!("{e:?}")))?;
        Ok(Self {
            engine,
            peers,
            index,
            blocks_file,
            wal_path: dir.join("economy.wal"),
            timers: Vec::new(),
            started: false,
            last_decide: Instant::now(),
            future: Vec::new(),
            evidence: 0,
            last_resend: Instant::now(),
            last_sync: Instant::now(),
            interval: Duration::from_millis(cfg.empty_block_interval_ms.max(20)),
            me: teko_crypto::public_key_bytes(key),
        })
    }

    pub(crate) fn height(&self) -> u64 {
        self.engine.app().chain.height()
    }

    fn chain(&self) -> &EconomyChain {
        &self.engine.app().chain
    }

    /// Wire messages for every peer, queued by the callers and sent AFTER the lock is
    /// released.
    fn exec(&mut self, actions: Vec<Action>, out: &mut Vec<Msg>) {
        for a in actions {
            match a {
                Action::Broadcast(m) => {
                    let (h, env) = match &m {
                        BftMsg::Proposal(p) => (
                            p.height,
                            BftEnvelope {
                                proposal: p.encode_to_vec(),
                                vote: Vec::new(),
                            },
                        ),
                        BftMsg::Vote(v) => (
                            v.height,
                            BftEnvelope {
                                proposal: Vec::new(),
                                vote: v.encode_to_vec(),
                            },
                        ),
                    };
                    // Remember we spoke in this round BEFORE the message leaves.
                    write_wal(&self.wal_path, h, self.engine.round());
                    out.push(wire::msg(wire::ECON_BFT, &env));
                }
                Action::Schedule {
                    round,
                    step,
                    after_ms,
                } => self.timers.push(Timer {
                    at: Instant::now() + Duration::from_millis(after_ms),
                    height: self.engine.height(),
                    round,
                    step,
                }),
                Action::Evidence(_) => self.evidence += 1,
                Action::Decide { commit, payload } => {
                    let d = DecidedBlock {
                        payload,
                        commit: Some(commit),
                    };
                    self.finish(d, out);
                }
            }
        }
    }

    /// Applies a block the engine decided and gossips it to everyone.
    fn finish(&mut self, d: DecidedBlock, out: &mut Vec<Msg>) {
        if self.engine.app_mut().on_decided(d.clone()).is_err() {
            return;
        }
        let _ = persist(&mut self.blocks_file, &d);
        out.push(wire::msg(wire::ECON_COMMIT, &d));
        self.started = false;
        self.last_decide = Instant::now();
        self.timers.clear();
    }

    /// Starts the next height when it is time: something is waiting in the mempool, or
    /// the empty-block interval passed.
    fn maybe_start(&mut self, out: &mut Vec<Msg>) {
        if self.started {
            return;
        }
        let due = self.engine.app().mempool_len() > 0
            || self.last_decide.elapsed() >= self.interval
            || self.chain().height() == 0 && self.last_decide.elapsed() >= self.interval / 4;
        if !due {
            return;
        }
        let Ok(set) = self.chain().validator_set() else {
            return;
        };
        let height = self.chain().height() + 1;
        let first_round = match read_wal(&self.wal_path) {
            Some((h, r)) if h == height => r.saturating_add(1),
            _ => 0,
        };
        self.started = true;
        let actions = self.engine.start_height_at(height, set, first_round);
        self.exec(actions, out);
        // replay what arrived early for this height
        let early: Vec<BftMsg> = {
            let mut keep = Vec::new();
            let mut now = Vec::new();
            for (h, m) in std::mem::take(&mut self.future) {
                match h.cmp(&height) {
                    std::cmp::Ordering::Equal => now.push(m),
                    std::cmp::Ordering::Greater => keep.push((h, m)),
                    std::cmp::Ordering::Less => {}
                }
            }
            self.future = keep;
            now
        };
        for m in early {
            if self.started {
                let acts = self.engine.on_message(m);
                self.exec(acts, out);
            }
        }
    }

    fn on_bft(&mut self, m: BftMsg, out: &mut Vec<Msg>) {
        let h = match &m {
            BftMsg::Proposal(p) => p.height,
            BftMsg::Vote(v) => v.height,
        };
        if h <= self.chain().height() {
            return; // an old height
        }
        if !self.started || h > self.engine.height() {
            if self.future.len() < MAX_FUTURE {
                self.future.push((h, m));
            }
            return;
        }
        let acts = self.engine.on_message(m);
        self.exec(acts, out);
    }

    /// A final block from a peer (pushed or synced): verified before it is applied.
    fn accept_decided(&mut self, d: DecidedBlock, out: &mut Vec<Msg>) {
        let Ok(b) = d.block() else { return };
        if b.height != self.chain().height() + 1 {
            return;
        }
        if self.started && self.engine.height() == b.height {
            let Some(c) = d.commit.clone() else { return };
            let acts = self.engine.on_commit(&c, &d.payload);
            self.exec(acts, out);
        } else if self.engine.app_mut().on_decided(d.clone()).is_ok() {
            let _ = persist(&mut self.blocks_file, &d);
            self.last_decide = Instant::now();
            self.future.retain(|(h, _)| *h > b.height);
        }
    }

    fn tick(&mut self, out: &mut Vec<Msg>) {
        let now = Instant::now();
        let (due, rest): (Vec<Timer>, Vec<Timer>) = std::mem::take(&mut self.timers)
            .into_iter()
            .partition(|t| t.at <= now);
        self.timers = rest;
        for t in due {
            if t.height == self.engine.height() && self.started {
                let acts = self.engine.on_timeout(t.round, t.step);
                self.exec(acts, out);
            }
        }
        self.maybe_start(out);
        if self.started && self.last_resend.elapsed() >= RESEND_EVERY {
            self.last_resend = Instant::now();
            let acts = self.engine.resend();
            self.exec(acts, out);
        }
    }

    fn status(&self) -> EconStatusReply {
        let chain = self.chain();
        let l = chain.ledger();
        let set = chain.validator_set().ok();
        let powers: BTreeMap<Vec<u8>, u64> = set
            .as_ref()
            .map(|s| {
                s.validators()
                    .iter()
                    .map(|v| (v.key.clone(), v.power))
                    .collect()
            })
            .unwrap_or_default();
        EconStatusReply {
            chain_id: chain.chain_id().to_string(),
            height: chain.height(),
            state_root: l.state_root().to_vec(),
            mempool: self.engine.app().mempool_len() as u32,
            founder_phase_active: teko_coin::founder_phase_active(l),
            total_stake: teko_coin::total_stake(l),
            validators: powers
                .iter()
                .map(|(k, p)| EconValidator {
                    key: k.clone(),
                    owner: chain.owner_of(k).to_vec(),
                    power: *p,
                })
                .collect(),
            is_validator: powers.contains_key(&self.me),
            round: self.engine.round(),
            evidence: self.evidence,
            founder_target: teko_coin::config(l).map_or(0, |c| c.founder_target),
        }
    }
}

fn send_all(peers: &[SocketAddr], index: usize, msgs: Vec<Msg>) {
    for m in msgs {
        for (i, addr) in peers.iter().enumerate() {
            if i == index {
                continue;
            }
            let (m, addr) = (m.clone(), *addr);
            thread::spawn(move || {
                let _ = call(addr, &m, PEER_TIMEOUT);
            });
        }
    }
}

fn ack(ok: bool, info: &str, accepted: u32) -> Msg {
    wire::msg(
        wire::ACK,
        &Ack {
            ok,
            info: info.to_string(),
            accepted,
        },
    )
}

/// The thread body: ticks timers, starts heights, resends, and syncs from peers.
pub(crate) fn run_loop(s: &Arc<Shared>) {
    while !s.stop.load(std::sync::atomic::Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(40));
        let Some(econ) = s.econ.as_ref() else { return };
        let mut out = Vec::new();
        let (peers, index, sync_now, from) = {
            let mut e = crate::node::lock_econ(econ);
            e.tick(&mut out);
            let sync_now = e.last_sync.elapsed() >= SYNC_EVERY;
            if sync_now {
                e.last_sync = Instant::now();
            }
            (e.peers.clone(), e.index, sync_now, e.chain().height() + 1)
        };
        send_all(&peers, index, out);
        if sync_now {
            sync_from_peers(s, &peers, index, from);
        }
    }
}

fn sync_from_peers(s: &Arc<Shared>, peers: &[SocketAddr], index: usize, from: u64) {
    let Some(econ) = s.econ.as_ref() else { return };
    for (i, addr) in peers.iter().enumerate() {
        if i == index {
            continue;
        }
        let req = wire::msg(wire::ECON_BLOCKS, &GetBlocks { from, max: 100 });
        let Ok(r) = call(*addr, &req, PEER_TIMEOUT) else {
            continue;
        };
        let Ok(reply) = wire::decode::<BlocksReply>(&r) else {
            continue;
        };
        for raw in reply.blocks {
            let Ok(d) = DecidedBlock::decode(raw.as_slice()) else {
                break;
            };
            let mut out = Vec::new();
            {
                let mut e = crate::node::lock_econ(econ);
                e.accept_decided(d, &mut out);
            }
            send_all(peers, index, out);
        }
    }
}

/// Handles the economy wire messages. Returns None for kinds that are not ours.
pub(crate) fn handle(s: &Arc<Shared>, req: &Msg) -> Option<Msg> {
    let econ = s.econ.as_ref()?;
    let reply = match req.kind {
        wire::ECON_TX | wire::ECON_TX_FWD => {
            let Ok(b) = wire::decode::<SubmitTxs>(req) else {
                return Some(ack(false, "malformed", 0));
            };
            let mut accepted = 0u32;
            let mut fwd = Vec::new();
            {
                let mut e = crate::node::lock_econ(econ);
                for raw in &b.txs {
                    let Ok(tx) = Tx::from_bytes(raw) else {
                        return Some(ack(false, "invalid transaction", accepted));
                    };
                    match e.engine.app_mut().submit(tx) {
                        Ok(true) => {
                            accepted += 1;
                            fwd.push(raw.clone());
                        }
                        Ok(false) => {}
                        Err(why) => return Some(ack(false, &why, accepted)),
                    }
                }
                if req.kind == wire::ECON_TX && !fwd.is_empty() {
                    let m = wire::msg(wire::ECON_TX_FWD, &SubmitTxs { txs: fwd });
                    let (peers, index) = (e.peers.clone(), e.index);
                    drop(e);
                    send_all(&peers, index, vec![m]);
                }
            }
            ack(true, "queued", accepted)
        }
        wire::ECON_BFT => {
            let Ok(env) = wire::decode::<BftEnvelope>(req) else {
                return Some(ack(false, "malformed", 0));
            };
            let m = if env.proposal.is_empty() {
                Vote::decode(env.vote.as_slice()).ok().map(BftMsg::Vote)
            } else {
                Proposal::decode(env.proposal.as_slice())
                    .ok()
                    .map(BftMsg::Proposal)
            };
            let Some(m) = m else {
                return Some(ack(false, "malformed", 0));
            };
            let mut out = Vec::new();
            let (peers, index) = {
                let mut e = crate::node::lock_econ(econ);
                e.on_bft(m, &mut out);
                (e.peers.clone(), e.index)
            };
            send_all(&peers, index, out);
            ack(true, "ok", 0)
        }
        wire::ECON_COMMIT => {
            let Ok(d) = wire::decode::<DecidedBlock>(req) else {
                return Some(ack(false, "malformed", 0));
            };
            let mut out = Vec::new();
            let (peers, index) = {
                let mut e = crate::node::lock_econ(econ);
                e.accept_decided(d, &mut out);
                (e.peers.clone(), e.index)
            };
            send_all(&peers, index, out);
            ack(true, "ok", 0)
        }
        wire::ECON_BLOCKS => {
            let Ok(g) = wire::decode::<GetBlocks>(req) else {
                return Some(ack(false, "malformed", 0));
            };
            let e = crate::node::lock_econ(econ);
            let blocks: Vec<Vec<u8>> = e
                .chain()
                .blocks_from(g.from.max(1))
                .iter()
                .take(g.max.clamp(1, 200) as usize)
                .map(Message::encode_to_vec)
                .collect();
            wire::msg(
                wire::BLOCKS_REPLY,
                &BlocksReply {
                    blocks,
                    height: e.height(),
                },
            )
        }
        wire::ECON_STATUS => {
            let e = crate::node::lock_econ(econ);
            wire::msg(wire::ECON_STATUS_REPLY, &e.status())
        }
        _ => return None,
    };
    Some(reply)
}
