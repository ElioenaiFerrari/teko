//! Election administration for the prototype: publish an election on the
//! bulletin board, append ballots, SIMULATE voters (clearly marked as simulated)
//! and tally. Plays the role of the board OPERATOR: in the prototype there is no
//! anonymous registration, so any well-formed key is accepted and the counts are
//! NOT trustworthy against fraud (ADR-0008/0018).

mod backend;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::{Duration, Instant};

pub use backend::{AnyBoard, NetBackend, NetBoard};
use prost::Message;
use teko_attestation::TrustPolicy;
use teko_bulletin::{BoardError, BulletinBoard, Entry, EntryKind, FileBoard, Receipt};
use teko_crypto::{public_key_bytes, tagged_hash, voter::derive_voter_key};
use teko_election::{
    BallotBody, ElectionBundle, ElectionTally, GovernancePolicy, LogEntry, LogKind, Phase,
    SubmittedBallot, SubmittedRegistration, Tallier, contest_hash, registration_message,
    sign_ballot, tally_with,
};
use teko_registration::{BlindSignature, IssuerKey, RsaRegistration, blind, finalize};
use thiserror::Error;

pub const BOARD_FILE: &str = "board.log";
/// Present iff synthetic ballots were added. Holds the next simulated voter index.
pub const SIM_MARKER: &str = "board.simulated";

