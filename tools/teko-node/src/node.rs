use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use prost::Message;
use teko_bulletin::{BulletinBoard, Entry, EntryKind};
use teko_chain::{Block, Chain, NetworkProfile, block_hash};
use teko_crypto::{SigningKey, public_key_bytes};

use crate::client::call;
use crate::error::NodeError;
use crate::wire::{
    self, Ack, BlockMsg, BlocksReply, GetBlocks, Msg, SigReply, StatusReply, SubmitEntries,
};

const PEER_TIMEOUT: Duration = Duration::from_millis(1_500);
const MAX_MEMPOOL: usize = 50_000;
const MAX_PER_BLOCK: usize = 1_000;

pub struct NodeConfig {
    /// Where `blocks.log` lives (this node's own folder).
    pub dir: PathBuf,
    pub profile: NetworkProfile,
    pub key: SigningKey,
    pub index: usize,
    /// Every validator's address, by index (this node's own included).
    pub peers: Vec<SocketAddr>,
    /// Loopback only unless this is set (never on by default).
    pub allow_remote: bool,
    /// Public trust anchors (`trust.txt`): governance keys and the network the
    /// election is bound to. Without them the node still produces blocks but does NOT
    /// decrypt results (it cannot tell which manifests are legitimate).
    pub trust: Option<teko_attestation::TrustPolicy>,
}

/// The key ceremony in progress at this node (secret polynomial).
pub(crate) struct DkgState {
    pub participant: teko_ballot_crypto::dkg::Participant,
    pub session: Vec<u8>,
    pub n: u32,
    pub threshold: usize,
}

struct State {
    /// This validator's share of the election key (after the ceremony).
    tally: Option<teko_ballot_crypto::dkg::KeyShare>,
    dkg: Option<DkgState>,
    chain: Chain,
    mempool: Vec<Entry>,
    seen: HashSet<[u8; 32]>,
    /// height -> (round, header hash) this validator already signed.
    signed: HashMap<u64, (u32, [u8; 32])>,
    equivocations: Vec<(u64, [u8; 32], [u8; 32])>,
    blocks_file: File,
    /// When the current batch of pending entries started waiting (drives view change).
    pending_since: Option<Instant>,
    last_commit: Instant,
    last_proposal: Option<(u64, u32, Instant)>,
    peers_online: u32,
    /// This node's view of the election, rebuilt from its verified chain (feeds the result service).
    tallier: Option<teko_election::Tallier>,
    fed: u64,
    /// contest id -> when this node published its decryption shares.
    results_sent: HashMap<String, Instant>,
}

struct Shared {
    state: Mutex<State>,
    cfg: NodeConfig,
    me: Vec<u8>,
    stop: AtomicBool,
}

pub struct Node;

pub struct NodeHandle {
    shared: Arc<Shared>,
    threads: Vec<JoinHandle<()>>,
    pub addr: SocketAddr,
}

fn lock(s: &Shared) -> MutexGuard<'_, State> {
    s.state.lock().unwrap_or_else(|e| e.into_inner())
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn persist(file: &mut File, b: &Block) -> Result<(), NodeError> {
    let bytes = b.encode_to_vec();
    file.write_all(&(bytes.len() as u32).to_be_bytes())?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(())
}

fn load_blocks(path: &PathBuf, chain: &mut Chain) -> Result<(), NodeError> {
    let Ok(mut f) = File::open(path) else {
        return Ok(());
    };
    let mut data = Vec::new();
    f.read_to_end(&mut data)?;
    let mut at = 0usize;
    while at + 4 <= data.len() {
        let n = u32::from_be_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]]) as usize;
        let end = at + 4 + n;
        if end > data.len() {
            break; // torn last write: ignore the tail
        }
        let b = Block::decode(&data[at + 4..end])
            .map_err(|_| NodeError::Protocol("corrupt blocks.log".into()))?;
        chain.apply(b)?; // every block is RE-VERIFIED at startup
        at = end;
    }
    Ok(())
}

