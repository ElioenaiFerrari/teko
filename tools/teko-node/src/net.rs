//! The network layer of a node (ADR-0048): who it talks to and how. Validators reach each other
//! through authenticated, encrypted Noise sessions (`teko-p2p`) on the PEER port; clients use the
//! PUBLIC port (plain, read and submit only, with limits). Addresses come from signed records that
//! the validators exchange, seeded by the `peers.txt` bootstrap list.

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{RecvTimeoutError, SyncSender, sync_channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use teko_p2p::{
    ConnLimiter, MergeOutcome, NetIdentity, PeerBook, PeerPolicy, PeerRecord, Reputation, Session,
    TokenBucket, connect,
};

use crate::error::NodeError;
use crate::setup::PeerLink;
use crate::wire::{self, Msg, PeerRecords};

pub(crate) const PEER_TIMEOUT: Duration = Duration::from_millis(1_500);
const CONNECT_TIMEOUT: Duration = Duration::from_millis(1_500);
/// A reply (a page of blocks) may be big; a request has its own, much smaller, caps per kind.
const MAX_REPLY: usize = 17 * 1024 * 1024;
const QUEUE: usize = 256;
/// A node's own record lives this long and is renewed well before it ends.
const RECORD_TTL: u64 = 86_400;
const DISCOVER_EVERY: Duration = Duration::from_secs(20);

/// Which connection of a peer a call uses: consensus gossip and request/response calls do not queue
/// behind each other (a slow block download never delays a vote).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum Chan {
    Gossip,
    Rpc,
}

/// Who is calling a handler.
pub(crate) enum Caller<'a> {
    /// A client on the public port: no identity.
    Public,
    /// A validator that proved its consensus key.
    Peer(&'a [u8]),
}

/// What each caller may ask for. The peer port serves every kind; the public port only reads and
/// submits things that authenticate themselves (signed entries and transactions).
pub(crate) fn allowed(kind: u32, caller: &Caller<'_>) -> bool {
    match caller {
        Caller::Public => matches!(
            kind,
            wire::SUBMIT
                | wire::STATUS
                | wire::GET_BLOCKS
                | wire::ECON_TX
                | wire::ECON_BLOCKS
                | wire::ECON_STATUS
        ),
        Caller::Peer(_) => matches!(
            kind,
            wire::SUBMIT
                | wire::FORWARD
                | wire::STATUS
                | wire::GET_BLOCKS
                | wire::COMMIT
                | wire::VOTE_BFT
                | wire::DKG_START
                | wire::DKG_PACKAGES
                | wire::DKG_SHARE
                | wire::ECON_TX
                | wire::ECON_TX_FWD
                | wire::ECON_BFT
                | wire::ECON_COMMIT
                | wire::ECON_BLOCKS
                | wire::ECON_STATUS
                | wire::PEERS
        ),
    }
}

/// The largest body a request of this kind may have, checked before it is read.
pub(crate) fn max_request(kind: u32) -> usize {
    match kind {
        wire::STATUS
        | wire::GET_BLOCKS
        | wire::ECON_BLOCKS
        | wire::ECON_STATUS
        | wire::DKG_SHARE => 4 * 1024,
        wire::VOTE_BFT | wire::ECON_BFT | wire::DKG_START | wire::DKG_PACKAGES | wire::PEERS => {
            1024 * 1024
        }
        // submissions and decided blocks: up to a block
        _ => 9 * 1024 * 1024,
    }
}

/// Names the network and its starting state: the profile (validators, threshold) and the economy
/// genesis. Two nodes that disagree on it refuse each other in the handshake.
pub(crate) fn genesis_hash(
    profile: &teko_chain::NetworkProfile,
    economy: Option<&crate::economy::EconomyConfig>,
) -> Vec<u8> {
    // the block interval is a local tuning knob, not part of what the network IS
    let mut parts: Vec<Vec<u8>> = vec![
        profile.chain_id.as_bytes().to_vec(),
        profile.kind.name().as_bytes().to_vec(),
        profile.threshold.to_string().into_bytes(),
    ];
    parts.extend(profile.validators.iter().cloned());
    parts.push(
        economy
            .map(|e| e.genesis.to_text())
            .unwrap_or_default()
            .into_bytes(),
    );
    let refs: Vec<&[u8]> = parts.iter().map(Vec::as_slice).collect();
    teko_crypto::tagged_hash("teko/v2/net-genesis", &refs).to_vec()
}

pub(crate) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// How a node announces itself.
pub(crate) struct Advertise {
    pub peer: String,
    pub public: String,
}

