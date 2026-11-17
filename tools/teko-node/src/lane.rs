//! The BFT host of ONE chain of a node (ADR-0028, ADR-0034): a `teko_bft::Engine` over an
//! application (`LaneApp`), plus everything the sans-I/O engine leaves to the host: timers,
//! gossip over the wire, persistence (final blocks and the WAL of votes), catch-up from peers.
//!
//! A node runs one lane for the economy and one for the votes; they differ only in the
//! application, the wire message kinds and the file names.
//!
//! NO BLOCK IS MADE ON A TIMER (ADR-0034): a height starts only when something is waiting in the
//! mempool (after a short batching delay so a burst shares a block) or when the peers already
//! started it (their proposal or votes arrived), so an idle node still votes and catches up.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use prost::Message;
use teko_bft::{
    Action, App, Commit, Engine, Evidence, Message as BftMsg, Proposal, Step, ValidatorSet, Vote,
};
use teko_crypto::SigningKey;

use crate::error::NodeError;
use crate::net::{Chan, Net};
use crate::setup::PeerLink;
use crate::wire::{self, Ack, BftEnvelope, BlocksReply, GetBlocks, Msg};

/// A catch-up request may carry a page of blocks back.
const SYNC_TIMEOUT: Duration = Duration::from_secs(10);
const RESEND_EVERY: Duration = Duration::from_millis(1_000);
const SYNC_EVERY: Duration = Duration::from_millis(1_000);
const MAX_FUTURE: usize = 4_000;

/// Wire message kinds of one lane.
#[derive(Clone, Copy)]
pub(crate) struct Kinds {
    /// node -> node: one consensus message (proposal or vote).
    pub bft: u32,
    /// node -> node: a decided block with its certificate.
    pub commit: u32,
    /// anyone -> node: `GetBlocks` -> `BlocksReply` of decided blocks.
    pub blocks: u32,
}

/// What a chain must offer to be hosted by a lane.
pub(crate) trait LaneApp: App + Send + 'static {
    /// The final block as it goes over the wire and into the block file.
    type Decided: Message + Default + Clone;
    /// File prefix (`<name>.blocks`, `<name>.wal`).
    const NAME: &'static str;

    fn height(&self) -> u64;
    /// The validators of the NEXT height.
    fn validator_set(&self) -> Option<ValidatorSet>;
    /// The consensus keys that may open a peer session for this chain: the validators of the next
    /// height (a vote chain adds the sets of changes still pending, ADR-0045).
    fn peer_keys(&self) -> Vec<Vec<u8>> {
        self.validator_set()
            .map(|s| s.validators().iter().map(|v| v.key.clone()).collect())
            .unwrap_or_default()
    }
    fn pending(&self) -> usize;
    /// Verifies a final block completely and applies it (every block is re-verified at startup).
    fn apply(&mut self, d: Self::Decided) -> Result<(), NodeError>;
    /// Height, payload and certificate of a final block.
    fn parts(d: &Self::Decided) -> Option<(u64, Vec<u8>, Commit)>;
    fn make(payload: Vec<u8>, commit: Commit) -> Self::Decided;
    /// Encoded final blocks with height >= `from`.
    fn blocks_from(&self, from: u64, max: usize) -> Vec<Vec<u8>>;
    /// Reacts to equivocation evidence; returns wire messages to gossip (the economy turns it
    /// into a `Slash` transaction; the vote chain has no coin to burn and only counts it).
    fn on_evidence(&mut self, _ev: &Evidence, _key: &SigningKey) -> Vec<Msg> {
        Vec::new()
    }
}

struct Timer {
    at: Instant,
    height: u64,
    round: u32,
    step: Step,
}