impl Node {
    pub fn start(cfg: NodeConfig, listen: SocketAddr) -> Result<NodeHandle, NodeError> {
        if !cfg.allow_remote && !listen.ip().is_loopback() {
            return Err(NodeError::Refused(
                "the node only listens on loopback (this is a local test network)".into(),
            ));
        }
        fs::create_dir_all(&cfg.dir)?;
        let path = cfg.dir.join("blocks.log");
        let mut chain = Chain::new(cfg.profile.clone());
        load_blocks(&path, &mut chain)?;
        let mut seen = HashSet::new();
        for i in 0..chain.board().len() {
            if let Some(e) = chain.board().entry(i) {
                seen.insert(e.leaf_hash());
            }
        }
        let blocks_file = OpenOptions::new().create(true).append(true).open(&path)?;
        let me = public_key_bytes(&cfg.key);
        if cfg.profile.validators.get(cfg.index) != Some(&me) {
            return Err(NodeError::Refused(
                "this key is not validator `index` of the network".into(),
            ));
        }
        let listener = TcpListener::bind(listen)?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                tally: crate::dkg::load_share(&cfg.dir),
                dkg: None,
                chain,
                mempool: Vec::new(),
                seen,
                signed: HashMap::new(),
                equivocations: Vec::new(),
                blocks_file,
                pending_since: None,
                last_commit: Instant::now(),
                last_proposal: None,
                peers_online: 0,
                tallier: None,
                fed: 0,
                results_sent: HashMap::new(),
            }),
            cfg,
            me,
            stop: AtomicBool::new(false),
        });
        let mut threads = Vec::new();
        let s = shared.clone();
        threads.push(thread::spawn(move || accept_loop(&s, &listener)));
        let s = shared.clone();
        threads.push(thread::spawn(move || producer_loop(&s)));
        Ok(NodeHandle {
            shared,
            threads,
            addr,
        })
    }
}

impl NodeHandle {
    pub fn height(&self) -> u64 {
        lock(&self.shared).chain.height()
    }

    pub fn stop(mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

fn accept_loop(s: &Arc<Shared>, listener: &TcpListener) {
    while !s.stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _)) => {
                let s2 = s.clone();
                thread::spawn(move || {
                    let _ = serve(&s2, stream);
                });
            }
            Err(_) => thread::sleep(Duration::from_millis(15)),
        }
    }
}

fn serve(s: &Arc<Shared>, mut stream: TcpStream) -> Result<(), NodeError> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    let req = wire::read_msg(&mut stream)?;
    let reply = handle(s, &req);
    wire::write_msg(&mut stream, &reply)
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

fn handle(s: &Arc<Shared>, req: &Msg) -> Msg {
    match req.kind {
        wire::SUBMIT | wire::FORWARD => match wire::decode::<SubmitEntries>(req) {
            Ok(b) => submit(s, &b, req.kind == wire::SUBMIT),
            Err(_) => ack(false, "malformed", 0),
        },
        wire::STATUS => status(s),
        wire::GET_BLOCKS => match wire::decode::<GetBlocks>(req) {
            Ok(g) => get_blocks(s, &g),
            Err(_) => ack(false, "malformed", 0),
        },
        wire::PROPOSAL => match wire::decode::<BlockMsg>(req) {
            Ok(b) => sign_proposal(s, &b),
            Err(_) => ack(false, "malformed", 0),
        },
        wire::COMMIT => match wire::decode::<BlockMsg>(req) {
            Ok(b) => commit(s, &b),
            Err(_) => ack(false, "malformed", 0),
        },
        wire::DKG_START => match wire::decode::<wire::DkgStart>(req) {
            Ok(m) => {
                let mut st = lock(s);
                crate::dkg::dkg_msg(crate::dkg::start(&mut st.dkg, s.cfg.index, &m))
            }
            Err(_) => ack(false, "malformed", 0),
        },
        wire::DKG_SHARE => match wire::decode::<wire::DkgShareReq>(req) {
            Ok(m) => crate::dkg::dkg_msg(crate::dkg::share_for(&lock(s).dkg, m.requester)),
            Err(_) => ack(false, "malformed", 0),
        },
        wire::DKG_PACKAGES => match wire::decode::<wire::DkgPackages>(req) {
            Ok(m) => dkg_finish(s, &m),
            Err(_) => ack(false, "malformed", 0),
        },
        _ => ack(false, "unknown message", 0),
    }
}