/// One reusable session to a peer (empty until the first call, or after a failure).
type Slot = Arc<Mutex<Option<Session>>>;

pub(crate) struct Net {
    pub(crate) identity: NetIdentity,
    pub(crate) me: Vec<u8>,
    chain_id: String,
    /// The validators in force per lane (votes, economy): who may open a peer session.
    sets: Mutex<HashMap<&'static str, HashSet<Vec<u8>>>>,
    /// `peers.txt`: where to start (never trusted over a newer signed record).
    bootstrap: Mutex<Vec<PeerLink>>,
    book: Mutex<PeerBook>,
    book_path: PathBuf,
    advertise: Advertise,
    slots: Mutex<HashMap<(Vec<u8>, Chan), Slot>>,
    senders: Mutex<HashMap<Vec<u8>, SyncSender<Msg>>>,
    pub(crate) peer_conns: ConnLimiter,
    pub(crate) public_conns: ConnLimiter,
    pub(crate) rep: Mutex<Reputation>,
    buckets: Mutex<HashMap<IpAddr, TokenBucket>>,
    pub(crate) stop: AtomicBool,
}

impl PeerPolicy for Net {
    fn accepts(&self, key: &[u8]) -> bool {
        self.is_validator(key)
    }
}

impl Net {
    pub(crate) fn new(
        identity: NetIdentity,
        chain_id: &str,
        bootstrap: Vec<PeerLink>,
        advertise: Advertise,
        dir: &Path,
    ) -> Arc<Net> {
        let book_path = dir.join("peers.book");
        Arc::new(Net {
            me: identity.consensus_key(),
            identity,
            chain_id: chain_id.to_string(),
            sets: Mutex::new(HashMap::new()),
            bootstrap: Mutex::new(bootstrap),
            book: Mutex::new(PeerBook::load(chain_id, &book_path)),
            book_path,
            advertise,
            slots: Mutex::new(HashMap::new()),
            senders: Mutex::new(HashMap::new()),
            // peers on a local network all share one IP: the per-IP cap is generous there
            peer_conns: ConnLimiter::new(256, 64, 4),
            public_conns: ConnLimiter::new(256, 32, usize::MAX),
            rep: Mutex::new(Reputation::new(
                50,
                Duration::from_secs(600),
                Duration::from_secs(120),
            )),
            buckets: Mutex::new(HashMap::new()),
            stop: AtomicBool::new(false),
        })
    }

    /// The validators a lane says are in force (current set and any change still pending).
    pub(crate) fn set_validators(&self, lane: &'static str, keys: Vec<Vec<u8>>) {
        lock(&self.sets).insert(lane, keys.into_iter().collect());
    }

    pub(crate) fn is_validator(&self, key: &[u8]) -> bool {
        lock(&self.sets).values().any(|s| s.contains(key))
    }

    /// Every validator key known (all lanes), without this node's own.
    pub(crate) fn validator_keys(&self) -> Vec<Vec<u8>> {
        let mut all: Vec<Vec<u8>> = lock(&self.sets)
            .values()
            .flat_map(|s| s.iter().cloned())
            .filter(|k| *k != self.me)
            .collect();
        all.sort();
        all.dedup();
        all
    }

    /// Adds `peers.txt` lines learned after the start (a rotation appends them).
    pub(crate) fn learn(&self, links: &[PeerLink]) {
        let mut b = lock(&self.bootstrap);
        for l in links {
            match b.iter_mut().find(|x| x.key == l.key) {
                Some(x) => *x = l.clone(),
                None => b.push(l.clone()),
            }
        }
    }

    /// Where to dial a validator: the newest signed record, else the bootstrap line.
    pub(crate) fn addr_of(&self, key: &[u8]) -> Option<String> {
        if let Some(r) = lock(&self.book).get(key)
            && r.expires_at > now_secs()
        {
            return Some(r.peer_addr.clone());
        }
        lock(&self.bootstrap)
            .iter()
            .find(|l| l.key == key)
            .map(|l| l.peer.clone())
    }

    fn dial(&self, key: &[u8]) -> Result<Session, NodeError> {
        let addr = self
            .addr_of(key)
            .ok_or_else(|| NodeError::Refused("no address for this validator".into()))?;
        let sock: SocketAddr = addr
            .to_socket_addrs()?
            .next()
            .ok_or(NodeError::Unreachable)?;
        let stream = TcpStream::connect_timeout(&sock, CONNECT_TIMEOUT)?;
        Ok(connect(stream, &self.identity, self, Some(key))?)
    }