pub(crate) struct Lane<A: LaneApp> {
    pub(crate) engine: Engine<A>,
    /// Every node of `peers.txt`, by index (this node's own included).
    pub(crate) peers: Vec<PeerLink>,
    pub(crate) net: Arc<Net>,
    /// `peers.txt` of the network, re-read now and then: nodes that join by a rotation are APPENDED
    /// to it (ADR-0045) and the running nodes must learn their addresses.
    pub(crate) peers_file: Option<std::path::PathBuf>,
    pub(crate) last_peers_read: Instant,
    /// This node's position in `peers.txt` (kept for diagnostics: peers are addressed by KEY).
    #[allow(dead_code)]
    pub(crate) index: usize,
    blocks_file: File,
    wal_path: PathBuf,
    timers: Vec<Timer>,
    /// The engine is active at height `app.height() + 1`.
    started: bool,
    /// When the mempool became non-empty while idle (drives the batching delay).
    pending_since: Option<Instant>,
    batch: Duration,
    future: Vec<(u64, BftMsg)>,
    pub(crate) evidence: u32,
    /// Peers that answered the last catch-up round (for status).
    pub(crate) peers_online: u32,
    /// Offences already reported by this node (validator, height, round, kind).
    reported: std::collections::HashSet<(Vec<u8>, u64, u32, u32)>,
    key: SigningKey,
    last_resend: Instant,
    last_sync: Instant,
    kinds: Kinds,
}

pub(crate) fn lock_lane<A: LaneApp>(m: &Mutex<Lane<A>>) -> MutexGuard<'_, Lane<A>> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn persist<D: Message>(file: &mut File, d: &D) -> Result<(), NodeError> {
    let bytes = d.encode_to_vec();
    file.write_all(&(bytes.len() as u32).to_be_bytes())?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(())
}

fn load_blocks<A: LaneApp>(path: &Path, app: &mut A) -> Result<(), NodeError> {
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
        let d = A::Decided::decode(raw)
            .map_err(|_| NodeError::Protocol(format!("corrupt {}.blocks", A::NAME)))?;
        // every block is RE-VERIFIED at startup (certificate, proposer, content, root)
        app.apply(d)?;
        at += 4 + n;
    }
    Ok(())
}

/// What a restart must remember (ADR-0030): the last `(height, round)` this validator
/// spoke in (so it does not vote twice in a round) and the block it is LOCKED on (so it
/// does not prevote a different block after precommitting one).
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct WalEntry {
    pub height: u64,
    pub round: u32,
    pub lock: Option<(Vec<u8>, u32)>,
}

/// Line format: `height round [lock_round lock_block_id_hex]`.
pub(crate) fn read_wal(path: &Path) -> Option<WalEntry> {
    let text = std::fs::read_to_string(path).ok()?;
    let line = text.lines().last()?;
    let mut it = line.split_whitespace();
    let height = it.next()?.parse().ok()?;
    let round = it.next()?.parse().ok()?;
    let lock = match (it.next(), it.next()) {
        (Some(r), Some(id)) => Some((teko_crypto::hex::decode(id).ok()?, r.parse().ok()?)),
        _ => None,
    };
    Some(WalEntry {
        height,
        round,
        lock,
    })
}

/// Appended and synced BEFORE the vote leaves the node.
pub(crate) fn write_wal(path: &Path, height: u64, round: u32, lock: Option<(&[u8], u32)>) {
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = match lock {
            Some((id, r)) => writeln!(f, "{height} {round} {r} {}", teko_crypto::hex::encode(id)),
            None => writeln!(f, "{height} {round}"),
        };
        let _ = f.sync_all();
    }
}

/// Everything a lane needs besides its application.
pub(crate) struct Setup<'a> {
    pub chain_id: &'a str,
    pub key: &'a SigningKey,
    /// This node's index among `peers`.
    pub index: usize,
    pub peers: Vec<PeerLink>,
    pub net: Arc<Net>,
    pub kinds: Kinds,
    /// How long the first waiting entry/transaction waits so a burst shares one block.
    pub batch: Duration,
}