/// Round 2. The state lock is NOT held while talking to the peers.
fn dkg_finish(s: &Arc<Shared>, m: &wire::DkgPackages) -> Msg {
    use crate::dkg::{dkg_msg, finish, reply, save_share};
    // Take the ceremony state out, work without the lock, put it back.
    let Some(st) = lock(s).dkg.take() else {
        return dkg_msg(reply(false, "no ceremony"));
    };
    let result = finish(&st, s.cfg.index, &s.cfg.peers, m);
    let mut guard = lock(s);
    match result {
        Ok(ks) => {
            if save_share(&s.cfg.dir, &ks).is_err() {
                guard.dkg = Some(st);
                return dkg_msg(reply(false, "could not save the share"));
            }
            let mut out = reply(true, "done");
            out.group_key = teko_ballot_crypto::point_to_bytes(&ks.group_public_key).to_vec();
            guard.tally = Some(ks);
            // keep serving shares to slower peers until they finish; the state stays
            guard.dkg = Some(st);
            dkg_msg(out)
        }
        Err(e) => {
            guard.dkg = Some(st);
            dkg_msg(reply(false, &e))
        }
    }
}

fn submit(s: &Arc<Shared>, b: &SubmitEntries, from_client: bool) -> Msg {
    let mut entries = Vec::new();
    for e in &b.entries {
        match e.to_entry() {
            Ok(x) if x.payload.len() <= 1024 * 1024 => entries.push(x),
            _ => return ack(false, "invalid entry", 0),
        }
    }
    let mut accepted = 0u32;
    {
        let mut st = lock(s);
        if st.mempool.len() + entries.len() > MAX_MEMPOOL {
            return ack(false, "mempool full", 0);
        }
        for e in entries {
            if st.seen.insert(e.leaf_hash()) {
                st.mempool.push(e);
                accepted += 1;
            }
        }
        if st.pending_since.is_none() && !st.mempool.is_empty() {
            st.pending_since = Some(Instant::now());
        }
    }
    if from_client && accepted > 0 {
        // Gossip to the other validators (they do not re-forward).
        let m = wire::msg(wire::FORWARD, b);
        for (i, addr) in s.cfg.peers.iter().enumerate() {
            if i == s.cfg.index {
                continue;
            }
            let (m, addr) = (m.clone(), *addr);
            thread::spawn(move || {
                let _ = call(addr, &m, PEER_TIMEOUT);
            });
        }
    }
    ack(true, "queued", accepted)
}

fn status(s: &Arc<Shared>) -> Msg {
    let st = lock(s);
    wire::msg(
        wire::STATUS_REPLY,
        &StatusReply {
            chain_id: st.chain.profile().chain_id.clone(),
            height: st.chain.height(),
            tip_hash: st.chain.tip_hash().to_vec(),
            log_size: st.chain.board().len(),
            log_root: st.chain.board().root().to_vec(),
            mempool: st.mempool.len() as u32,
            peers_online: st.peers_online,
            peers_total: s.cfg.peers.len().saturating_sub(1) as u32,
            validator_index: s.cfg.index as u32,
            equivocations: st.equivocations.len() as u32,
        },
    )
}

fn get_blocks(s: &Arc<Shared>, g: &GetBlocks) -> Msg {
    let st = lock(s);
    let mut out = Vec::new();
    let mut bytes = 0usize;
    for b in st
        .chain
        .blocks_from(g.from)
        .iter()
        .take(g.max.clamp(1, 500) as usize)
    {
        let raw = b.encode_to_vec();
        if bytes + raw.len() > 8 * 1024 * 1024 && !out.is_empty() {
            break;
        }
        bytes += raw.len();
        out.push(raw);
    }
    wire::msg(
        wire::BLOCKS_REPLY,
        &BlocksReply {
            blocks: out,
            height: st.chain.height(),
        },
    )
}

/// A validator signs a block ONLY if it fully checks (`Chain::sign`) and it has
/// not already signed a block of an equal or higher round for that height.
fn sign_proposal(s: &Arc<Shared>, b: &BlockMsg) -> Msg {
    let reply = |sig: Option<teko_attestation::SignerSignature>, why: &str| {
        wire::msg(
            wire::SIG_REPLY,
            &SigReply {
                signature: sig,
                reason: why.to_string(),
            },
        )
    };
    let Ok(block) = Block::decode(b.block.as_slice()) else {
        return reply(None, "malformed block");
    };
    let mut st = lock(s);
    let hash = block.header_hash();
    if let Some((round, h)) = st.signed.get(&block.height).copied()
        && h != hash
        && block.round <= round
    {
        return reply(None, "already signed another block for this height");
    }
    match st.chain.sign(&s.cfg.key, &block) {
        Ok(sig) => {
            st.signed.insert(block.height, (block.round, hash));
            reply(Some(sig), "")
        }
        Err(e) => reply(None, &e.to_string()),
    }
}