    fn slot(&self, key: &[u8], chan: Chan) -> Slot {
        lock(&self.slots)
            .entry((key.to_vec(), chan))
            .or_default()
            .clone()
    }

    /// One request and its reply over the (reused) session to `key`. A dead session is replaced
    /// once; the caller sees the second failure.
    pub(crate) fn call(
        &self,
        key: &[u8],
        chan: Chan,
        m: &Msg,
        timeout: Duration,
    ) -> Result<Msg, NodeError> {
        let slot = self.slot(key, chan);
        let mut g = lock(&slot);
        let mut last = NodeError::Unreachable;
        for _ in 0..2 {
            if g.is_none() {
                *g = Some(self.dial(key)?);
            }
            let Some(sess) = g.as_mut() else { continue };
            let r = sess
                .send(m.kind, &m.body, timeout)
                .and_then(|()| sess.recv(&|_| MAX_REPLY, timeout));
            match r {
                Ok((kind, body)) => return Ok(Msg { kind, body }),
                Err(e) => {
                    *g = None;
                    last = e.into();
                }
            }
        }
        Err(last)
    }

    /// Queues `msgs` for every `target`: one sender thread per peer with a bounded queue (a dead or
    /// slow peer drops messages instead of piling up threads; the engine resends).
    pub(crate) fn broadcast(self: &Arc<Self>, targets: &[Vec<u8>], msgs: Vec<Msg>) {
        for key in targets {
            if *key == self.me {
                continue;
            }
            let tx = {
                let mut senders = lock(&self.senders);
                senders
                    .entry(key.clone())
                    .or_insert_with(|| {
                        let (tx, rx) = sync_channel::<Msg>(QUEUE);
                        let (net, key) = (self.clone(), key.clone());
                        thread::spawn(move || {
                            while !net.stop.load(Ordering::SeqCst) {
                                match rx.recv_timeout(Duration::from_millis(200)) {
                                    Ok(m) => {
                                        if net.call(&key, Chan::Gossip, &m, PEER_TIMEOUT).is_err() {
                                            // a peer that does not answer: forget what piled up
                                            while rx.try_recv().is_ok() {}
                                            thread::sleep(Duration::from_millis(300));
                                        }
                                    }
                                    Err(RecvTimeoutError::Timeout) => {}
                                    Err(RecvTimeoutError::Disconnected) => break,
                                }
                            }
                        });
                        tx
                    })
                    .clone()
            };
            for m in &msgs {
                let _ = tx.try_send(m.clone());
            }
        }
    }

    pub(crate) fn is_banned(&self, subject: &[u8]) -> bool {
        lock(&self.rep).is_banned(subject, Instant::now())
    }

    pub(crate) fn strike(&self, subject: &[u8], points: u32) -> bool {
        lock(&self.rep).strike(subject, points, Instant::now())
    }

    /// Bad behaviour from a remote IP. Loopback is never banned: on a local test network every node
    /// shares it, and one misbehaving local process must not lock out the others.
    pub(crate) fn strike_ip(&self, ip: IpAddr, points: u32) {
        if !ip.is_loopback() {
            self.strike(&Reputation::ip_subject(ip), points);
        }
    }

    pub(crate) fn is_banned_ip(&self, ip: IpAddr) -> bool {
        !ip.is_loopback() && self.is_banned(&Reputation::ip_subject(ip))
    }

    /// Takes this node's rate-limit token for a client IP of the public port.
    pub(crate) fn public_allows(&self, ip: IpAddr) -> bool {
        let now = Instant::now();
        let mut b = lock(&self.buckets);
        if b.len() > 4_096 {
            b.clear();
        }
        b.entry(ip)
            .or_insert_with(|| TokenBucket::new(600, 200, now))
            .try_take(1, now)
    }

    // ───────────────────────── discovery ─────────────────────────

    /// This node's own, freshly signed record.
    fn own_record(&self) -> PeerRecord {
        let now = now_secs();
        // the sequence is the signing time: it only goes up, across restarts too
        PeerRecord::sign(
            &self.identity.consensus,
            &self.chain_id,
            &self.advertise.peer,
            &self.advertise.public,
            now,
            now + RECORD_TTL,
        )
    }

    /// Merges records a peer sent; returns whether the book changed.
    pub(crate) fn merge_records(&self, records: Vec<PeerRecord>) -> bool {
        let now = now_secs();
        let mut changed = false;
        let mut b = lock(&self.book);
        for r in records.into_iter().take(256) {
            // our own record is ours to make
            if r.consensus_key == self.me {
                continue;
            }
            let out = b.merge(r, &|k| self.is_validator(k), now);
            changed |= matches!(out, MergeOutcome::Added | MergeOutcome::Updated);
        }
        if changed {
            let _ = b.save(&self.book_path);
        }
        changed
    }