#[derive(Debug, Error)]
pub enum AdminError {
    #[error("i/o: {0}")]
    Io(#[from] io::Error),
    #[error("board: {0}")]
    Board(#[from] BoardError),
    #[error("sem política de governança em trust.txt (rode `make setup`)")]
    NoGovernance,
    #[error("trust.txt inválido: {0}")]
    Trust(String),
    #[error("não encontrei election.bundle (rode `make setup`)")]
    NoBundle,
    #[error("rede: {0}")]
    Net(String),
    #[error("simulação: {0}")]
    Simulation(String),
    #[error("pacote de eleição malformado")]
    BadBundle,
    #[error("o quadro ainda não tem uma eleição publicada (rode `publish`)")]
    NotPublished,
}

/// Maps a board entry kind to what the tally understands; other kinds are skipped.
pub fn log_kind(k: EntryKind) -> Option<LogKind> {
    match k {
        EntryKind::MANIFEST => Some(LogKind::Manifest),
        EntryKind::TRANSITION => Some(LogKind::Transition),
        EntryKind::REGISTRATION => Some(LogKind::Registration),
        EntryKind::BALLOT => Some(LogKind::Ballot),
        _ => None,
    }
}

/// Feeds the entries the tallier has not seen yet (the live-view fast path).
/// Returns how many entries of the board were consumed in total.
pub fn feed_new(tallier: &mut Tallier, board: &impl BulletinBoard) -> u64 {
    let mut i = tallier.consumed();
    while i < board.len() {
        if let Some(e) = board.entry(i) {
            // Skipped kinds still advance the cursor via a no-op Ballot-less path.
            match log_kind(e.kind) {
                Some(kind) => tallier.feed(LogEntry {
                    kind,
                    payload: &e.payload,
                }),
                None => tallier.skip(),
            }
        }
        i += 1;
    }
    tallier.consumed()
}

/// A tallier that can check registration tokens (RFC 9474 blind RSA). Use this,
/// never a bare `Tallier::new`: without a verifier every contest that requires
/// registration rejects all ballots.
pub fn new_tallier(policy: &GovernancePolicy) -> Tallier {
    Tallier::new(policy.clone()).with_verifier(Box::new(RsaRegistration))
}

/// Full replay of a board.
pub fn tally_board(policy: &GovernancePolicy, board: &impl BulletinBoard) -> ElectionTally {
    let entries: Vec<Entry> = (0..board.len())
        .filter_map(|i| board.entry(i).cloned())
        .collect();
    tally_with(
        policy,
        Box::new(RsaRegistration),
        entries.iter().filter_map(|e| {
            log_kind(e.kind).map(|kind| LogEntry {
                kind,
                payload: &e.payload,
            })
        }),
    )
}

pub struct Admin {
    dir: PathBuf,
    policy: GovernancePolicy,
    /// Network of this election (from trust.txt); empty = unbound (dev).
    chain_id: String,
    /// A running local test network, if any (else the board is a local file).
    net: Option<NetBackend>,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct PublishReport {
    pub appended: u64,
    pub already_there: u64,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SimReport {
    pub ballots: u64,
    pub contests: u64,
}

// ───────────────────────── tiny deterministic PRNG (simulation only) ─────────────────────────

struct Prng(u64);

impl Prng {
    fn next(&mut self) -> u64 {
        // splitmix64
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Synthetic share of the national electorate per contest (NOT real data).
fn synthetic_weight(contest_id: &str) -> f64 {
    match contest_id {
        "presidente" => 1.0,
        "governador-sp" => 0.22,
        "governador-mg" => 0.10,
        "governador-rj" => 0.08,
        "governador-ba" => 0.07,
        "governador-rs" => 0.055,
        _ => 0.04,
    }
}

struct SimContest {
    id: String,
    hash: Vec<u8>,
    /// Cumulative popularity per option (skewed, shuffled per contest).
    cdf: Vec<f64>,
    weight: f64,
    /// Authority key of this contest, when it requires registration. SIMULATION:
    /// the simulator plays the authority for its synthetic voters.
    registration: Option<(IssuerKey, Vec<u8>)>,
}

impl Admin {
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, AdminError> {
        let dir = dir.into();
        let text = fs::read_to_string(dir.join("trust.txt"))?;
        let trust = TrustPolicy::parse(&text).map_err(|e| AdminError::Trust(e.to_string()))?;
        let policy = GovernancePolicy::new(trust.gov_signers, trust.gov_threshold)
            .map_err(|_| AdminError::NoGovernance)?;
        let net = NetBackend::detect(&dir);
        Ok(Self {
            dir,
            policy,
            chain_id: trust.chain_id,
            net,
        })
    }

    /// A tallier bound to this election's network.
    pub fn new_tallier(&self) -> Tallier {
        let t = new_tallier(&self.policy);
        if self.chain_id.is_empty() {
            t
        } else {
            t.with_chain_id(self.chain_id.clone())
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn policy(&self) -> &GovernancePolicy {
        &self.policy
    }

    /// The test network in use, if any.
    pub fn network(&self) -> Option<&NetBackend> {
        self.net.as_ref()
    }

    /// Whether a board exists to read (the network always does once nodes run).
    pub fn has_board(&self) -> bool {
        self.net.is_some() || self.board_path().exists()
    }

    /// The board to READ: a local file, or a replica of the network verified here.
    pub fn read_board(&self) -> Result<AnyBoard, AdminError> {
        match &self.net {
            Some(n) => n
                .read()
                .map(AnyBoard::Net)
                .map_err(|e| AdminError::Net(e.to_string())),
            None => Ok(AnyBoard::File(
                FileBoard::open_tolerant(self.board_path())?.0,
            )),
        }
    }

    /// Appends entries (file: one strict write; network: queued for the next block).
    fn append_entries(&self, entries: Vec<Entry>) -> Result<(), AdminError> {
        match &self.net {
            Some(n) => n
                .submit(&entries)
                .map(|_| ())
                .map_err(|e| AdminError::Net(e.to_string())),
            None => {
                self.open_for_write()?.append_many(entries)?;
                Ok(())
            }
        }
    }

    pub fn board_path(&self) -> PathBuf {
        self.dir.join(BOARD_FILE)
    }

    pub fn is_simulated(&self) -> bool {
        self.dir.join(SIM_MARKER).exists()
    }

    pub fn reset_board(&self) {
        if self.net.is_some() {
            return; // a blockchain is append-only: reset it with `make devnet-reset`
        }
        for f in [BOARD_FILE, SIM_MARKER] {
            let _ = fs::remove_file(self.dir.join(f));
        }
    }

    /// Strict open for writing; retries briefly if another writer is mid-append.
    fn open_for_write(&self) -> Result<FileBoard, AdminError> {
        let mut last = None;
        for _ in 0..8 {
            match FileBoard::open(self.board_path()) {
                Ok(b) => return Ok(b),
                Err(BoardError::Corrupt(_)) => {
                    sleep(Duration::from_millis(40));
                    last = Some(BoardError::Corrupt("another writer is mid-append"));
                }
                Err(e) => return Err(e.into()),
            }
        }
        Err(last.map_or(AdminError::NotPublished, AdminError::Board))
    }

    /// Appends `election.bundle` (manifest, then transitions) to the board,
    /// skipping entries that are already there. Idempotent.
    pub fn publish(&self) -> Result<PublishReport, AdminError> {
        let bytes = fs::read(self.dir.join("election.bundle")).map_err(|_| AdminError::NoBundle)?;
        let bundle = ElectionBundle::decode(bytes.as_slice()).map_err(|_| AdminError::BadBundle)?;
        let board = self.read_board()?;
        let existing: Vec<(EntryKind, Vec<u8>)> = (0..board.len())
            .filter_map(|i| board.entry(i).map(|e| (e.kind, e.payload.clone())))
            .collect();
        let mut new = Vec::new();
        let mut already = 0;
        let wanted = std::iter::once((EntryKind::MANIFEST, bundle.manifest)).chain(
            bundle
                .transitions
                .into_iter()
                .map(|t| (EntryKind::TRANSITION, t)),
        );
        for (kind, payload) in wanted {
            if existing.iter().any(|(k, p)| *k == kind && *p == payload) {
                already += 1;
            } else {
                new.push(Entry::new(kind, payload));
            }
        }
        let appended = new.len() as u64;
        drop(board);
        if !new.is_empty() {
            self.append_entries(new)?;
        }
        Ok(PublishReport {
            appended,
            already_there: already,
        })
    }

    /// SIMULATED governance: closes every contest whose deadline has passed (signed
    /// `VotingClosed`, appended to the bundle) and, if the board exists, publishes
    /// it. From then on the tally refuses further ballots for those contests.
    pub fn close_due(&self, now: u64) -> Result<usize, AdminError> {
        let n = issuer_cli::Issuer::open(&self.dir)
            .close_due(now)
            .map_err(|e| AdminError::Simulation(e.to_string()))?;
        if self.has_board() {
            self.publish()?;
        }
        Ok(n)
    }

    /// One entry; on the network it waits for the block that carries it, so the
    /// returned receipt says where it really landed.
    fn append_one(&self, e: Entry) -> Result<Receipt, AdminError> {
        match &self.net {
            Some(n) => n
                .submit_and_wait(&e)
                .map_err(|x| AdminError::Net(x.to_string())),
            None => Ok(self.open_for_write()?.append(e)?),
        }
    }

    /// Appends one ballot (what the Suite does after the device signs).
    pub fn submit_ballot(&self, ballot: &SubmittedBallot) -> Result<Receipt, AdminError> {
        self.append_one(Entry::new(EntryKind::BALLOT, ballot.encode_to_vec()))
    }

    /// Appends one registration: `(voter_key, token)` and NOTHING from the
    /// authority exchange (ADR-0008: separate channel, no link to the identity).
    pub fn submit_registration(&self, r: &SubmittedRegistration) -> Result<Receipt, AdminError> {
        self.append_one(Entry::new(EntryKind::REGISTRATION, r.encode_to_vec()))
    }

    /// Full tally of the current board (tolerates a writer mid-append).
    pub fn tally(&self) -> Result<ElectionTally, AdminError> {
        let board = self.read_board()?;
        if board.is_empty() {
            return Err(AdminError::NotPublished);
        }
        let mut t = self.new_tallier();
        feed_new(&mut t, &board);
        Ok(t.snapshot())
    }

    fn sim_contests(&self, seed: u64) -> Result<(String, Vec<SimContest>), AdminError> {
        let board = self.read_board()?;
        let mut t = self.new_tallier();
        feed_new(&mut t, &board);
        let manifest = t.manifest().cloned().ok_or(AdminError::NotPublished)?;
        let snap = t.snapshot();
        let mut out = Vec::new();
        for c in &manifest.contests {
            if snap.contest(&c.contest_id).map(|x| x.phase) != Some(Phase::VotingOpen) {
                continue;
            }
            let mut rng =
                Prng(seed ^ u64::from(c.contest_id.len() as u32).wrapping_mul(0x1234_5678_9ABC));
            for b in c.contest_id.bytes() {
                rng.0 = rng.0.wrapping_mul(31).wrapping_add(u64::from(b));
            }
            // popularity ~ 1/(rank+1)^1.2, ranks shuffled per contest
            let n = c.options.len();
            let mut ranks: Vec<usize> = (0..n).collect();
            for i in (1..n).rev() {
                ranks.swap(i, (rng.next() % (i as u64 + 1)) as usize);
            }
            let w: Vec<f64> = ranks
                .iter()
                .map(|r| 1.0 / ((*r as f64) + 1.0).powf(1.2))
                .collect();
            let total: f64 = w.iter().sum();
            let mut acc = 0.0;
            let cdf = w
                .iter()
                .map(|x| {
                    acc += x / total;
                    acc
                })
                .collect();
            let registration = if c.registration_key.is_empty() {
                None
            } else {
                let key = issuer_cli::Issuer::open(&self.dir)
                    .registration_key(&c.contest_id)
                    .map_err(|e| AdminError::Simulation(e.to_string()))?;
                Some((key, c.registration_key.clone()))
            };
            out.push(SimContest {
                id: c.contest_id.clone(),
                hash: contest_hash(c).to_vec(),
                cdf,
                weight: synthetic_weight(&c.contest_id),
                registration,
            });
        }
        Ok((manifest.election_id, out))
    }

    fn next_index(&self) -> u64 {
        fs::read_to_string(self.dir.join(SIM_MARKER))
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0)
    }

    /// The board entries ONE synthetic voter produces: a registration (when the
    /// contest requires it) and then the ballot. SIMULATION: the full blind-RSA
    /// flow runs for real, with the simulator playing the authority.
    fn voter_entries(
        election: &str,
        c: &SimContest,
        voter: u64,
        seed: u64,
        rng: &mut Prng,
    ) -> Vec<Entry> {
        let Some(ballot) = Self::make_ballot(election, c, voter, seed, rng) else {
            return Vec::new();
        };
        let mut out = Vec::with_capacity(2);
        if let Some((authority, spki)) = &c.registration {
            let msg = registration_message(election, &c.id, &ballot.voter_key);
            let token = blind(spki, &msg)
                .ok()
                .and_then(|b| {
                    let sig = authority.blind_sign(&b.blind_message).ok()?;
                    finalize(spki, &b.state, &BlindSignature(sig.0), &msg).ok()
                })
                .map(|t| t.to_bytes());
            let Some(token) = token else {
                return Vec::new();
            };
            out.push(Entry::new(
                EntryKind::REGISTRATION,
                SubmittedRegistration {
                    election_id: election.to_string(),
                    contest_id: c.id.clone(),
                    voter_key: ballot.voter_key.clone(),
                    token,
                }
                .encode_to_vec(),
            ));
        }
        out.push(Entry::new(EntryKind::BALLOT, ballot.encode_to_vec()));
        out
    }

    fn make_ballot(
        election: &str,
        c: &SimContest,
        voter: u64,
        seed: u64,
        rng: &mut Prng,
    ) -> Option<SubmittedBallot> {
        let root = tagged_hash(
            "teko/sim/voter-root",
            &[&seed.to_be_bytes(), &voter.to_be_bytes()],
        );
        let key = derive_voter_key(&root, election, &c.id).ok()?;
        let u = rng.unit();
        let option_index = c
            .cdf
            .iter()
            .position(|x| u <= *x)
            .unwrap_or(c.cdf.len() - 1) as u32;
        let body = BallotBody {
            election_id: election.to_string(),
            contest_id: c.id.clone(),
            contest_hash: c.hash.clone(),
            option_index,
        };
        Some(SubmittedBallot {
            election_id: body.election_id.clone(),
            contest_id: body.contest_id.clone(),
            contest_hash: body.contest_hash.clone(),
            option_index,
            voter_key: public_key_bytes(&key),
            signature: sign_ballot(&key, &body),
        })
    }

    /// Appends `voters` SYNTHETIC voters (President gets `voters`, each governor a
    /// synthetic share of it) in ONE batch, then marks the board as simulated.
    pub fn simulate_batch(&self, voters: u64, seed: u64) -> Result<SimReport, AdminError> {
        let (election, contests) = self.sim_contests(seed)?;
        let start = self.next_index();
        let mut rng = Prng(seed.wrapping_add(start));
        let mut entries = Vec::new();
        for (ci, c) in contests.iter().enumerate() {
            let n = (voters as f64 * c.weight).round() as u64;
            for i in 0..n {
                let voter = start + (ci as u64) * 1_000_000_000 + i;
                entries.extend(Self::voter_entries(&election, c, voter, seed, &mut rng));
            }
        }
        let ballots = entries
            .iter()
            .filter(|e| e.kind == EntryKind::BALLOT)
            .count() as u64;
        if !entries.is_empty() {
            self.append_entries(entries)?;
        }
        fs::write(
            self.dir.join(SIM_MARKER),
            (start + voters.max(1)).to_string(),
        )?;
        Ok(SimReport {
            ballots,
            contests: contests.len() as u64,
        })
    }

    /// Streams synthetic ballots at ~`rate` per second for `duration`, in small
    /// batches, so a live chart visibly moves. `on_tick(total_so_far)` per batch.
    pub fn simulate_live(
        &self,
        rate: u64,
        duration: Duration,
        seed: u64,
        mut on_tick: impl FnMut(u64),
    ) -> Result<SimReport, AdminError> {
        let (election, contests) = self.sim_contests(seed)?;
        let total_weight: f64 = contests.iter().map(|c| c.weight).sum();
        let start = self.next_index();
        fs::write(self.dir.join(SIM_MARKER), start.to_string())?;
        let mut rng = Prng(seed.wrapping_add(start));
        let tick = Duration::from_millis(250);
        let began = Instant::now();
        let mut sent = 0u64;
        let mut counter = start;
        while began.elapsed() < duration {
            let per_tick = ((rate as f64) * tick.as_secs_f64()).max(1.0) as u64;
            let mut entries = Vec::new();
            for _ in 0..per_tick {
                // pick a contest proportionally to its synthetic weight
                let mut x = rng.unit() * total_weight;
                let c = contests
                    .iter()
                    .find(|c| {
                        x -= c.weight;
                        x <= 0.0
                    })
                    .or(contests.last());
                let Some(c) = c else { break };
                counter += 1;
                let voter = counter + 7_000_000_000;
                entries.extend(Self::voter_entries(&election, c, voter, seed, &mut rng));
            }
            sent += entries
                .iter()
                .filter(|e| e.kind == EntryKind::BALLOT)
                .count() as u64;
            if !entries.is_empty() {
                self.append_entries(entries)?;
            }
            fs::write(self.dir.join(SIM_MARKER), counter.to_string())?;
            on_tick(sent);
            sleep(tick);
        }
        Ok(SimReport {
            ballots: sent,
            contests: contests.len() as u64,
        })
    }
}