fn after_commit(st: &mut State, block: &Block) {
    for e in &block.entries {
        if let Ok(x) = e.to_entry() {
            st.seen.insert(x.leaf_hash());
        }
    }
    let included: HashSet<[u8; 32]> = block
        .entries
        .iter()
        .filter_map(|e| e.to_entry().ok().map(|x| x.leaf_hash()))
        .collect();
    st.mempool.retain(|e| !included.contains(&e.leaf_hash()));
    st.last_commit = Instant::now();
    st.pending_since = (!st.mempool.is_empty()).then(Instant::now);
    st.signed.retain(|h, _| *h > block.height);
}

fn commit(s: &Arc<Shared>, b: &BlockMsg) -> Msg {
    let Ok(block) = Block::decode(b.block.as_slice()) else {
        return ack(false, "malformed block", 0);
    };
    let mut st = lock(s);
    if block.height < st.chain.height() {
        // Already have this height. A DIFFERENT block with a full quorum is equivocation.
        let have = st.chain.blocks().get(block.height as usize).map(block_hash);
        if let Some(h) = have
            && h != block.header_hash()
            && st.chain.count_signatures(&block) >= st.chain.profile().threshold
            && !st
                .equivocations
                .iter()
                .any(|(ht, _, o)| *ht == block.height && *o == block.header_hash())
        {
            st.equivocations
                .push((block.height, h, block.header_hash()));
        }
        return ack(true, "already known", 0);
    }
    match st.chain.apply(block.clone()) {
        Ok(()) => {
            let State { blocks_file, .. } = &mut *st;
            if persist(blocks_file, &block).is_err() {
                return ack(false, "could not persist", 0);
            }
            after_commit(&mut st, &block);
            ack(true, "committed", 0)
        }
        Err(e) => ack(false, &e.to_string(), 0),
    }
}

fn producer_loop(s: &Arc<Shared>) {
    let interval = Duration::from_millis(s.cfg.profile.block_interval_ms);
    let tick = (interval / 4).max(Duration::from_millis(25));
    let mut last_sync = Instant::now();
    while !s.stop.load(Ordering::SeqCst) {
        thread::sleep(tick);
        if last_sync.elapsed() >= interval {
            last_sync = Instant::now();
            sync_from_peers(s);
        }
        try_propose(s, interval);
        decrypt_closed_contests(s, interval);
    }
}

/// The RESULT service: when this node's own verified replica shows an encrypted contest
/// CLOSED, publish this validator's decryption shares (with proofs) for the final sums.
/// Before the close nothing is computed, so honest software never helps open a vote early.
fn decrypt_closed_contests(s: &Arc<Shared>, interval: Duration) {
    use teko_election::{GovernancePolicy, LogEntry, LogKind, Tallier, build_shares};
    let Some(trust) = s.cfg.trust.as_ref() else {
        return;
    };
    let pending = {
        let mut st = lock(s);
        let Some(ks) = st.tally.as_ref().map(|k| k.index) else {
            return;
        };
        if st.tallier.is_none() {
            let Ok(policy) = GovernancePolicy::new(trust.gov_signers.clone(), trust.gov_threshold)
            else {
                return;
            };
            let mut t =
                Tallier::new(policy).with_verifier(Box::new(teko_registration::RsaRegistration));
            if !trust.chain_id.is_empty() {
                t = t.with_chain_id(trust.chain_id.clone());
            }
            st.tallier = Some(t);
        }
        // Feed what the chain gained since last time.
        let State {
            chain,
            tallier,
            fed,
            ..
        } = &mut *st;
        let board = chain.board();
        let Some(t) = tallier.as_mut() else {
            return;
        };
        while *fed < board.len() {
            if let Some(e) = board.entry(*fed) {
                let kind = match e.kind {
                    EntryKind::MANIFEST => Some(LogKind::Manifest),
                    EntryKind::TRANSITION => Some(LogKind::Transition),
                    EntryKind::REGISTRATION => Some(LogKind::Registration),
                    EntryKind::BALLOT => Some(LogKind::Ballot),
                    EntryKind::DKG => Some(LogKind::Dkg),
                    EntryKind::RESULT => Some(LogKind::Result),
                    _ => None,
                };
                match kind {
                    Some(kind) => t.feed(LogEntry {
                        kind,
                        payload: &e.payload,
                    }),
                    None => t.skip(),
                }
            }
            *fed += 1;
        }
        let list = t.pending_decryptions(ks);
        let m = t.manifest().map(|m| m.election_id.clone());
        let sent = &st.results_sent;
        let retry = interval * 20;
        list.into_iter()
            .filter(|(c, _, _)| sent.get(c).is_none_or(|at| at.elapsed() > retry))
            .map(|x| (m.clone(), x))
            .collect::<Vec<_>>()
    };
    for (election_id, (contest_id, hash, sums)) in pending {
        let Some(election_id) = election_id else {
            continue;
        };
        // Heavy work (proofs) outside the lock.
        let (index, secret) = {
            let st = lock(s);
            match st.tally.as_ref() {
                Some(k) => (k.index, k.secret),
                None => continue,
            }
        };
        let mut rng = rand::rand_core::UnwrapErr(rand::rngs::SysRng);
        let r = build_shares(
            &election_id,
            &contest_id,
            &hash,
            &sums,
            index,
            &secret,
            &mut rng,
        );
        let body = SubmitEntries {
            entries: vec![teko_chain::BlockEntry {
                kind: u32::from(EntryKind::RESULT.0),
                payload: prost::Message::encode_to_vec(&r),
            }],
        };
        lock(s).results_sent.insert(contest_id, Instant::now());
        let _ = submit(s, &body, true);
    }
}