    /// The records this node shares: its own and every live one it holds.
    pub(crate) fn records_to_share(&self) -> Vec<PeerRecord> {
        let mut out = vec![self.own_record()];
        out.extend(lock(&self.book).live(now_secs()));
        out
    }

    fn discover_once(&self) {
        let ours = wire::msg(
            wire::PEERS,
            &PeerRecords {
                records: self.records_to_share(),
            },
        );
        for key in self.validator_keys() {
            if self.stop.load(Ordering::SeqCst) {
                return;
            }
            if let Ok(r) = self.call(&key, Chan::Rpc, &ours, Duration::from_secs(3))
                && let Ok(theirs) = wire::decode::<PeerRecords>(&r)
            {
                self.merge_records(theirs.records);
            }
        }
        let now = now_secs();
        lock(&self.book).prune(&|k| self.is_validator(k), now);
    }

    /// The discovery thread: exchanges records with every validator now and then.
    pub(crate) fn discovery_loop(self: &Arc<Self>) {
        let mut last = Instant::now()
            .checked_sub(DISCOVER_EVERY)
            .unwrap_or_else(Instant::now);
        while !self.stop.load(Ordering::SeqCst) {
            if last.elapsed() >= DISCOVER_EVERY {
                last = Instant::now();
                self.discover_once();
            }
            thread::sleep(Duration::from_millis(200));
        }
    }
}

/// The genesis hash of the network in `dir` (what two nodes must agree on to talk): printed by
/// `teko-node genesis-hash` so institutions compare it out of band.
pub fn genesis_hash_of(dir: &Path) -> Result<Vec<u8>, NodeError> {
    let net = crate::setup::load_network(dir)?;
    let economy = crate::setup::load_economy(dir)?;
    Ok(genesis_hash(&net.profile, economy.as_ref()))
}

/// A validator's identity for TOOLS (the key ceremony, rotation, tests): a short-lived client of the
/// PEER port that proves a validator key found in a network folder, one fresh session per call.
pub struct PeerClient {
    identity: NetIdentity,
}

struct Only(Vec<u8>);

impl PeerPolicy for Only {
    fn accepts(&self, key: &[u8]) -> bool {
        self.0 == key
    }
}

impl PeerClient {
    /// Uses the first validator key of the folder (`node-<i>/validator.key`): on a testnet machine
    /// that is the institution's own.
    pub fn open(dir: &Path) -> Result<PeerClient, NodeError> {
        let net = crate::setup::load_network(dir)?;
        for i in 0..net.links.len() {
            if crate::setup::node_dir(dir, i)
                .join("validator.key")
                .exists()
            {
                return Self::open_as(dir, i);
            }
        }
        Err(NodeError::Refused(
            "no validator key in this folder to act as an institution".into(),
        ))
    }

    /// Acts as institution `index` (its key folder must be here).
    pub fn open_as(dir: &Path, index: usize) -> Result<PeerClient, NodeError> {
        let net = crate::setup::load_network(dir)?;
        let economy = crate::setup::load_economy(dir)?;
        let genesis = genesis_hash(&net.profile, economy.as_ref());
        let key = crate::setup::load_validator_key(dir, index)?;
        let identity = NetIdentity::load_or_create(
            key,
            &net.profile.chain_id,
            &genesis,
            &crate::setup::node_dir(dir, index).join("noise.key"),
        )?;
        Ok(PeerClient { identity })
    }

    /// One request to `link` over a fresh authenticated session.
    pub fn call(&self, link: &PeerLink, m: &Msg, timeout: Duration) -> Result<Msg, NodeError> {
        let sock: SocketAddr = link
            .peer
            .to_socket_addrs()?
            .next()
            .ok_or(NodeError::Unreachable)?;
        let stream = TcpStream::connect_timeout(&sock, CONNECT_TIMEOUT)?;
        let mut sess = connect(
            stream,
            &self.identity,
            &Only(link.key.clone()),
            Some(&link.key),
        )?;
        sess.send(m.kind, &m.body, timeout)?;
        let (kind, body) = sess.recv(&|_| MAX_REPLY, timeout)?;
        Ok(Msg { kind, body })
    }

    /// The consensus key this client proves.
    pub fn key(&self) -> Vec<u8> {
        self.identity.consensus_key()
    }
}
