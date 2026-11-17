use std::collections::HashMap;
use std::fs;
use std::net::{SocketAddr, TcpListener, ToSocketAddrs};
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
use crate::net::{Advertise, Caller, Net};
use crate::setup::PeerLink;
use crate::wire::{self, Ack, Msg, SubmitEntries};

pub struct NodeConfig {
    /// Where `blocks.log` lives (this node's own folder).
    pub dir: PathBuf,
    pub profile: NetworkProfile,
    pub key: SigningKey,
    pub index: usize,
    /// Every node of `peers.txt`, by index (this node's own included): the keys that may open a peer
    /// session and where to dial them (ADR-0048).
    pub links: Vec<PeerLink>,
    /// Where the authenticated PEER port listens; `None` = the own link's `peer` address. Separate
    /// from the address the node ANNOUNCES (`advertise`), so a node behind a firewall or a proxy can
    /// listen on `0.0.0.0` and tell the others its public name.
    pub listen_peer: Option<SocketAddr>,
    /// `(peer, public)` addresses this node announces in its signed record; `None` = its own link.
    pub advertise: Option<(String, String)>,
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
    /// This node's 1-based index in THIS ceremony.
    pub index: u32,
    /// The consensus keys of the participants, in ceremony order (position + 1 = ceremony index).
    pub participants: Vec<Vec<u8>>,
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
    /// Contests whose signed OUTCOME this validator already published (ADR-0040).
    outcomes_sent: HashMap<String, Instant>,
    /// When this validator last published its share of the draw of a tied contest (ADR-0043).
    draws_sent: HashMap<String, Instant>,
}

pub(crate) struct Shared {
    state: Mutex<State>,
    pub(crate) cfg: NodeConfig,
    pub(crate) stop: AtomicBool,
    /// The vote chain: bulletin entries ordered by BFT among the institutions (ADR-0034).
    pub(crate) votes: Mutex<crate::votes::Votes>,
    pub(crate) econ: Option<Mutex<crate::economy::Econ>>,
    /// Identity, peer sessions, discovery and limits (ADR-0048).
    pub(crate) net: Arc<Net>,
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
    /// Starts a node. `listen` is where the PUBLIC port listens (what clients dial); the peer port
    /// listens on `cfg.listen_peer` or on the own `peers.txt` link.
    pub fn start(cfg: NodeConfig, listen: SocketAddr) -> Result<NodeHandle, NodeError> {
        fs::create_dir_all(&cfg.dir)?;
        let me = public_key_bytes(&cfg.key);
        // a genesis institution holds its own slot; a node that joined LATER (a rotation, ADR-0045)
        // sits after them in `peers.txt` and becomes an institution only when the chain says so
        let genesis_slot = cfg.profile.validators.get(cfg.index);
        let joined_later = cfg.index >= cfg.profile.validators.len() && cfg.index < cfg.links.len();
        if genesis_slot != Some(&me) && !joined_later {
            return Err(NodeError::Refused(
                "this key is not validator `index` of the network".into(),
            ));
        }
        let own = cfg
            .links
            .get(cfg.index)
            .ok_or_else(|| NodeError::Refused("this node is not in peers.txt".into()))?;
        if own.key != me {
            return Err(NodeError::Refused(
                "peers.txt lists another key for this index".into(),
            ));
        }
        let peer_listen: SocketAddr = match cfg.listen_peer {
            Some(a) => a,
            None => own
                .peer
                .to_socket_addrs()?
                .next()
                .ok_or_else(|| NodeError::Refused("cannot resolve the peer address".into()))?,
        };
        // Loopback only unless asked: the PUBLIC port has no login, and a local test network has no
        // reason to be reachable from outside.
        if !cfg.allow_remote && !(listen.ip().is_loopback() && peer_listen.ip().is_loopback()) {
            return Err(NodeError::Refused(
                "the node only listens on loopback unless remote listening is enabled".into(),
            ));
        }
        let (adv_peer, adv_public) = cfg
            .advertise
            .clone()
            .unwrap_or_else(|| (own.peer.clone(), own.public.clone()));
        let genesis = crate::net::genesis_hash(&cfg.profile, cfg.economy.as_ref());
        let identity = teko_p2p::NetIdentity::load_or_create(
            cfg.key.clone(),
            &cfg.profile.chain_id,
            &genesis,
            &cfg.dir.join("noise.key"),
        )?;
        let net = Net::new(
            identity,
            &cfg.profile.chain_id,
            cfg.links.clone(),
            Advertise {
                peer: adv_peer,
                public: adv_public,
            },
            &cfg.dir,
        );
        let batch = Duration::from_millis(cfg.profile.block_interval_ms);
        let votes = crate::votes::open(
            &cfg.dir,
            &cfg.profile,
            &cfg.key,
            cfg.index,
            cfg.links.clone(),
            net.clone(),
        )?;
        net.set_validators("votes", crate::lane::LaneApp::peer_keys(votes.app()));
        let votes = Mutex::new(votes);
        let econ = match cfg.economy.as_ref() {
            Some(ec) => {
                let e = crate::economy::open(
                    &cfg.dir,
                    ec,
                    &cfg.key,
                    cfg.index,
                    cfg.links.clone(),
                    net.clone(),
                    batch,
                )?;
                net.set_validators("economy", crate::lane::LaneApp::peer_keys(e.app()));
                Some(Mutex::new(e))
            }
            None => None,
        };
        let peer_door = TcpListener::bind(peer_listen)?;
        peer_door.set_nonblocking(true)?;
        let public_door = TcpListener::bind(listen)?;
        public_door.set_nonblocking(true)?;
        let addr = public_door.local_addr()?;
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                tally: crate::dkg::load_share(&cfg.dir),
                dkg: None,
                tallier: None,
                fed: 0,
                results_sent: HashMap::new(),
                outcomes_sent: HashMap::new(),
                draws_sent: HashMap::new(),
            }),
            cfg,
            stop: AtomicBool::new(false),
            votes,
            econ,
            net,
        });
        let mut threads = Vec::new();
        let s = shared.clone();
        threads.push(thread::spawn(move || {
            crate::serve::peer_accept_loop(&s, &peer_door);
        }));
        let s = shared.clone();
        threads.push(thread::spawn(move || {
            crate::serve::public_accept_loop(&s, &public_door);
        }));
        let s = shared.clone();
        threads.push(thread::spawn(move || s.net.discovery_loop()));
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
        self.shared.net.stop.store(true, Ordering::SeqCst);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
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