/// Catch up from any peer that is ahead (every block re-verified by `apply`).
fn sync_from_peers(s: &Arc<Shared>) {
    let mut online = 0u32;
    for (i, addr) in s.cfg.peers.iter().enumerate() {
        if i == s.cfg.index {
            continue;
        }
        let from = lock(s).chain.height();
        let req = wire::msg(wire::GET_BLOCKS, &GetBlocks { from, max: 200 });
        let Ok(r) = call(*addr, &req, PEER_TIMEOUT) else {
            continue;
        };
        online += 1;
        let Ok(reply) = wire::decode::<BlocksReply>(&r) else {
            continue;
        };
        for raw in reply.blocks {
            if let Ok(b) = Block::decode(raw.as_slice()) {
                let _ = commit(
                    s,
                    &BlockMsg {
                        block: b.encode_to_vec(),
                    },
                );
            }
        }
    }
    lock(s).peers_online = online;
}

fn try_propose(s: &Arc<Shared>, interval: Duration) {
    let block = {
        let mut st = lock(s);
        let Some(since) = st.pending_since else {
            return;
        };
        if st.mempool.is_empty() || st.last_commit.elapsed() < interval {
            return;
        }
        // View change: each `3 * interval` of waiting hands the turn to the next validator.
        let round = (since.elapsed().as_millis() / (3 * interval.as_millis().max(1))) as u32;
        let h = st.chain.height();
        if st.chain.proposer_for(h, round) != s.me.as_slice() {
            return;
        }
        if let Some((ph, pr, at)) = st.last_proposal
            && ph == h
            && pr == round
            && at.elapsed() < interval * 2
        {
            return;
        }
        st.last_proposal = Some((h, round, Instant::now()));
        let take: Vec<Entry> = st.mempool.iter().take(MAX_PER_BLOCK).cloned().collect();
        let mut b = st.chain.propose(&take, now_secs(), round);
        match st.chain.sign(&s.cfg.key, &b) {
            Ok(sig) => {
                st.signed.insert(h, (round, b.header_hash()));
                b.signatures.push(sig);
            }
            Err(_) => return,
        }
        b
    };
    let need = s.cfg.profile.threshold;
    let proposal = wire::msg(
        wire::PROPOSAL,
        &BlockMsg {
            block: block.encode_to_vec(),
        },
    );
    let mut block = block;
    for (i, addr) in s.cfg.peers.iter().enumerate() {
        if i == s.cfg.index || block.signatures.len() >= need {
            continue;
        }
        if let Ok(r) = call(*addr, &proposal, PEER_TIMEOUT)
            && let Ok(reply) = wire::decode::<SigReply>(&r)
            && let Some(sig) = reply.signature
        {
            block.signatures.push(sig);
        }
    }
    if block.signatures.len() < need {
        return; // not enough validators answered: try again later (or view change)
    }
    let raw = BlockMsg {
        block: block.encode_to_vec(),
    };
    let result = commit(s, &raw);
    if result.kind == wire::ACK && wire::decode::<Ack>(&result).is_ok_and(|a| a.ok) {
        let m = wire::msg(wire::COMMIT, &raw);
        for (i, addr) in s.cfg.peers.iter().enumerate() {
            if i != s.cfg.index {
                let (m, addr) = (m.clone(), *addr);
                thread::spawn(move || {
                    let _ = call(addr, &m, PEER_TIMEOUT);
                });
            }
        }
    }
}