impl<A: LaneApp> Lane<A> {
    pub(crate) fn open(dir: &Path, mut app: A, setup: Setup<'_>) -> Result<Self, NodeError> {
        let Setup {
            chain_id,
            key,
            index,
            peers,
            net,
            kinds,
            batch,
        } = setup;
        let blocks_path = dir.join(format!("{}.blocks", A::NAME));
        load_blocks(&blocks_path, &mut app)?;
        let blocks_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&blocks_path)?;
        let engine = Engine::new(chain_id, key.clone(), app)
            .map_err(|e| NodeError::Protocol(format!("{}: {e:?}", A::NAME)))?;
        Ok(Self {
            engine,
            peers,
            net,
            peers_file: dir.parent().map(|p| p.join("peers.txt")),
            last_peers_read: Instant::now(),
            index,
            blocks_file,
            wal_path: dir.join(format!("{}.wal", A::NAME)),
            timers: Vec::new(),
            started: false,
            pending_since: None,
            batch,
            future: Vec::new(),
            evidence: 0,
            peers_online: 0,
            reported: std::collections::HashSet::new(),
            key: key.clone(),
            last_resend: Instant::now(),
            last_sync: Instant::now(),
            kinds,
        })
    }

    pub(crate) fn app(&self) -> &A {
        self.engine.app()
    }

    pub(crate) fn app_mut(&mut self) -> &mut A {
        self.engine.app_mut()
    }

    pub(crate) fn height(&self) -> u64 {
        self.engine.app().height()
    }

    /// This node's CONSENSUS public key.
    pub(crate) fn me(&self) -> Vec<u8> {
        teko_crypto::public_key_bytes(&self.key)
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
                    let lock = self.engine.locked().map(|(id, r)| (id.to_vec(), r));
                    write_wal(
                        &self.wal_path,
                        h,
                        self.engine.round(),
                        lock.as_ref().map(|(id, r)| (id.as_slice(), *r)),
                    );
                    out.push(wire::msg(self.kinds.bft, &env));
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
                Action::Evidence(ev) => {
                    self.evidence += 1;
                    self.report(&ev, out);
                }
                Action::Decide { commit, payload } => {
                    let d = A::make(payload, commit);
                    self.finish(d, out);
                }
            }
        }
    }

    /// Once per offence: lets the application react (the economy queues a `Slash`).
    pub(crate) fn report(&mut self, ev: &Evidence, out: &mut Vec<Msg>) {
        let (Some(a), Some(validator)) = (&ev.a, ev.validator()) else {
            return;
        };
        if !self
            .reported
            .insert((validator.to_vec(), a.height, a.round, a.kind))
        {
            return;
        }
        let key = self.key.clone();
        out.extend(self.engine.app_mut().on_evidence(ev, &key));
    }

    /// Applies a block the engine decided and gossips it to everyone.
    fn finish(&mut self, d: A::Decided, out: &mut Vec<Msg>) {
        if self.engine.app_mut().apply(d.clone()).is_err() {
            return;
        }
        let _ = persist(&mut self.blocks_file, &d);
        out.push(wire::msg(self.kinds.commit, &d));
        self.started = false;
        self.pending_since = None;
        self.timers.clear();
    }

    /// Starts the next height only when there is a reason (ADR-0034): something waits in the
    /// mempool (a short batching delay lets a burst share one block), or the peers already
    /// started this height, so this node votes too. NEVER on a timer.
    fn maybe_start(&mut self, out: &mut Vec<Msg>) {
        if self.started {
            return;
        }
        let next = self.height() + 1;
        let peers_started = self.future.iter().any(|(h, _)| *h == next);
        let waiting = self.engine.app().pending() > 0;
        if waiting && self.pending_since.is_none() {
            self.pending_since = Some(Instant::now());
        }
        let batched = self
            .pending_since
            .is_some_and(|t| waiting && t.elapsed() >= self.batch);
        if !(peers_started || batched) {
            return;
        }
        let Some(set) = self.engine.app().validator_set() else {
            return;
        };
        let (first_round, lock) = match read_wal(&self.wal_path) {
            Some(w) if w.height == next => (w.round.saturating_add(1), w.lock),
            _ => (0, None),
        };
        self.started = true;
        self.pending_since = None;
        let actions = self
            .engine
            .start_height_locked(next, set, first_round, lock);
        self.exec(actions, out);
        // replay what arrived early for this height
        let early: Vec<BftMsg> = {
            let mut keep = Vec::new();
            let mut now = Vec::new();
            for (h, m) in std::mem::take(&mut self.future) {
                match h.cmp(&next) {
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

    pub(crate) fn on_bft(&mut self, m: BftMsg, out: &mut Vec<Msg>) {
        let h = match &m {
            BftMsg::Proposal(p) => p.height,
            BftMsg::Vote(v) => v.height,
        };
        if h <= self.height() {
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
    pub(crate) fn accept_decided(&mut self, d: A::Decided, out: &mut Vec<Msg>) {
        let Some((height, payload, commit)) = A::parts(&d) else {
            return;
        };
        if height != self.height() + 1 {
            return;
        }
        if self.started && self.engine.height() == height {
            let acts = self.engine.on_commit(&commit, &payload);
            self.exec(acts, out);
        } else if self.engine.app_mut().apply(d.clone()).is_ok() {
            let _ = persist(&mut self.blocks_file, &d);
            self.pending_since = None;
            self.future.retain(|(h, _)| *h > height);
        }
    }

    pub(crate) fn tick(&mut self, out: &mut Vec<Msg>) {
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
}

/// Queues `msgs` for every other node (one bounded queue and sender per peer, see `Net::broadcast`).
pub(crate) fn send_all(net: &Arc<Net>, peers: &[PeerLink], msgs: Vec<Msg>) {
    let keys: Vec<Vec<u8>> = peers.iter().map(|p| p.key.clone()).collect();
    net.broadcast(&keys, msgs);
}

pub(crate) fn ack(ok: bool, info: &str, accepted: u32) -> Msg {
    wire::msg(
        wire::ACK,
        &Ack {
            ok,
            info: info.to_string(),
            accepted,
        },
    )
}

impl<A: LaneApp> Lane<A> {
    /// Learns the addresses of nodes appended to `peers.txt` since the start (a rotation adds a node;
    /// the existing positions never change). Cheap: reads the file every couple of seconds.
    fn refresh_peers(&mut self) {
        if self.last_peers_read.elapsed() < Duration::from_secs(2) {
            return;
        }
        self.last_peers_read = Instant::now();
        let Some(text) = self
            .peers_file
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
        else {
            return;
        };
        let Ok(listed) = crate::setup::parse_peers(&text) else {
            return;
        };
        let links: Vec<PeerLink> = listed.iter().map(|(_, l)| l.clone()).collect();
        self.net.learn(&links);
        for (i, l) in listed {
            if i == self.peers.len() {
                self.peers.push(l);
            }
        }
    }
}

/// The thread body: ticks timers, starts heights, resends, and syncs from peers.
pub(crate) fn run_loop<A: LaneApp>(stop: &AtomicBool, lane: &Mutex<Lane<A>>) {
    while !stop.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(40));
        let mut out = Vec::new();
        let (net, peers, sync_now, from) = {
            let mut e = lock_lane(lane);
            e.refresh_peers();
            e.tick(&mut out);
            let sync_now = e.last_sync.elapsed() >= SYNC_EVERY;
            if sync_now {
                e.last_sync = Instant::now();
                // who may open a peer session follows the chain (rotations, stake changes)
                e.net.set_validators(A::NAME, e.app().peer_keys());
            }
            (e.net.clone(), e.peers.clone(), sync_now, e.height() + 1)
        };
        send_all(&net, &peers, out);
        if sync_now {
            sync_from_peers(lane, &net, &peers, from);
        }
    }
}

fn sync_from_peers<A: LaneApp>(
    lane: &Mutex<Lane<A>>,
    net: &Arc<Net>,
    peers: &[PeerLink],
    from: u64,
) {
    let kinds = lock_lane(lane).kinds;
    let mut online = 0u32;
    for p in peers {
        if p.key == net.me {
            continue;
        }
        let req = wire::msg(kinds.blocks, &GetBlocks { from, max: 100 });
        let Ok(r) = net.call(&p.key, Chan::Rpc, &req, SYNC_TIMEOUT) else {
            continue;
        };
        online += 1;
        let Ok(reply) = wire::decode::<BlocksReply>(&r) else {
            continue;
        };
        for raw in reply.blocks {
            let Ok(d) = A::Decided::decode(raw.as_slice()) else {
                break;
            };
            let mut out = Vec::new();
            {
                let mut e = lock_lane(lane);
                e.accept_decided(d, &mut out);
            }
            send_all(net, peers, out);
        }
    }
    lock_lane(lane).peers_online = online;
}

/// Handles the consensus wire messages of a lane (consensus, final blocks, block download).
/// Returns None for kinds that are not this lane's.
pub(crate) fn handle<A: LaneApp>(lane: &Mutex<Lane<A>>, req: &Msg) -> Option<Msg> {
    let kinds = lock_lane(lane).kinds;
    if req.kind == kinds.bft {
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
        let (net, peers) = {
            let mut e = lock_lane(lane);
            e.on_bft(m, &mut out);
            (e.net.clone(), e.peers.clone())
        };
        send_all(&net, &peers, out);
        return Some(ack(true, "ok", 0));
    }
    if req.kind == kinds.commit {
        let Ok(d) = wire::decode::<A::Decided>(req) else {
            return Some(ack(false, "malformed", 0));
        };
        let mut out = Vec::new();
        let (net, peers) = {
            let mut e = lock_lane(lane);
            e.accept_decided(d, &mut out);
            (e.net.clone(), e.peers.clone())
        };
        send_all(&net, &peers, out);
        return Some(ack(true, "ok", 0));
    }
    if req.kind == kinds.blocks {
        let Ok(g) = wire::decode::<GetBlocks>(req) else {
            return Some(ack(false, "malformed", 0));
        };
        let e = lock_lane(lane);
        let mut blocks = Vec::new();
        let mut bytes = 0usize;
        for raw in e
            .app()
            .blocks_from(g.from.max(1), g.max.clamp(1, 500) as usize)
        {
            if bytes + raw.len() > 8 * 1024 * 1024 && !blocks.is_empty() {
                break;
            }
            bytes += raw.len();
            blocks.push(raw);
        }
        return Some(wire::msg(
            wire::BLOCKS_REPLY,
            &BlocksReply {
                blocks,
                height: e.height(),
            },
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("teko-lane-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_wal_remembers_the_round_and_the_lock_across_a_restart() {
        let d = tmp("wal");
        let p = d.join("lane.wal");
        assert_eq!(read_wal(&p), None);
        write_wal(&p, 7, 0, None);
        assert_eq!(
            read_wal(&p),
            Some(WalEntry {
                height: 7,
                round: 0,
                lock: None
            })
        );
        write_wal(&p, 7, 2, Some((&[0xAB; 32], 1)));
        assert_eq!(
            read_wal(&p),
            Some(WalEntry {
                height: 7,
                round: 2,
                lock: Some((vec![0xAB; 32], 1))
            })
        );
        // a torn last line is ignored instead of invented
        std::fs::write(&p, "7 2 1 zz\n").unwrap();
        assert_eq!(read_wal(&p), None);
        let _ = std::fs::remove_dir_all(&d);
    }
}
