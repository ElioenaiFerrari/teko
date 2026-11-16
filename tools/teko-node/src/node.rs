use std::collections::HashMap;
use std::fs;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use teko_bulletin::{BulletinBoard, EntryKind};
use teko_chain::NetworkProfile;
use teko_crypto::{SigningKey, public_key_bytes};

use crate::error::NodeError;
use crate::lane::{self, lock_lane};
use crate::wire::{self, Ack, Msg, SubmitEntries};

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
    /// The economy ledger on BFT (None = this node does not run it).
    pub economy: Option<crate::economy::EconomyConfig>,
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
    /// This node's view of the election, rebuilt from its verified chain (feeds the result service).
    tallier: Option<teko_election::Tallier>,
    fed: u64,
    /// contest id -> when this node published its decryption shares.
    results_sent: HashMap<String, Instant>,
}

pub(crate) struct Shared {
    state: Mutex<State>,
    pub(crate) cfg: NodeConfig,
    pub(crate) stop: AtomicBool,
    /// The vote chain: bulletin entries ordered by BFT among the institutions (ADR-0034).
    pub(crate) votes: Mutex<crate::votes::Votes>,
    pub(crate) econ: Option<Mutex<crate::economy::Econ>>,
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

impl Node {
    pub fn start(cfg: NodeConfig, listen: SocketAddr) -> Result<NodeHandle, NodeError> {
        if !cfg.allow_remote && !listen.ip().is_loopback() {
            return Err(NodeError::Refused(
                "the node only listens on loopback (this is a local test network)".into(),
            ));
        }
        fs::create_dir_all(&cfg.dir)?;
        let me = public_key_bytes(&cfg.key);
        if cfg.profile.validators.get(cfg.index) != Some(&me) {
            return Err(NodeError::Refused(
                "this key is not validator `index` of the network".into(),
            ));
        }
        let batch = Duration::from_millis(cfg.profile.block_interval_ms);
        let votes = Mutex::new(crate::votes::open(
            &cfg.dir,
            &cfg.profile,
            &cfg.key,
            cfg.index,
            cfg.peers.clone(),
        )?);
        let econ = match cfg.economy.as_ref() {
            Some(ec) => Some(Mutex::new(crate::economy::open(
                &cfg.dir,
                ec,
                &cfg.key,
                cfg.index,
                cfg.peers.clone(),
                batch,
            )?)),
            None => None,
        };
        let listener = TcpListener::bind(listen)?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                tally: crate::dkg::load_share(&cfg.dir),
                dkg: None,
                tallier: None,
                fed: 0,
                results_sent: HashMap::new(),
            }),
            cfg,
            stop: AtomicBool::new(false),
            votes,
            econ,
        });
        let mut threads = Vec::new();
        let s = shared.clone();
        threads.push(thread::spawn(move || accept_loop(&s, &listener)));
        let s = shared.clone();
        threads.push(thread::spawn(move || {
            lane::run_loop(&s.stop, &s.votes);
        }));
        let s = shared.clone();
        threads.push(thread::spawn(move || results_loop(&s)));
        if shared.econ.is_some() {
            let s = shared.clone();
            threads.push(thread::spawn(move || {
                if let Some(e) = s.econ.as_ref() {
                    lane::run_loop(&s.stop, e);
                }
            }));
        }
        Ok(NodeHandle {
            shared,
            threads,
            addr,
        })
    }
}

impl NodeHandle {
    /// Height of the vote chain.
    pub fn height(&self) -> u64 {
        lock_lane(&self.shared.votes).height()
    }

    /// Height of the economy chain (0 when this node does not run it).
    pub fn economy_height(&self) -> u64 {
        self.shared
            .econ
            .as_ref()
            .map_or(0, |e| lock_lane(e).height())
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
    if let Some(r) = crate::economy::handle(s, req) {
        return r;
    }
    if let Some(r) = crate::votes::handle(s, req) {
        return r;
    }
    match req.kind {
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

fn results_loop(s: &Arc<Shared>) {
    let interval = Duration::from_millis(s.cfg.profile.block_interval_ms);
    let tick = (interval / 4).max(Duration::from_millis(25));
    while !s.stop.load(Ordering::SeqCst) {
        thread::sleep(tick);
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
        // Feed what the vote chain gained since last time (the lane lock is taken AFTER the state
        // lock, never the other way round).
        let State { tallier, fed, .. } = &mut *st;
        let votes = lock_lane(&s.votes);
        let board = votes.app().chain.board();
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
        drop(votes);
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
        let _ = crate::votes::submit(s, &body, true);
    }
}