/// Answers one request. The caller (a client on the public port, or a validator that proved its
/// key) has already been allowed this KIND; what the handler adds is per-caller checks of its own
/// (the key ceremony).
pub(crate) fn handle(s: &Arc<Shared>, req: &Msg, caller: &Caller<'_>) -> Msg {
    if let Some(r) = crate::economy::handle(s, req) {
        return r;
    }
    if let Some(r) = crate::votes::handle(s, req) {
        return r;
    }
    let Caller::Peer(caller_key) = caller else {
        return ack(false, "unknown message", 0);
    };
    match req.kind {
        wire::PEERS => match wire::decode::<wire::PeerRecords>(req) {
            Ok(m) => {
                s.net.merge_records(m.records);
                wire::msg(
                    wire::PEERS_REPLY,
                    &wire::PeerRecords {
                        records: s.net.records_to_share(),
                    },
                )
            }
            Err(_) => ack(false, "malformed", 0),
        },
        wire::DKG_START => match wire::decode::<wire::DkgStart>(req) {
            Ok(m) => {
                let mut st = lock(s);
                crate::dkg::dkg_msg(crate::dkg::start(&mut st.dkg, s.cfg.index, &s.net.me, &m))
            }
            Err(_) => ack(false, "malformed", 0),
        },
        wire::DKG_SHARE => match wire::decode::<wire::DkgShareReq>(req) {
            Ok(m) => {
                crate::dkg::dkg_msg(crate::dkg::share_for(&lock(s).dkg, m.requester, caller_key))
            }
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
    let result = finish(&st, &s.net, m);
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
        publish_draws(s, interval);
        publish_outcomes(s, interval);
        punish_vote_equivocations(s);
    }
}

/// Equivocation the vote lane detected becomes a `SlashVotes` in the economy, where the
/// institution's bond is (ADR-0035). Without an economy lane there is nothing to burn.
fn punish_vote_equivocations(s: &Arc<Shared>) {
    let evidence = lock_lane(&s.votes).app_mut().take_evidence();
    let Some(econ) = s.econ.as_ref().filter(|_| !evidence.is_empty()) else {
        return;
    };
    let (msgs, net, peers) = {
        let mut e = lock_lane(econ);
        let msgs = crate::economy::punish_vote_equivocations(&mut e, &s.cfg.key, &evidence);
        (msgs, e.net.clone(), e.peers.clone())
    };
    lane::send_all(&net, &peers, msgs);
}

/// The OUTCOME service (ADR-0040): once a contest is CLOSED and its totals are final (revealed, if
/// encrypted), this validator decides the result by the pure rule from ITS OWN verified tally,
/// signs it with its consensus key and publishes the signature on the board. The vote chain does
/// not judge it: what makes it count is that more than 2/3 of the institutions sign the SAME bytes.
fn publish_outcomes(s: &Arc<Shared>, interval: Duration) {
    use teko_election::{SignedOutcome, contest_hash, decide_with, sign_outcome};
    let list: Vec<(String, Vec<u8>)> = {
        let st = lock(s);
        let Some(t) = st.tallier.as_ref() else {
            return;
        };
        let Some(m) = t.manifest() else {
            return;
        };
        let snap = t.snapshot();
        let retry = interval * 40;
        let mut out = Vec::new();
        for c in &snap.contests {
            if st
                .outcomes_sent
                .get(&c.contest_id)
                .is_some_and(|at| at.elapsed() <= retry)
            {
                continue;
            }
            let Some(contest) = m.contest(&c.contest_id) else {
                continue;
            };
            let h = contest_hash(contest);
            // a tie the rule cannot break waits for the seed of the verifiable draw (ADR-0043)
            if t.awaits_draw(&c.contest_id) {
                continue;
            }
            let seed = t.draw_seed(&c.contest_id);
            if let Some(o) = decide_with(
                &s.cfg.profile.chain_id,
                &m.election_id,
                contest,
                &h,
                c,
                seed.as_ref(),
            ) {
                let sig = sign_outcome(&s.cfg.key, &o);
                out.push((
                    c.contest_id.clone(),
                    prost::Message::encode_to_vec(&SignedOutcome::new(&o, &sig)),
                ));
            }
        }
        out
    };
    for (contest_id, payload) in list {
        let body = SubmitEntries {
            entries: vec![teko_chain::BlockEntry {
                kind: u32::from(EntryKind::OUTCOME.0),
                payload,
            }],
        };
        lock(s).outcomes_sent.insert(contest_id, Instant::now());
        let _ = crate::votes::submit(s, &body, true);
    }
}

/// The DRAW service (ADR-0043): when a contest of this node's own verified replica is CLOSED,
/// revealed and really TIED, this validator publishes its share of the verifiable draw (`d_i * H`
/// with a proof against its verification key). Nothing is published for a contest without a tie, and
/// the shares alone reveal nothing: only `threshold` of them together give the seed.
fn publish_draws(s: &Arc<Shared>, interval: Duration) {
    use teko_ballot_crypto::draw::{draw_share, hash_to_point};
    use teko_election::draw::{DrawShareEntry, draw_ctx, draw_input};
    let list = {
        let st = lock(s);
        let (Some(t), Some(ks)) = (st.tallier.as_ref(), st.tally.as_ref()) else {
            return;
        };
        let Some(m) = t.manifest() else {
            return;
        };
        let retry = interval * 20;
        t.pending_draws(ks.index)
            .into_iter()
            .filter(|(c, _, _)| st.draws_sent.get(c).is_none_or(|at| at.elapsed() > retry))
            .map(|x| (m.election_id.clone(), x))
            .collect::<Vec<_>>()
    };
    for (election_id, (contest_id, hash, need)) in list {
        let (index, secret) = {
            let st = lock(s);
            match st.tally.as_ref() {
                Some(k) => (k.index, k.secret),
                None => continue,
            }
        };
        let input = draw_input(&election_id, &contest_id, &hash, &need);
        let Some(h) = hash_to_point(&input) else {
            continue;
        };
        let ctx = draw_ctx(&election_id, &contest_id, &input, index);
        let mut rng = rand::rand_core::UnwrapErr(rand::rngs::SysRng);
        let (share, proof) = draw_share(&secret, &h, &ctx, &mut rng);
        let entry = DrawShareEntry {
            election_id,
            contest_id: contest_id.clone(),
            contest_hash: hash.to_vec(),
            input,
            validator_index: index,
            share: teko_ballot_crypto::point_to_bytes(&share).to_vec(),
            proof: proof.to_bytes().to_vec(),
        };
        let body = SubmitEntries {
            entries: vec![teko_chain::BlockEntry {
                kind: u32::from(EntryKind::DRAW.0),
                payload: prost::Message::encode_to_vec(&entry),
            }],
        };
        lock(s).draws_sent.insert(contest_id, Instant::now());
        let _ = crate::votes::submit(s, &body, true);
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
                    EntryKind::DRAW => Some(LogKind::Draw),
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
