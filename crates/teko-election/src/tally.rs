//! Tally: counting ballots is a PURE FUNCTION of the bulletin-board log (ADR-0018).
//!
//! Anyone can replay the same entries and get the same numbers; nothing is taken
//! on trust. "Time" is the ORDER of the log, not a clock: a ballot counts only if
//! its contest was in `VotingOpen` at the point where the ballot was appended.
//!
//! PROTOTYPE LIMITS: there is no anonymous registration yet (ADR-0008), so any
//! well-formed key is accepted, and no ZK proof/nullifier (ADR-0013): the voter
//! key itself de-duplicates. The counts are NOT trustworthy against fraud.

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use prost::Message;

use teko_ballot_crypto::dkg_wire::{DkgRecord, VerifiedCeremony};
use teko_ballot_crypto::{
    BallotShape, Ciphertext, DecryptProof, EncryptedBallot, Point, combine_shares,
    point_from_bytes, solve_small_dlog, verify_decryption,
};

use teko_ballot_crypto::draw::{combine_draw, draw_seed, hash_to_point, verify_draw_share};

use crate::ballot::{BallotBody, ballot_ctx, ciphertext_hash, verify_ballot};
use crate::draw::{DrawNeed, DrawShareEntry, draw_ctx, draw_input};
use crate::eligibility::Scope;
use crate::lifecycle::{ElectionStatus, Phase, Transition, apply};
use crate::manifest::{
    ElectionManifest, Office, OptionKind, ResultsVisibility, contest_hash, validate_revision,
    verify_manifest,
};
use crate::outcome::needs_draw;
use crate::policy::GovernancePolicy;
use crate::registration::{RegistrationVerifier, SubmittedRegistration, registration_message};
use crate::result::{DecryptionShares, decrypt_ctx};

/// What a log entry carries. The bulletin board maps its own kind byte to this.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogKind {
    Manifest,
    Transition,
    Ballot,
    /// Anonymous registration token (ADR-0008).
    Registration,
    /// Key ceremony record (`DkgRecord`): the election key and the validators' verification keys.
    Dkg,
    /// A validator's decryption shares for a closed, encrypted contest.
    Result,
    /// A validator's share of the verifiable draw of a tie (ADR-0043).
    Draw,
}

#[derive(Clone, Copy, Debug)]
pub struct LogEntry<'a> {
    pub kind: LogKind,
    pub payload: &'a [u8],
}

/// A ballot as it is published on the board.
#[derive(Clone, PartialEq, Message)]
pub struct SubmittedBallot {
    #[prost(string, tag = "1")]
    pub election_id: String,
    #[prost(string, tag = "2")]
    pub contest_id: String,
    #[prost(bytes = "vec", tag = "3")]
    pub contest_hash: Vec<u8>,
    #[prost(uint32, tag = "4")]
    pub option_index: u32,
    /// PUBLIC per-contest voter key.
    #[prost(bytes = "vec", tag = "5")]
    pub voter_key: Vec<u8>,
    #[prost(bytes = "vec", tag = "6")]
    pub signature: Vec<u8>,
    /// Encrypted mode: the `EncryptedBallot` bytes (one-hot ElGamal + proofs). Then
    /// `option_index` is 0 and the signature covers `ciphertext_hash`.
    #[prost(bytes = "vec", tag = "7")]
    pub encrypted: Vec<u8>,
}

impl SubmittedBallot {
    pub fn body(&self) -> BallotBody {
        BallotBody {
            election_id: self.election_id.clone(),
            contest_id: self.contest_id.clone(),
            contest_hash: self.contest_hash.clone(),
            option_index: self.option_index,
            ciphertext_hash: if self.encrypted.is_empty() {
                Vec::new()
            } else {
                ciphertext_hash(&self.encrypted)
            },
        }
    }
}

/// What the tally did with one ballot (ADR-0041): it entered the count, or was refused for a reason
/// that is the same one the reject counters report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BallotVerdict {
    Counted,
    Rejected(&'static str),
}

/// Why ballots were NOT counted (they are counted as rejected, never dropped silently).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RejectCounts {
    pub malformed: u64,
    pub unknown_contest: u64,
    pub wrong_contest_version: u64,
    pub not_open: u64,
    pub unknown_option: u64,
    pub bad_signature: u64,
    pub duplicate: u64,
    /// Contest requires registration and this voter key has none.
    pub unregistered: u64,
    /// Encrypted ballot whose proofs/encoding do not verify (or wrong mode).
    pub bad_proof: u64,
}

impl RejectCounts {
    pub fn total(&self) -> u64 {
        self.malformed
            + self.unknown_contest
            + self.wrong_contest_version
            + self.not_open
            + self.unknown_option
            + self.bad_signature
            + self.duplicate
            + self.unregistered
            + self.bad_proof
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OptionTally {
    pub index: u32,
    pub name: String,
    pub number: String,
    pub votes: u64,
    /// Candidate, blank or null (ADR-0042).
    pub kind: OptionKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContestTally {
    pub contest_id: String,
    pub title: String,
    pub office: Office,
    pub scope: Scope,
    pub phase: Phase,
    /// Counted ballots.
    pub accepted: u64,
    /// Valid registrations seen (0 when the contest does not require it).
    pub registered: u64,
    /// Registrations refused (bad token, wrong phase, repeated key...).
    pub registrations_rejected: u64,
    /// Whether this contest requires registration to vote.
    pub registration_required: bool,
    /// Ballots are ENCRYPTED: `options[].votes` stays 0 until `revealed`.
    pub encrypted: bool,
    /// The totals were decrypted by enough validators and VERIFIED.
    pub revealed: bool,
    pub rejected: RejectCounts,
    /// In ballot order (NOT ranked); see `ranked`.
    pub options: Vec<OptionTally>,
    /// How many people the contest elects (1 unless it is a multi-seat senator contest, ADR-0044).
    pub seats: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ElectionTally {
    pub election_id: String,
    pub title: String,
    pub revision: u32,
    pub visibility: ResultsVisibility,
    /// Last governance sequence applied.
    pub seq: u64,
    pub contests: Vec<ContestTally>,
    /// Manifest/transition entries refused (bad signatures, illegal steps, ...).
    pub ignored_governance: u64,
    /// Ballots that arrived before any valid manifest existed.
    pub orphan_ballots: u64,
    /// Rejections with no contest to blame (malformed bytes, wrong election, unknown contest).
    pub unattributed: RejectCounts,
    /// Result/ceremony entries refused (bad proof, wrong key, too early, duplicate...).
    pub bad_results: u64,
    pub entries_seen: u64,
}

impl ElectionTally {
    pub fn contest(&self, id: &str) -> Option<&ContestTally> {
        self.contests.iter().find(|c| c.contest_id == id)
    }
}

/// Options sorted by votes (desc), ties by ballot position; `top` kept, the rest
/// folded into `others`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ranking {
    pub top: Vec<OptionTally>,
    pub others_votes: u64,
    pub others_count: usize,
}

impl ContestTally {
    /// `sum(candidates) + seats * (blank + null)`: the quantity every ballot contributes between 1
    /// and `seats` to (a ballot proves its WEIGHTED total is in `1..=seats`, ADR-0044). With one
    /// seat it is exactly the number of ballots.
    pub fn weighted_votes(&self) -> u64 {
        self.valid_votes() + self.seats.max(1) as u64 * (self.blank_votes() + self.null_votes())
    }

    /// Do the decrypted totals fit the ballots that were counted? Each ballot adds a weighted
    /// total in `1..=seats`, so the sum lies in `accepted..=seats * accepted`. A one-seat contest
    /// must add up to the ballots EXACTLY.
    pub fn totals_fit_the_ballots(&self) -> bool {
        let w = self.weighted_votes();
        let s = self.seats.max(1) as u64;
        w >= self.accepted && w <= s * self.accepted
    }

    /// Votes in CANDIDATES: the denominator of the majority (blank and null are outside it, ADR-0042).
    pub fn valid_votes(&self) -> u64 {
        self.options
            .iter()
            .filter(|o| o.kind == OptionKind::Candidate)
            .map(|o| o.votes)
            .sum()
    }

    pub fn blank_votes(&self) -> u64 {
        self.votes_of(OptionKind::Blank)
    }

    pub fn null_votes(&self) -> u64 {
        self.votes_of(OptionKind::Null)
    }

    fn votes_of(&self, k: OptionKind) -> u64 {
        self.options
            .iter()
            .filter(|o| o.kind == k)
            .map(|o| o.votes)
            .sum()
    }

    /// The ranking of the CANDIDATES only; blank and null are reported apart.
    pub fn ranked(&self, top_n: usize) -> Ranking {
        let mut all: Vec<OptionTally> = self
            .options
            .iter()
            .filter(|o| o.kind == OptionKind::Candidate)
            .cloned()
            .collect();
        all.sort_by(|a, b| b.votes.cmp(&a.votes).then(a.index.cmp(&b.index)));
        let rest = all.split_off(top_n.min(all.len()));
        Ranking {
            top: all,
            others_votes: rest.iter().map(|o| o.votes).sum(),
            others_count: rest.len(),
        }
    }

    /// What the PUBLIC may see under `visibility` (ADR-0018). Presentation only:
    /// with public ballots the log itself is countable by anyone.
    pub fn public_view(&self, visibility: ResultsVisibility) -> PublicView {
        let closed = matches!(
            self.phase,
            Phase::VotingClosed | Phase::Tallied | Phase::Audited
        );
        if self.phase == Phase::Cancelled {
            return PublicView {
                accepted: None,
                distribution: false,
            };
        }
        let mut v = match visibility {
            ResultsVisibility::Live => PublicView {
                accepted: Some(self.accepted),
                distribution: true,
            },
            ResultsVisibility::TurnoutOnly => PublicView {
                accepted: Some(self.accepted),
                distribution: closed,
            },
            ResultsVisibility::AfterClose | ResultsVisibility::Unspecified => PublicView {
                accepted: closed.then_some(self.accepted),
                distribution: closed,
            },
        };
        // Encrypted contest: nothing about the distribution exists until the validators
        // have decrypted the totals and the proofs verified, whatever the policy says.
        if self.encrypted && !self.revealed {
            v.distribution = false;
        }
        v
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PublicView {
    /// `None` = hidden.
    pub accepted: Option<u64>,
    pub distribution: bool,
}

/// Incremental tally: feed entries as the log grows and take a `snapshot()` at
/// any time (what a live view does). Equivalent to `tally()` over the same entries.
pub struct Tallier {
    policy: GovernancePolicy,
    manifest: Option<ElectionManifest>,
    status: Option<ElectionStatus>,
    contests: Vec<ContestTally>,
    seen: BTreeSet<(String, Vec<u8>)>,
    registered: BTreeSet<(String, Vec<u8>)>,
    verifier: Option<Box<dyn RegistrationVerifier>>,
    /// When set, manifests of any other network are ignored (and counted).
    chain_id: Option<String>,
    /// Encrypted contests: per-option homomorphic sums of the VALID ballots (nothing else
    /// about their content is ever kept).
    sums: BTreeMap<String, Vec<Ciphertext>>,
    /// Verified key ceremonies: `(election public key bytes, ceremony)`.
    ceremonies: Vec<(Vec<u8>, VerifiedCeremony)>,
    /// Decryption shares accepted so far: contest -> `(validator index, point per option)`.
    shares: BTreeMap<String, Vec<(u32, Vec<Point>)>>,
    /// Verified shares of the draw of each tied contest: contest -> `(validator index, d_i * H)`.
    draws: BTreeMap<String, Vec<(u32, Point)>>,
    bad_results: u64,
    ignored: u64,
    orphans: u64,
    count: u64,
    unattributed: RejectCounts,
    watch: BTreeSet<[u8; 32]>,
    verdicts: BTreeMap<[u8; 32], BallotVerdict>,
}

impl Tallier {
    pub fn new(policy: GovernancePolicy) -> Self {
        Self {
            policy,
            manifest: None,
            status: None,
            contests: Vec::new(),
            seen: BTreeSet::new(),
            registered: BTreeSet::new(),
            verifier: None,
            chain_id: None,
            sums: BTreeMap::new(),
            ceremonies: Vec::new(),
            shares: BTreeMap::new(),
            draws: BTreeMap::new(),
            bad_results: 0,
            ignored: 0,
            orphans: 0,
            count: 0,
            unattributed: RejectCounts::default(),
            watch: BTreeSet::new(),
            verdicts: BTreeMap::new(),
        }
    }

    /// Accept manifests of this network only.
    pub fn with_chain_id(mut self, id: String) -> Self {
        self.chain_id = Some(id);
        self
    }

    /// Installs the registration-token checker. WITHOUT one, contests that
    /// require registration reject every ballot (fail closed).
    pub fn with_verifier(mut self, v: Box<dyn RegistrationVerifier>) -> Self {
        self.verifier = Some(v);
        self
    }

    /// Advances the cursor over a log entry this tally does not interpret.
    pub fn skip(&mut self) {
        self.count += 1;
    }

    /// Closed, ENCRYPTED contests whose totals are not revealed yet and for which
    /// validator `validator_index` has not published shares: `(contest id, contest
    /// hash, final per-option sums)`. Empty until a verified ceremony exists for the
    /// election key. A validator decrypts ONLY what this lists, so it never decrypts
    /// before the close it has itself verified in the chain.
    pub fn pending_decryptions(
        &self,
        validator_index: u32,
    ) -> Vec<(String, [u8; 32], Vec<Ciphertext>)> {
        let (Some(m), Some(status)) = (self.manifest.as_ref(), self.status.as_ref()) else {
            return Vec::new();
        };
        if m.tally_key.is_empty() || !self.ceremonies.iter().any(|(k, _)| *k == m.tally_key) {
            return Vec::new();
        }
        self.contests
            .iter()
            .filter(|c| {
                c.encrypted
                    && !c.revealed
                    && matches!(
                        status.phase_of(&c.contest_id),
                        Some(Phase::VotingClosed | Phase::Tallied | Phase::Audited)
                    )
                    && !self
                        .shares
                        .get(&c.contest_id)
                        .is_some_and(|v| v.iter().any(|(j, _)| *j == validator_index))
            })
            .filter_map(|c| {
                let sums = self.sums.get(&c.contest_id)?.clone();
                let hash = contest_hash(m.contest(&c.contest_id)?);
                Some((c.contest_id.clone(), hash, sums))
            })
            .collect()
    }

    /// The registration-token checker, if installed (the host uses it to pre-check
    /// registrations on several threads).
    pub fn verifier(&self) -> Option<&dyn RegistrationVerifier> {
        self.verifier.as_deref()
    }

    /// The manifest currently in force (latest valid revision), if any.
    pub fn manifest(&self) -> Option<&ElectionManifest> {
        self.manifest.as_ref()
    }

    /// Number of log entries consumed so far (the cursor for the next `feed`).
    pub fn consumed(&self) -> u64 {
        self.count
    }

    /// Invalid governance entries (a rogue operator appending a forged transition,
    /// a manifest that alters a contest...) are IGNORED and counted, so one bad
    /// entry cannot stop the public count.
    pub fn feed(&mut self, e: LogEntry<'_>) {
        self.feed_prechecked(e, None);
    }

    /// Like `feed`, with the expensive checks of a ballot already done (by [`precheck`],
    /// possibly on other threads). The verdict is used ONLY if it was computed for this
    /// very payload and for the election key and option count in force right now;
    /// otherwise the checks run again, so the outcome is always the sequential one.
    pub fn feed_prechecked(&mut self, e: LogEntry<'_>, pre: Option<Precheck>) {
        self.count += 1;
        match e.kind {
            LogKind::Manifest => self.feed_manifest(e.payload),
            LogKind::Transition => self.feed_transition(e.payload),
            LogKind::Registration => {
                let (Some(m), Some(s)) = (self.manifest.as_ref(), self.status.as_ref()) else {
                    self.orphans += 1;
                    return;
                };
                let pre = match pre {
                    Some(Precheck::Registration(r))
                        if r.payload_hash == payload_hash(e.payload) =>
                    {
                        Some(r)
                    }
                    _ => None,
                };
                count_registration(
                    m,
                    s,
                    self.verifier.as_deref(),
                    &mut self.contests,
                    &mut self.registered,
                    &mut self.unattributed,
                    e.payload,
                    pre,
                );
            }
            LogKind::Dkg => self.feed_dkg(e.payload),
            LogKind::Result => self.feed_result(e.payload),
            LogKind::Draw => {
                let _ = self.feed_draw(e.payload);
            }
            LogKind::Ballot => {
                let watched = self.watch.contains(&payload_hash(e.payload));
                let (Some(m), Some(s)) = (self.manifest.as_ref(), self.status.as_ref()) else {
                    self.orphans += 1;
                    if watched {
                        self.verdicts.insert(
                            payload_hash(e.payload),
                            BallotVerdict::Rejected("antes de qualquer eleição válida"),
                        );
                    }
                    return;
                };
                let pre = match pre {
                    Some(Precheck::Ballot(b)) if b.payload_hash == payload_hash(e.payload) => {
                        Some(b)
                    }
                    _ => None,
                };
                let verdict = count_ballot(
                    m,
                    s,
                    &mut self.contests,
                    &mut self.sums,
                    &mut self.seen,
                    &self.registered,
                    &mut self.unattributed,
                    e.payload,
                    pre,
                );
                if watched {
                    self.verdicts.insert(payload_hash(e.payload), verdict);
                }
            }
        }
    }

    /// Asks to remember what happens to THIS ballot payload (ADR-0041): was it counted or refused,
    /// and why. Call BEFORE feeding it. Only watched ballots are remembered, so a tally of a million
    /// ballots keeps nothing extra.
    pub fn watch_ballot(&mut self, payload: &[u8]) {
        self.watch.insert(payload_hash(payload));
    }

    /// The verdict on a watched ballot, once it has been fed.
    pub fn verdict_of(&self, payload: &[u8]) -> Option<BallotVerdict> {
        self.verdicts.get(&payload_hash(payload)).copied()
    }

    /// A validator's share of the draw of a tie (ADR-0043). Accepted only for a contest that is
    /// CLOSED, revealed and really tied, for THIS tie (the input binds the tied options and their
    /// votes), with a valid proof against the verification key of the key ceremony; one per validator.
    fn feed_draw(&mut self, payload: &[u8]) -> Option<()> {
        let d = DrawShareEntry::decode(payload).ok()?;
        let m = self.manifest.as_ref()?;
        let contest = m
            .contest(&d.contest_id)
            .filter(|_| d.election_id == m.election_id)?;
        let hash = contest_hash(contest);
        if m.tally_key.is_empty() || d.contest_hash.as_slice() != hash.as_slice() {
            return None;
        }
        let ct = self
            .contests
            .iter()
            .find(|c| c.contest_id == d.contest_id)?;
        let need = needs_draw(contest, ct)?;
        let input = draw_input(&d.election_id, &d.contest_id, &hash, &need);
        if d.input != input {
            return None;
        }
        let (_, cer) = self.ceremonies.iter().find(|(k, _)| *k == m.tally_key)?;
        let vk = cer
            .verification_keys
            .iter()
            .find(|(j, _)| *j == d.validator_index)?
            .1;
        let h = hash_to_point(&input)?;
        let share = point_from_bytes(&d.share)?;
        let proof = DecryptProof::from_bytes(&d.proof)?;
        let ctx = draw_ctx(&d.election_id, &d.contest_id, &input, d.validator_index);
        if !verify_draw_share(&vk, &h, &share, &proof, &ctx) {
            return None;
        }
        let list = self.draws.entry(d.contest_id).or_default();
        if list.iter().any(|(j, _)| *j == d.validator_index) {
            return None; // one share per validator and tie
        }
        list.push((d.validator_index, share));
        Some(())
    }

    /// The seed of the draw of a tied contest, once `threshold` valid shares are on the board.
    /// Any `threshold` shares give the SAME point, so every honest node gets the same seed.
    pub fn draw_seed(&self, contest_id: &str) -> Option<[u8; 32]> {
        let m = self.manifest.as_ref()?;
        let contest = m.contest(contest_id)?;
        let ct = self.contests.iter().find(|c| c.contest_id == contest_id)?;
        let need = needs_draw(contest, ct)?;
        let (_, cer) = self.ceremonies.iter().find(|(k, _)| *k == m.tally_key)?;
        let list = self.draws.get(contest_id)?;
        if list.len() < cer.threshold {
            return None;
        }
        let mut firsts: Vec<(u32, Point)> = list.clone();
        firsts.sort_by_key(|(j, _)| *j);
        firsts.truncate(cer.threshold);
        let p = combine_draw(&firsts)?;
        let input = draw_input(&m.election_id, contest_id, &contest_hash(contest), &need);
        Some(draw_seed(&p, &input))
    }

    /// The contest is tied, a draw is possible (encrypted election with its key ceremony on the
    /// board) and the seed is not there yet: an outcome would be premature.
    pub fn awaits_draw(&self, contest_id: &str) -> bool {
        let Some(m) = self.manifest.as_ref() else {
            return false;
        };
        let (Some(contest), Some(ct)) = (
            m.contest(contest_id),
            self.contests.iter().find(|c| c.contest_id == contest_id),
        ) else {
            return false;
        };
        !m.tally_key.is_empty()
            && self.ceremonies.iter().any(|(k, _)| *k == m.tally_key)
            && needs_draw(contest, ct).is_some()
            && self.draw_seed(contest_id).is_none()
    }

    /// May the institutions rotate NOW (ADR-0045)? Only when no contest still depends on the key
    /// ceremony of the set in force: every contest is cancelled, or CLOSED with its totals decrypted
    /// (if encrypted) and no tie-break draw waiting. `Err` says what blocks the rotation.
    pub fn rotation_safe(&self) -> Result<(), String> {
        for c in &self.contests {
            let closed = matches!(
                c.phase,
                Phase::VotingClosed | Phase::Tallied | Phase::Audited
            );
            if c.phase == Phase::Cancelled {
                continue;
            }
            if !closed {
                return Err(format!(
                    "o pleito `{}` ainda não terminou (fase {:?}): espere o fechamento",
                    c.contest_id, c.phase
                ));
            }
            if c.encrypted && !c.revealed {
                return Err(format!(
                    "o pleito `{}` está fechado, mas os totais ainda não foram decifrados",
                    c.contest_id
                ));
            }
            if self.awaits_draw(&c.contest_id) {
                return Err(format!(
                    "o pleito `{}` tem um sorteio de desempate pendente",
                    c.contest_id
                ));
            }
        }
        Ok(())
    }

    /// How many valid shares of the draw of this contest are in, and how many are needed.
    pub fn draw_progress(&self, contest_id: &str) -> (usize, usize) {
        let need = self
            .manifest
            .as_ref()
            .and_then(|m| self.ceremonies.iter().find(|(k, _)| *k == m.tally_key))
            .map_or(0, |(_, c)| c.threshold);
        (self.draws.get(contest_id).map_or(0, Vec::len), need)
    }

    /// The ties this validator still has to publish its share of: `(contest id, contest hash, tie)`.
    /// Only contests of an encrypted election whose key ceremony is on the board (no ceremony, no draw).
    pub fn pending_draws(&self, validator_index: u32) -> Vec<(String, [u8; 32], DrawNeed)> {
        let Some(m) = self.manifest.as_ref() else {
            return Vec::new();
        };
        if m.tally_key.is_empty() || !self.ceremonies.iter().any(|(k, _)| *k == m.tally_key) {
            return Vec::new();
        }
        self.contests
            .iter()
            .filter(|c| {
                !self
                    .draws
                    .get(&c.contest_id)
                    .is_some_and(|v| v.iter().any(|(j, _)| *j == validator_index))
            })
            .filter_map(|c| {
                let contest = m.contest(&c.contest_id)?;
                let need = needs_draw(contest, c)?;
                Some((c.contest_id.clone(), contest_hash(contest), need))
            })
            .collect()
    }

    fn feed_manifest(&mut self, payload: &[u8]) {
        let Ok(m) = ElectionManifest::decode(payload) else {
            self.ignored += 1;
            return;
        };
        let ok = verify_manifest(&self.policy, &m).is_ok()
            && self.chain_id.as_ref().is_none_or(|c| *c == m.chain_id)
            && match &self.manifest {
                None => m.revision == 1,
                Some(prev) => validate_revision(prev, &m).is_ok(),
            };
        if !ok {
            self.ignored += 1;
            return;
        }
        match self.status.as_mut() {
            Some(s) => s.extend_for(&m),
            None => self.status = Some(ElectionStatus::new(&m)),
        }
        for c in &m.contests {
            if !self.contests.iter().any(|t| t.contest_id == c.contest_id) {
                self.contests.push(ContestTally {
                    contest_id: c.contest_id.clone(),
                    title: c.title.clone(),
                    office: c.office(),
                    scope: c.scope.clone().unwrap_or_default(),
                    phase: Phase::Draft,
                    accepted: 0,
                    registered: 0,
                    registrations_rejected: 0,
                    registration_required: !c.registration_key.is_empty(),
                    seats: c.seat_count(),
                    encrypted: !m.tally_key.is_empty(),
                    revealed: false,
                    rejected: RejectCounts::default(),
                    options: c
                        .options
                        .iter()
                        .map(|o| OptionTally {
                            index: o.index,
                            name: o.name.clone(),
                            number: o.number.clone(),
                            votes: 0,
                            kind: o.kind(),
                        })
                        .collect(),
                });
                if !m.tally_key.is_empty() {
                    self.sums.insert(
                        c.contest_id.clone(),
                        alloc::vec![Ciphertext::zero(); c.options.len()],
                    );
                }
            }
        }
        self.manifest = Some(m);
    }

    fn feed_dkg(&mut self, payload: &[u8]) {
        // Every proof of possession is RE-CHECKED and the key recomputed: a forged
        // ceremony record cannot make a wrong key look legitimate.
        let verified = DkgRecord::decode(payload)
            .ok()
            .and_then(|r| r.verify().ok().map(|v| (r.group_key, v)));
        match verified {
            Some((key, v)) if !self.ceremonies.iter().any(|(k, _)| *k == key) => {
                self.ceremonies.push((key, v));
            }
            Some(_) => {}
            None => self.bad_results += 1,
        }
    }

    /// A validator's decryption shares. Accepted only for a CLOSED, encrypted contest,
    /// from a validator of THE ceremony that made this election's key, once, with every
    /// proof valid against the final sums. `threshold` valid shares reveal the totals.
    fn feed_result(&mut self, payload: &[u8]) {
        let ok = self.try_result(payload).is_some();
        if !ok {
            self.bad_results += 1;
        }
    }

    fn try_result(&mut self, payload: &[u8]) -> Option<()> {
        let r = DecryptionShares::decode(payload).ok()?;
        let m = self.manifest.as_ref()?;
        let status = self.status.as_ref()?;
        let contest = m
            .contest(&r.contest_id)
            .filter(|_| r.election_id == m.election_id)?;
        if m.tally_key.is_empty() || r.contest_hash.as_slice() != contest_hash(contest).as_slice() {
            return None;
        }
        if !matches!(
            status.phase_of(&r.contest_id)?,
            Phase::VotingClosed | Phase::Tallied | Phase::Audited
        ) {
            return None; // before the close nobody may publish shares
        }
        let (_, cer) = self.ceremonies.iter().find(|(k, _)| *k == m.tally_key)?;
        let vk = cer
            .verification_keys
            .iter()
            .find(|(j, _)| *j == r.validator_index)?
            .1;
        let threshold = cer.threshold;
        let ct = self
            .contests
            .iter()
            .find(|c| c.contest_id == r.contest_id)?;
        if ct.revealed {
            return None;
        }
        let sums = self.sums.get(&r.contest_id)?;
        let n = sums.len();
        if r.shares.len() != n || r.proofs.len() != n {
            return None;
        }
        if self
            .shares
            .get(&r.contest_id)
            .is_some_and(|v| v.iter().any(|(j, _)| *j == r.validator_index))
        {
            return None; // one set of shares per validator and contest
        }
        let mut points = Vec::with_capacity(n);
        for (pos, ((raw, proof), sum)) in r.shares.iter().zip(&r.proofs).zip(sums).enumerate() {
            let d = point_from_bytes(raw)?;
            let p = DecryptProof::from_bytes(proof)?;
            let ctx = decrypt_ctx(
                &r.election_id,
                &r.contest_id,
                &r.contest_hash,
                pos,
                r.validator_index,
            );
            if !verify_decryption(&vk, sum, &d, &p, &ctx) {
                return None;
            }
            points.push(d);
        }
        let list = self.shares.entry(r.contest_id.clone()).or_default();
        list.push((r.validator_index, points));
        if list.len() >= threshold {
            self.try_reveal(&r.contest_id);
        }
        Some(())
    }

    /// Combines the first `threshold` accepted shares into the per-option totals and
    /// only reveals them if they add up to exactly the number of accepted ballots.
    fn try_reveal(&mut self, contest_id: &str) {
        let Some(ct) = self
            .contests
            .iter_mut()
            .find(|c| c.contest_id == contest_id)
        else {
            return;
        };
        let (Some(sums), Some(list)) = (self.sums.get(contest_id), self.shares.get(contest_id))
        else {
            return;
        };
        let mut totals = Vec::with_capacity(sums.len());
        for (pos, sum) in sums.iter().enumerate() {
            let parts: Vec<(u32, Point)> = list
                .iter()
                .filter_map(|(j, pts)| pts.get(pos).map(|p| (*j, *p)))
                .collect();
            let Some(m_point) = combine_shares(sum, &parts) else {
                self.bad_results += 1;
                return;
            };
            let Some(v) = solve_small_dlog(&m_point, ct.accepted) else {
                self.bad_results += 1;
                return;
            };
            totals.push(v);
        }
        // every ballot adds a weighted total in 1..=seats (exactly 1 with one seat)
        let weighted: u64 = ct
            .options
            .iter()
            .zip(&totals)
            .map(|(o, v)| {
                if o.kind == OptionKind::Candidate {
                    *v
                } else {
                    ct.seats.max(1) as u64 * *v
                }
            })
            .sum();
        let seats = ct.seats.max(1) as u64;
        if weighted < ct.accepted || weighted > seats * ct.accepted {
            self.bad_results += 1; // the totals MUST fit the ballots that were counted
            return;
        }
        for (o, v) in ct.options.iter_mut().zip(totals) {
            o.votes = v;
        }
        ct.revealed = true;
    }

    fn feed_transition(&mut self, payload: &[u8]) {
        let (Some(m), Some(s)) = (self.manifest.as_ref(), self.status.as_mut()) else {
            self.ignored += 1;
            return;
        };
        let Ok(t) = Transition::decode(payload) else {
            self.ignored += 1;
            return;
        };
        // Work on a copy so a refused transition leaves no trace.
        let mut trial = s.clone();
        if apply(m, &mut trial, &t, &self.policy).is_err() {
            self.ignored += 1;
            return;
        }
        *s = trial;
        if let Some(ct) = self
            .contests
            .iter_mut()
            .find(|c| c.contest_id == t.contest_id)
        {
            ct.phase = s.phase_of(&t.contest_id).unwrap_or(ct.phase);
        }
    }

    pub fn snapshot(&self) -> ElectionTally {
        let (election_id, title, revision, visibility) = match &self.manifest {
            Some(m) => (
                m.election_id.clone(),
                m.title.clone(),
                m.revision,
                m.results_visibility(),
            ),
            None => (
                String::new(),
                String::new(),
                0,
                ResultsVisibility::Unspecified,
            ),
        };
        ElectionTally {
            election_id,
            title,
            revision,
            visibility,
            seq: self.status.as_ref().map_or(0, |s| s.seq),
            contests: self.contests.clone(),
            ignored_governance: self.ignored,
            orphan_ballots: self.orphans,
            unattributed: self.unattributed,
            bad_results: self.bad_results,
            entries_seen: self.count,
        }
    }
}

/// Replays a whole log. Pure: same entries, same numbers.
pub fn tally<'a>(
    policy: &GovernancePolicy,
    entries: impl IntoIterator<Item = LogEntry<'a>>,
) -> ElectionTally {
    replay_all(Tallier::new(policy.clone()), entries)
}

/// Like `tally`, with a registration verifier (needed for contests that require one).
pub fn tally_with<'a>(
    policy: &GovernancePolicy,
    verifier: Box<dyn RegistrationVerifier>,
    entries: impl IntoIterator<Item = LogEntry<'a>>,
) -> ElectionTally {
    replay_all(
        Tallier::new(policy.clone()).with_verifier(verifier),
        entries,
    )
}

fn replay_all<'a>(
    mut t: Tallier,
    entries: impl IntoIterator<Item = LogEntry<'a>>,
) -> ElectionTally {
    for e in entries {
        t.feed(e);
    }
    t.snapshot()
}

#[allow(clippy::too_many_arguments)]
fn count_registration(
    m: &ElectionManifest,
    s: &ElectionStatus,
    verifier: Option<&dyn RegistrationVerifier>,
    contests: &mut [ContestTally],
    registered: &mut BTreeSet<(String, Vec<u8>)>,
    unattributed: &mut RejectCounts,
    payload: &[u8],
    pre: Option<RegistrationCheck>,
) {
    let Ok(r) = SubmittedRegistration::decode(payload) else {
        unattributed.malformed += 1;
        return;
    };
    let Some(contest) = m
        .contest(&r.contest_id)
        .filter(|_| r.election_id == m.election_id)
    else {
        unattributed.unknown_contest += 1;
        return;
    };
    let Some(ct) = contests.iter_mut().find(|c| c.contest_id == r.contest_id) else {
        unattributed.unknown_contest += 1;
        return;
    };
    let open = matches!(
        s.phase_of(&r.contest_id),
        Some(Phase::RegistrationOpen | Phase::VotingOpen)
    );
    let msg = registration_message(&r.election_id, &r.contest_id, &r.voter_key);
    // a token verdict computed earlier counts only for the same registration key
    let token_ok = |key: &[u8]| match pre.as_ref().filter(|p| p.registration_key == key) {
        Some(p) => p.token_ok,
        None => verifier.is_some_and(|v| v.verify(key, &msg, &r.token)),
    };
    let valid = open && !contest.registration_key.is_empty() && token_ok(&contest.registration_key);
    // A key registers once: repeats are refused, not double-counted.
    if !valid || !registered.insert((r.contest_id.clone(), r.voter_key.clone())) {
        ct.registrations_rejected += 1;
        return;
    }
    ct.registered += 1;
}

/// The expensive check of a registration: its blind-RSA token.
pub struct RegistrationCheck {
    payload_hash: [u8; 32],
    registration_key: Vec<u8>,
    token_ok: bool,
}

/// A verdict computed ahead of counting, by [`precheck`] or [`precheck_registration`].
pub enum Precheck {
    Ballot(BallotCheck),
    Registration(RegistrationCheck),
}

/// Checks the token of a registration against the contest's registration key. `None`
/// when the payload is not a registration of this election.
pub fn precheck_registration(
    m: &ElectionManifest,
    verifier: &dyn RegistrationVerifier,
    payload: &[u8],
) -> Option<RegistrationCheck> {
    let r = SubmittedRegistration::decode(payload).ok()?;
    let contest = m
        .contest(&r.contest_id)
        .filter(|_| r.election_id == m.election_id)?;
    if contest.registration_key.is_empty() {
        return None;
    }
    let msg = registration_message(&r.election_id, &r.contest_id, &r.voter_key);
    Some(RegistrationCheck {
        payload_hash: payload_hash(payload),
        registration_key: contest.registration_key.clone(),
        token_ok: verifier.verify(&contest.registration_key, &msg, &r.token),
    })
}

/// The expensive part of counting a ballot (its signature and, in an encrypted
/// election, the zero-knowledge proofs), done WITHOUT any tally state, so many ballots
/// can be checked at once on several threads while the counting itself stays
/// sequential and in board order.
pub struct BallotCheck {
    payload_hash: [u8; 32],
    /// The election key and option count the proofs were checked against.
    tally_key: Vec<u8>,
    options: usize,
    seats: usize,
    signature_ok: bool,
    /// Encrypted mode only: the parsed ballot when its proofs verified.
    proven: Option<EncryptedBallot>,
}

fn payload_hash(payload: &[u8]) -> [u8; 32] {
    teko_crypto::tagged_hash(teko_crypto::tags::BALLOT_PRECHECK, &[payload])
}

/// Checks the signature and the proofs of a ballot against `m` (the manifest in force).
/// `None` when the payload is not a ballot of this election (the tally then rejects it
/// itself, cheaply).
pub fn precheck(m: &ElectionManifest, payload: &[u8]) -> Option<BallotCheck> {
    let b = SubmittedBallot::decode(payload).ok()?;
    let contest = m
        .contest(&b.contest_id)
        .filter(|_| b.election_id == m.election_id)?;
    let options = contest.options.len();
    let seats = contest.seat_count();
    let special = contest.special_positions();
    let shape = BallotShape {
        n: options,
        seats,
        special: &special,
    };
    let signature_ok = verify_ballot(&b.voter_key, &b.body(), &b.signature).is_ok();
    let proven = if signature_ok && !m.tally_key.is_empty() {
        let ctx = ballot_ctx(&b.election_id, &b.contest_id, &b.voter_key);
        match (
            point_from_bytes(&m.tally_key),
            EncryptedBallot::from_bytes_seats(options, seats, &b.encrypted),
        ) {
            (Some(pk), Some(eb))
                if b.option_index == 0
                    && teko_ballot_crypto::verify_ballot_shape(&pk, &shape, &ctx, &eb) =>
            {
                Some(eb)
            }
            _ => None,
        }
    } else {
        None
    };
    Some(BallotCheck {
        payload_hash: payload_hash(payload),
        tally_key: m.tally_key.clone(),
        options,
        seats,
        signature_ok,
        proven,
    })
}

#[allow(clippy::too_many_arguments)]
fn count_ballot(
    m: &ElectionManifest,
    s: &ElectionStatus,
    contests: &mut [ContestTally],
    sums: &mut BTreeMap<String, Vec<Ciphertext>>,
    seen: &mut BTreeSet<(String, Vec<u8>)>,
    registered: &BTreeSet<(String, Vec<u8>)>,
    unattributed: &mut RejectCounts,
    payload: &[u8],
    pre: Option<BallotCheck>,
) -> BallotVerdict {
    let Ok(b) = SubmittedBallot::decode(payload) else {
        unattributed.malformed += 1;
        return BallotVerdict::Rejected("malformada");
    };
    let Some(contest) = m
        .contest(&b.contest_id)
        .filter(|_| b.election_id == m.election_id)
    else {
        unattributed.unknown_contest += 1;
        return BallotVerdict::Rejected("pleito desconhecido");
    };
    let Some(ct) = contests.iter_mut().find(|c| c.contest_id == b.contest_id) else {
        unattributed.unknown_contest += 1;
        return BallotVerdict::Rejected("pleito desconhecido");
    };
    if b.contest_hash.as_slice() != contest_hash(contest).as_slice() {
        ct.rejected.wrong_contest_version += 1;
        return BallotVerdict::Rejected("versão do pleito diferente");
    }
    if s.phase_of(&b.contest_id) != Some(Phase::VotingOpen) {
        ct.rejected.not_open += 1;
        return BallotVerdict::Rejected("a votação não estava aberta");
    }
    let encrypted_mode = !m.tally_key.is_empty();
    // Plaintext mode keeps the option; encrypted mode has NO option to read: the vote
    // is only inside the ciphertexts, and the signed body says option 0.
    let mut opt = 0;
    if !encrypted_mode {
        if !b.encrypted.is_empty() {
            ct.rejected.bad_proof += 1; // an encrypted ballot in a plaintext election
            return BallotVerdict::Rejected("cédula cifrada em eleição em claro");
        }
        let Some(pos) = ct.options.iter().position(|o| o.index == b.option_index) else {
            ct.rejected.unknown_option += 1;
            return BallotVerdict::Rejected("opção inexistente");
        };
        opt = pos;
    }
    // A verdict computed earlier is trusted only for the same election key and options
    let pre = pre.filter(|p| {
        p.tally_key == m.tally_key
            && p.options == contest.options.len()
            && p.seats == contest.seat_count()
    });
    let signature_ok = match &pre {
        Some(p) => p.signature_ok,
        None => verify_ballot(&b.voter_key, &b.body(), &b.signature).is_ok(),
    };
    if !signature_ok {
        ct.rejected.bad_signature += 1;
        return BallotVerdict::Rejected("assinatura inválida");
    }
    let mut cts: Option<EncryptedBallot> = None;
    if encrypted_mode {
        match pre {
            // checked already (a ballot whose proofs failed carries no parsed form)
            Some(p) => match p.proven {
                Some(eb) => cts = Some(eb),
                None => {
                    ct.rejected.bad_proof += 1;
                    return BallotVerdict::Rejected("provas da cédula inválidas");
                }
            },
            None => {
                let n = contest.options.len();
                let special = contest.special_positions();
                let shape = BallotShape {
                    n,
                    seats: contest.seat_count(),
                    special: &special,
                };
                let pk = point_from_bytes(&m.tally_key);
                let parsed = EncryptedBallot::from_bytes_seats(n, shape.seats, &b.encrypted);
                let ctx = ballot_ctx(&b.election_id, &b.contest_id, &b.voter_key);
                match (pk, parsed) {
                    (Some(pk), Some(eb))
                        if b.option_index == 0
                            && teko_ballot_crypto::verify_ballot_shape(&pk, &shape, &ctx, &eb) =>
                    {
                        cts = Some(eb);
                    }
                    _ => {
                        ct.rejected.bad_proof += 1;
                        return BallotVerdict::Rejected("provas da cédula inválidas");
                    }
                }
            }
        }
    }
    if !contest.registration_key.is_empty()
        && !registered.contains(&(b.contest_id.clone(), b.voter_key.clone()))
    {
        ct.rejected.unregistered += 1;
        return BallotVerdict::Rejected("sem registro anônimo");
    }
    if !seen.insert((b.contest_id.clone(), b.voter_key.clone())) {
        ct.rejected.duplicate += 1;
        return BallotVerdict::Rejected("esta chave já votou neste pleito");
    }
    ct.accepted += 1;
    match cts {
        Some(eb) => {
            // Homomorphic: add the ENCRYPTED one-hot vector to the running sums.
            if let Some(acc) = sums.get_mut(&b.contest_id) {
                for (a, c) in acc.iter_mut().zip(&eb.cts) {
                    *a = *a + *c;
                }
            }
        }
        None => {
            if let Some(o) = ct.options.get_mut(opt) {
                o.votes += 1;
            }
        }
    }
    BallotVerdict::Counted
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ballot::sign_ballot;
    use crate::lifecycle::sign_transition;
    use crate::manifest::fixtures::*;
    use crate::manifest::sign_manifest;
    use proptest::prelude::*;
    use teko_crypto::SigningKey;
    use teko_crypto::voter::derive_voter_key;

    type Log = Vec<(LogKind, Vec<u8>)>;

    fn world(seed: u64) -> (Vec<SigningKey>, GovernancePolicy, ElectionManifest) {
        let k = keys(3, seed);
        let pol = policy(&k, 2);
        let m = signed(&k, &[0, 1]);
        (k, pol, m)
    }

    fn run(pol: &GovernancePolicy, log: &Log) -> ElectionTally {
        tally(
            pol,
            log.iter().map(|(kind, p)| LogEntry {
                kind: *kind,
                payload: p.as_slice(),
            }),
        )
    }

    /// Transitions that move `contest` to VotingClosed (4 steps), on ITS schedule.
    fn steps(m: &ElectionManifest, k: &[SigningKey], contest: &str, seq0: u64) -> Vec<Transition> {
        let c = m.contest(contest).unwrap();
        let s = c.schedule.clone().unwrap();
        let plan = [
            (
                Phase::Draft,
                Phase::CandidatesPublished,
                s.registration_opens - 50,
            ),
            (
                Phase::CandidatesPublished,
                Phase::RegistrationOpen,
                s.registration_opens,
            ),
            (Phase::RegistrationOpen, Phase::VotingOpen, s.voting_opens),
            (Phase::VotingOpen, Phase::VotingClosed, s.voting_closes),
        ];
        plan.iter()
            .enumerate()
            .map(|(i, (f, to, at))| {
                let mut t = Transition {
                    election_id: m.election_id.clone(),
                    contest_id: contest.into(),
                    contest_hash: contest_hash(c).to_vec(),
                    from_phase: *f as i32,
                    to_phase: *to as i32,
                    seq: seq0 + i as u64 + 1,
                    at: *at,
                    reason: String::new(),
                    signatures: vec![],
                };
                t.signatures = [0usize, 1]
                    .iter()
                    .map(|j| sign_transition(&k[*j], &t))
                    .collect();
                t
            })
            .collect()
    }

    fn tr(t: &Transition) -> (LogKind, Vec<u8>) {
        (LogKind::Transition, t.encode_to_vec())
    }

    fn ballot_msg(m: &ElectionManifest, contest: &str, voter: u8, option: u32) -> SubmittedBallot {
        let key = derive_voter_key(&[voter; 32], &m.election_id, contest).unwrap();
        let body = BallotBody {
            election_id: m.election_id.clone(),
            contest_id: contest.into(),
            contest_hash: contest_hash(m.contest(contest).unwrap()).to_vec(),
            option_index: option,
            ciphertext_hash: Vec::new(),
        };
        SubmittedBallot {
            election_id: body.election_id.clone(),
            contest_id: body.contest_id.clone(),
            contest_hash: body.contest_hash.clone(),
            option_index: option,
            voter_key: teko_crypto::public_key_bytes(&key),
            signature: sign_ballot(&key, &body),
            encrypted: Vec::new(),
        }
    }

    fn b(m: &ElectionManifest, contest: &str, voter: u8, option: u32) -> (LogKind, Vec<u8>) {
        (
            LogKind::Ballot,
            ballot_msg(m, contest, voter, option).encode_to_vec(),
        )
    }

    /// manifest + President brought to VotingOpen.
    fn open_president(m: &ElectionManifest, k: &[SigningKey]) -> Log {
        let mut log: Log = vec![(LogKind::Manifest, m.encode_to_vec())];
        log.extend(steps(m, k, PRESIDENT, 0)[..3].iter().map(tr));
        log
    }

    /// Accepts token == message (stands in for the RSA check in unit tests).
    struct Echo;
    impl RegistrationVerifier for Echo {
        fn verify(&self, _key: &[u8], message: &[u8], token: &[u8]) -> bool {
            message == token
        }
    }

    fn reg(m: &ElectionManifest, contest: &str, voter: u8) -> (LogKind, Vec<u8>) {
        let key = derive_voter_key(&[voter; 32], &m.election_id, contest).unwrap();
        let pk = teko_crypto::public_key_bytes(&key);
        let r = SubmittedRegistration {
            election_id: m.election_id.clone(),
            contest_id: contest.into(),
            token: registration_message(&m.election_id, contest, &pk),
            voter_key: pk,
        };
        (LogKind::Registration, r.encode_to_vec())
    }

    fn registered_world(seed: u64) -> (Vec<SigningKey>, GovernancePolicy, ElectionManifest) {
        let k = keys(3, seed);
        let pol = policy(&k, 2);
        let mut m = manifest();
        for c in &mut m.contests {
            if c.contest_id == PRESIDENT {
                c.registration_key = vec![9; 300];
            }
        }
        m.signatures = [0usize, 1]
            .iter()
            .map(|i| sign_manifest(&k[*i], &m))
            .collect();
        (k, pol, m)
    }

    fn run_reg(pol: &GovernancePolicy, log: &Log) -> ElectionTally {
        tally_with(
            pol,
            Box::new(Echo),
            log.iter().map(|(kind, p)| LogEntry {
                kind: *kind,
                payload: p.as_slice(),
            }),
        )
    }

    #[test]
    fn contests_requiring_registration_count_only_registered_keys() {
        let (k, pol, m) = registered_world(11);
        let st = steps(&m, &k, PRESIDENT, 0);
        let log: Log = vec![
            (LogKind::Manifest, m.encode_to_vec()),
            reg(&m, PRESIDENT, 1), // Draft: registration not open yet
            tr(&st[0]),
            tr(&st[1]),
            reg(&m, PRESIDENT, 2), // RegistrationOpen: ok
            reg(&m, PRESIDENT, 2), // same key again: refused
            tr(&st[2]),
            reg(&m, PRESIDENT, 3),  // still allowed while voting is open
            b(&m, PRESIDENT, 1, 0), // never registered (its registration came too early)
            b(&m, PRESIDENT, 2, 1),
            b(&m, PRESIDENT, 3, 1),
            b(&m, PRESIDENT, 4, 2), // never registered
        ];
        let c = run_reg(&pol, &log).contest(PRESIDENT).cloned().unwrap();
        assert!(c.registration_required);
        assert_eq!((c.registered, c.registrations_rejected), (2, 2));
        assert_eq!((c.accepted, c.rejected.unregistered), (2, 2));
        assert_eq!(c.options[1].votes, 2);
    }

    #[test]
    fn a_token_is_useless_for_another_key() {
        let (k, pol, m) = registered_world(12);
        let st = steps(&m, &k, PRESIDENT, 0);
        let (_, bytes) = reg(&m, PRESIDENT, 5);
        let mut forged = SubmittedRegistration::decode(bytes.as_slice()).unwrap();
        forged.voter_key[1] ^= 1; // token no longer matches this key
        let log: Log = vec![
            (LogKind::Manifest, m.encode_to_vec()),
            tr(&st[0]),
            tr(&st[1]),
            (LogKind::Registration, forged.encode_to_vec()),
        ];
        let c = run_reg(&pol, &log).contest(PRESIDENT).cloned().unwrap();
        assert_eq!((c.registered, c.registrations_rejected), (0, 1));
    }

    #[test]
    fn without_a_verifier_registration_contests_fail_closed() {
        let (k, pol, m) = registered_world(13);
        let mut log = open_president(&m, &k);
        log.push(reg(&m, PRESIDENT, 1));
        log.push(b(&m, PRESIDENT, 1, 0));
        let c = run(&pol, &log).contest(PRESIDENT).cloned().unwrap();
        assert_eq!(
            (c.registered, c.accepted, c.rejected.unregistered),
            (0, 0, 1)
        );
    }

    #[test]
    fn contests_without_a_registration_key_are_unchanged() {
        let (k, pol, m) = registered_world(14);
        let st = steps(&m, &k, GOVERNOR_SP, 0);
        let mut log: Log = vec![(LogKind::Manifest, m.encode_to_vec())];
        log.extend(st[..3].iter().map(tr));
        log.push(b(&m, GOVERNOR_SP, 1, 0));
        let c = run(&pol, &log).contest(GOVERNOR_SP).cloned().unwrap();
        assert!(!c.registration_required);
        assert_eq!((c.accepted, c.rejected.unregistered), (1, 0));
    }

    #[test]
    fn a_tallier_bound_to_a_network_ignores_manifests_of_another() {
        let k = keys(3, 21);
        let pol = policy(&k, 2);
        let mut m = manifest();
        m.chain_id = "teko-testnet-1".into();
        m.signatures = [0usize, 1]
            .iter()
            .map(|i| sign_manifest(&k[*i], &m))
            .collect();
        let log: Log = vec![(LogKind::Manifest, m.encode_to_vec())];
        let feed = |t: &mut Tallier| {
            for (kind, p) in &log {
                t.feed(LogEntry {
                    kind: *kind,
                    payload: p,
                });
            }
            t.snapshot()
        };
        let mine = feed(&mut Tallier::new(pol.clone()).with_chain_id("teko-devnet-1".into()));
        assert_eq!((mine.contests.len(), mine.ignored_governance), (0, 1));
        let ok = feed(&mut Tallier::new(pol.clone()).with_chain_id("teko-testnet-1".into()));
        assert_eq!((ok.contests.len(), ok.ignored_governance), (3, 0));
        let unbound = feed(&mut Tallier::new(pol));
        assert_eq!(
            unbound.contests.len(),
            3,
            "unbound tallier (dev) accepts any"
        );
    }

    #[test]
    fn counts_per_option_and_ranks_with_an_others_bucket() {
        let (k, pol, m) = world(1);
        let mut log = open_president(&m, &k);
        for (voter, opt) in [(1u8, 0u32), (2, 1), (3, 1), (4, 2), (5, 1), (6, 0), (7, 1)] {
            log.push(b(&m, PRESIDENT, voter, opt));
        }
        let t = run(&pol, &log);
        let c = t.contest(PRESIDENT).unwrap();
        assert_eq!(
            (c.accepted, c.rejected.total(), c.phase),
            (7, 0, Phase::VotingOpen)
        );
        assert_eq!(
            c.options.iter().map(|o| o.votes).collect::<Vec<_>>(),
            [2, 4, 1]
        );
        let r = c.ranked(2);
        assert_eq!(
            r.top.iter().map(|o| (o.index, o.votes)).collect::<Vec<_>>(),
            [(1, 4), (0, 2)]
        );
        assert_eq!((r.others_votes, r.others_count), (1, 1));
        // ties are broken by ballot position, so the ranking is deterministic
        let mut tie = open_president(&m, &k);
        tie.push(b(&m, PRESIDENT, 1, 2));
        tie.push(b(&m, PRESIDENT, 2, 0));
        let r = run(&pol, &tie).contest(PRESIDENT).unwrap().ranked(10);
        assert_eq!(r.top.iter().map(|o| o.index).collect::<Vec<_>>(), [0, 2, 1]);
        // other contests are untouched
        assert_eq!(t.contest(GOVERNOR_SP).unwrap().accepted, 0);
    }

    #[test]
    fn a_ballot_counts_only_while_its_contest_is_open_in_log_order() {
        let (k, pol, m) = world(2);
        let st = steps(&m, &k, PRESIDENT, 0);
        let log: Log = vec![
            (LogKind::Manifest, m.encode_to_vec()),
            b(&m, PRESIDENT, 1, 0), // draft: too early
            tr(&st[0]),
            tr(&st[1]),
            b(&m, PRESIDENT, 2, 0), // registration only: too early
            tr(&st[2]),
            b(&m, PRESIDENT, 3, 1), // OPEN: counts
            b(&m, PRESIDENT, 4, 1), // OPEN: counts
            tr(&st[3]),
            b(&m, PRESIDENT, 5, 1), // closed: too late
        ];
        let c = run(&pol, &log).contest(PRESIDENT).cloned().unwrap();
        assert_eq!(
            (c.accepted, c.rejected.not_open, c.phase),
            (2, 3, Phase::VotingClosed)
        );
        assert_eq!(c.options[1].votes, 2);
    }

    #[test]
    fn blank_and_null_are_counted_reported_apart_and_never_ranked() {
        use crate::manifest::{BLANK_NAME, ElectionOption, NULL_NAME, OptionKind};
        let k = keys(3, 11);
        let pol = policy(&k, 2);
        let mut m = crate::manifest::fixtures::manifest();
        for (kind, name) in [
            (OptionKind::Blank, BLANK_NAME),
            (OptionKind::Null, NULL_NAME),
        ] {
            let c = &mut m.contests[0];
            let index = c.options.len() as u32;
            c.options.push(ElectionOption {
                index,
                name: name.into(),
                kind: kind as i32,
                ..Default::default()
            });
        }
        m.signatures = [0usize, 1]
            .iter()
            .map(|i| crate::manifest::sign_manifest(&k[*i], &m))
            .collect();
        let mut log = open_president(&m, &k);
        // candidates 0,1,2; blank = 3, null = 4. Voters: 2 for cand 0, 1 for cand 2, 3 blank, 2 null
        let picks = [0u32, 0, 2, 3, 3, 3, 4, 4];
        for (i, o) in picks.iter().enumerate() {
            log.push(b(&m, PRESIDENT, i as u8 + 1, *o));
        }
        let c = run(&pol, &log).contest(PRESIDENT).cloned().unwrap();
        assert_eq!(c.accepted, 8);
        assert_eq!(
            (c.valid_votes(), c.blank_votes(), c.null_votes()),
            (3, 3, 2)
        );
        let r = c.ranked(10);
        assert_eq!(r.top.len(), 3, "only the candidates are ranked");
        assert!(r.top.iter().all(|o| o.kind == OptionKind::Candidate));
        assert_eq!(r.others_count, 0);
        assert_eq!(r.top[0].votes, 2);
        // with top-1, the others bucket holds candidates only
        let r1 = c.ranked(1);
        assert_eq!((r1.others_count, r1.others_votes), (2, 1));
    }

    #[test]
    fn the_institutions_may_rotate_only_when_every_contest_is_over() {
        let (k, pol, m) = world(3);
        // no election at all: nothing depends on the key
        assert!(Tallier::new(pol.clone()).rotation_safe().is_ok());
        // a contest that is open (the others still drafts): blocked
        let log = open_president(&m, &k);
        let mut t = Tallier::new(pol.clone());
        for (kind, p) in &log {
            t.feed(LogEntry {
                kind: *kind,
                payload: p,
            });
        }
        let e = t.rotation_safe().unwrap_err();
        assert!(e.contains("ainda não terminou"), "{e}");
        // cancelling every contest frees the rotation
        let mut cancelled = Tallier::new(pol);
        for (kind, p) in &log {
            cancelled.feed(LogEntry {
                kind: *kind,
                payload: p,
            });
        }
        for c in &mut cancelled.contests {
            c.phase = Phase::Cancelled;
        }
        assert!(cancelled.rotation_safe().is_ok());
    }

    #[test]
    fn one_ballot_per_voter_key_per_contest_first_wins() {
        let (k, pol, m) = world(3);
        let mut log = open_president(&m, &k);
        log.push(b(&m, PRESIDENT, 9, 0));
        log.push(b(&m, PRESIDENT, 9, 2)); // same voter, other option: ignored
        log.push(b(&m, PRESIDENT, 9, 0)); // replay: ignored
        let c = run(&pol, &log).contest(PRESIDENT).cloned().unwrap();
        assert_eq!((c.accepted, c.rejected.duplicate), (1, 2));
        assert_eq!(
            c.options.iter().map(|o| o.votes).collect::<Vec<_>>(),
            [1, 0, 0],
            "the FIRST vote stands"
        );
    }

    #[test]
    fn a_watched_ballot_gets_its_own_verdict_and_only_watched_ones_are_kept() {
        let (k, pol, m) = world(3);
        let mut log = open_president(&m, &k);
        let first = b(&m, PRESIDENT, 9, 0);
        let dup = b(&m, PRESIDENT, 9, 2);
        let other = b(&m, PRESIDENT, 8, 1);
        let mut bad = ballot_msg(&m, PRESIDENT, 7, 0);
        bad.signature[0] ^= 1;
        let bad = (LogKind::Ballot, bad.encode_to_vec());
        log.extend([first.clone(), dup.clone(), other.clone(), bad.clone()]);
        let mut t = Tallier::new(pol);
        for x in [&first, &dup, &bad] {
            t.watch_ballot(&x.1);
        }
        for (kind, p) in &log {
            t.feed(LogEntry {
                kind: *kind,
                payload: p,
            });
        }
        assert_eq!(t.verdict_of(&first.1), Some(BallotVerdict::Counted));
        assert!(matches!(
            t.verdict_of(&dup.1),
            Some(BallotVerdict::Rejected(why)) if why.contains("já votou")
        ));
        assert!(matches!(
            t.verdict_of(&bad.1),
            Some(BallotVerdict::Rejected(why)) if why.contains("assinatura")
        ));
        assert_eq!(t.verdict_of(&other.1), None, "not watched: nothing kept");
        // the verdicts agree with the counters
        let c = t.snapshot().contest(PRESIDENT).cloned().unwrap();
        assert_eq!(
            (c.accepted, c.rejected.duplicate, c.rejected.bad_signature),
            (2, 1, 1)
        );
    }

    #[test]
    fn a_ballot_outside_the_voting_window_is_refused_with_that_reason() {
        let (_, pol, m) = world(3);
        let early = b(&m, PRESIDENT, 9, 0);
        let mut t = Tallier::new(pol);
        t.watch_ballot(&early.1);
        t.feed(LogEntry {
            kind: early.0,
            payload: &early.1,
        });
        assert!(matches!(
            t.verdict_of(&early.1),
            Some(BallotVerdict::Rejected(_))
        ));
    }

    #[test]
    fn every_invalid_ballot_is_counted_by_reason_never_dropped_silently() {
        let (k, pol, m) = world(4);
        let mut log = open_president(&m, &k);
        // bad signature
        let mut bad = ballot_msg(&m, PRESIDENT, 1, 0);
        bad.signature[3] ^= 1;
        log.push((LogKind::Ballot, bad.encode_to_vec()));
        // signed for option 0 but claims option 1
        let mut forged = ballot_msg(&m, PRESIDENT, 2, 0);
        forged.option_index = 1;
        log.push((LogKind::Ballot, forged.encode_to_vec()));
        // another version of the contest
        let mut stale = ballot_msg(&m, PRESIDENT, 3, 0);
        stale.contest_hash = vec![9; 32];
        log.push((LogKind::Ballot, stale.encode_to_vec()));
        // option that does not exist (even though correctly signed)
        log.push(b(&m, PRESIDENT, 4, 9));
        // wrong election / unknown contest / garbage
        let mut other = ballot_msg(&m, PRESIDENT, 5, 0);
        other.election_id = "outra-eleicao".into();
        log.push((LogKind::Ballot, other.encode_to_vec()));
        log.push((
            LogKind::Ballot,
            ballot_msg(&m, PRESIDENT, 6, 0)
                .encode_to_vec()
                .into_iter()
                .map(|_| 0xFF)
                .collect(),
        ));
        log.push(b(&m, PRESIDENT, 7, 2)); // the only good one
        let t = run(&pol, &log);
        let c = t.contest(PRESIDENT).unwrap();
        assert_eq!(c.accepted, 1);
        assert_eq!(
            (
                c.rejected.bad_signature,
                c.rejected.wrong_contest_version,
                c.rejected.unknown_option
            ),
            (2, 1, 1)
        );
        assert_eq!(
            (t.unattributed.unknown_contest, t.unattributed.malformed),
            (1, 1)
        );
        // nothing vanished: 7 ballot entries = accepted + all rejections
        assert_eq!(c.accepted + c.rejected.total() + t.unattributed.total(), 7);
    }

    #[test]
    fn forged_governance_entries_are_ignored_and_counted_without_stopping_the_count() {
        let (k, pol, m) = world(5);
        let mut log = open_president(&m, &k);
        let st = steps(&m, &k, PRESIDENT, 0);
        // a rogue "close" signed by ONE institution
        let mut rogue = st[3].clone();
        rogue.signatures.truncate(1);
        log.push(tr(&rogue));
        // a manifest revision that ALTERS a contest (even if M-of-N signed)
        let mut evil = m.clone();
        evil.revision = 2;
        evil.contests[0].options[0].name = "Candidato Trocado".into();
        evil.signatures = [0usize, 1]
            .iter()
            .map(|i| sign_manifest(&k[*i], &evil))
            .collect();
        log.push((LogKind::Manifest, evil.encode_to_vec()));
        log.push(b(&m, PRESIDENT, 1, 1));
        let t = run(&pol, &log);
        assert_eq!(t.ignored_governance, 2);
        let c = t.contest(PRESIDENT).unwrap();
        assert_eq!(
            (c.accepted, c.phase),
            (1, Phase::VotingOpen),
            "voting was NOT closed by the rogue entry"
        );
        assert_eq!(
            c.options[0].name, "Candidata Onça",
            "the altered candidate was NOT adopted"
        );
    }

    #[test]
    fn ballots_before_any_valid_manifest_are_orphans() {
        let (k, pol, m) = world(6);
        let weak = signed(&k, &[0]); // only 1 of 2 signatures
        let log: Log = vec![
            b(&m, PRESIDENT, 1, 0),
            (LogKind::Manifest, weak.encode_to_vec()),
            b(&m, PRESIDENT, 2, 0),
        ];
        let t = run(&pol, &log);
        assert_eq!(
            (t.orphan_ballots, t.ignored_governance, t.contests.len()),
            (2, 1, 0)
        );
    }

    #[test]
    fn a_later_revision_adds_contests_and_keeps_counting() {
        let (k, pol, m) = world(7);
        let mut log = open_president(&m, &k);
        log.push(b(&m, PRESIDENT, 1, 0));
        let mut rev2 = m.clone();
        rev2.revision = 2;
        let mut extra = rev2.contests[2].clone();
        extra.contest_id = "prefeito-3550308-2turno".into();
        rev2.contests.push(extra);
        rev2.signatures = [0usize, 1]
            .iter()
            .map(|i| sign_manifest(&k[*i], &rev2))
            .collect();
        log.push((LogKind::Manifest, rev2.encode_to_vec()));
        log.push(b(&m, PRESIDENT, 2, 1));
        let t = run(&pol, &log);
        assert_eq!(
            (
                t.revision,
                t.contests.len(),
                t.contest(PRESIDENT).unwrap().accepted
            ),
            (2, 4, 2)
        );
        assert_eq!(
            t.contest("prefeito-3550308-2turno").unwrap().phase,
            Phase::Draft
        );
    }

    #[test]
    fn visibility_policy_masks_the_public_view_until_the_voting_closes() {
        let mut c = ContestTally {
            contest_id: "x".into(),
            title: "x".into(),
            office: Office::President,
            scope: Scope::national(),
            phase: Phase::VotingOpen,
            accepted: 42,
            registered: 0,
            registrations_rejected: 0,
            registration_required: false,
            encrypted: false,
            revealed: false,
            rejected: RejectCounts::default(),
            options: vec![],
            seats: 1,
        };
        let live = c.public_view(ResultsVisibility::Live);
        assert_eq!((live.accepted, live.distribution), (Some(42), true));
        let turnout = c.public_view(ResultsVisibility::TurnoutOnly);
        assert_eq!((turnout.accepted, turnout.distribution), (Some(42), false));
        let after = c.public_view(ResultsVisibility::AfterClose);
        assert_eq!((after.accepted, after.distribution), (None, false));
        c.phase = Phase::VotingClosed;
        for v in [
            ResultsVisibility::Live,
            ResultsVisibility::TurnoutOnly,
            ResultsVisibility::AfterClose,
        ] {
            let p = c.public_view(v);
            assert_eq!(
                (p.accepted, p.distribution),
                (Some(42), true),
                "{v:?} after close"
            );
        }
        c.phase = Phase::Cancelled;
        assert_eq!(
            c.public_view(ResultsVisibility::Live),
            PublicView {
                accepted: None,
                distribution: false
            }
        );
    }

    #[test]
    fn incremental_snapshots_equal_a_full_replay_at_every_point() {
        let (k, pol, m) = world(10);
        let mut log = open_president(&m, &k);
        for v in 0..6u8 {
            log.push(b(&m, PRESIDENT, v % 4, (v % 3) as u32)); // includes duplicates
        }
        let mut t = Tallier::new(pol.clone());
        for (i, (kind, p)) in log.iter().enumerate() {
            t.feed(LogEntry {
                kind: *kind,
                payload: p.as_slice(),
            });
            assert_eq!(t.consumed(), i as u64 + 1);
            assert_eq!(
                t.snapshot(),
                run(&pol, &log[..=i].to_vec()),
                "after entry {i}"
            );
        }
    }

    #[test]
    fn the_tally_is_deterministic_and_depends_on_log_order() {
        let (k, pol, m) = world(8);
        let opened = open_president(&m, &k);
        let mut late = opened.clone();
        late.push(b(&m, PRESIDENT, 1, 0));
        assert_eq!(run(&pol, &late), run(&pol, &late), "same log, same numbers");
        // the SAME ballot placed BEFORE the opening transitions is not counted
        let mut early: Log = vec![opened[0].clone(), b(&m, PRESIDENT, 1, 0)];
        early.extend(opened[1..].iter().cloned());
        assert_eq!(run(&pol, &late).contest(PRESIDENT).unwrap().accepted, 1);
        let e = run(&pol, &early);
        assert_eq!(e.contest(PRESIDENT).unwrap().accepted, 0);
        assert_eq!(e.contest(PRESIDENT).unwrap().rejected.not_open, 1);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(10))]
        /// However many times each voter submits, each distinct key counts once and
        /// the per-option totals always add up to the accepted count.
        #[test]
        fn totals_always_add_up(seed in any::<u64>(), picks in proptest::collection::vec((0u8..12, 0u32..3), 1..24)) {
            let (k, pol, m) = world(seed);
            let mut log = open_president(&m, &k);
            for (voter, opt) in &picks { log.push(b(&m, PRESIDENT, *voter, *opt)); }
            let t = run(&pol, &log);
            let c = t.contest(PRESIDENT).unwrap();
            let distinct: BTreeSet<u8> = picks.iter().map(|(v, _)| *v).collect();
            prop_assert_eq!(c.accepted, distinct.len() as u64);
            prop_assert_eq!(c.options.iter().map(|o| o.votes).sum::<u64>(), c.accepted);
            prop_assert_eq!(c.rejected.duplicate, (picks.len() - distinct.len()) as u64);
            // the FIRST pick of each voter is the one counted
            let mut first: alloc::collections::BTreeMap<u8, u32> = Default::default();
            for (v, o) in &picks { first.entry(*v).or_insert(*o); }
            for (i, o) in c.options.iter().enumerate() {
                prop_assert_eq!(o.votes, first.values().filter(|x| **x as usize == i).count() as u64);
            }
        }
    }

    // ───────────────────────── encrypted contests (ADR-0019) ─────────────────────────

    mod encrypted {
        use super::*;
        use crate::ballot::{ballot_ctx, ciphertext_hash};
        use crate::result::build_shares;
        use rand::SeedableRng;
        use rand::rngs::StdRng;
        use teko_ballot_crypto::dkg::{KeyShare, Participant, Round1};
        use teko_ballot_crypto::dkg_wire::{DkgRecord, Round1Msg};
        use teko_ballot_crypto::{
            EncryptedBallot, Scalar, encrypt_ballot, point_to_bytes, sum_ballots,
        };

        const SESSION: &[u8] = b"teko-devnet-1|tally-key";

        fn ceremony(seed: u64) -> (Vec<KeyShare>, DkgRecord) {
            let mut rng = StdRng::seed_from_u64(seed);
            let (n, t) = (3u32, 2usize);
            let parts: Vec<(Participant, Round1)> = (1..=n)
                .map(|i| Participant::new(&mut rng, SESSION, i, n, t).unwrap())
                .collect();
            let packages: Vec<Round1> = parts.iter().map(|p| p.1.clone()).collect();
            let shares: Vec<KeyShare> = parts
                .iter()
                .map(|(me, _)| {
                    let rec: Vec<(u32, Scalar)> = parts
                        .iter()
                        .map(|(o, p)| (p.index, o.share_for(me.index()).unwrap()))
                        .collect();
                    me.finish(SESSION, t, &packages, &rec).unwrap()
                })
                .collect();
            let record = DkgRecord {
                session: SESSION.to_vec(),
                threshold: t as u32,
                n,
                packages: packages.iter().map(Round1Msg::from).collect(),
                group_key: point_to_bytes(&shares[0].group_public_key).to_vec(),
            };
            (shares, record)
        }

        struct Voted {
            sub: SubmittedBallot,
            eb: EncryptedBallot,
        }

        fn enc_ballot(
            m: &ElectionManifest,
            contest: &str,
            voter: u8,
            choice: usize,
            rng: &mut StdRng,
        ) -> Voted {
            let pk = point_from_bytes(&m.tally_key).unwrap();
            let key = derive_voter_key(&[voter; 32], &m.election_id, contest).unwrap();
            let vk = teko_crypto::public_key_bytes(&key);
            let c = m.contest(contest).unwrap();
            let ctx = ballot_ctx(&m.election_id, contest, &vk);
            let eb = encrypt_ballot(&pk, c.options.len(), choice, &ctx, rng).unwrap();
            let bytes = eb.to_bytes();
            let body = BallotBody {
                election_id: m.election_id.clone(),
                contest_id: contest.into(),
                contest_hash: contest_hash(c).to_vec(),
                option_index: 0,
                ciphertext_hash: ciphertext_hash(&bytes),
            };
            let sub = SubmittedBallot {
                election_id: body.election_id.clone(),
                contest_id: body.contest_id.clone(),
                contest_hash: body.contest_hash.clone(),
                option_index: 0,
                voter_key: vk,
                signature: sign_ballot(&key, &body),
                encrypted: bytes,
            };
            Voted { sub, eb }
        }

        fn shares_entry(
            m: &ElectionManifest,
            ks: &KeyShare,
            sums: &[teko_ballot_crypto::Ciphertext],
            rng: &mut StdRng,
        ) -> (LogKind, Vec<u8>) {
            let c = m.contest(PRESIDENT).unwrap();
            let r = build_shares(
                &m.election_id,
                PRESIDENT,
                &contest_hash(c),
                sums,
                ks.index,
                &ks.secret,
                rng,
            );
            (LogKind::Result, r.encode_to_vec())
        }

        fn draw_entry(
            m: &ElectionManifest,
            ks: &KeyShare,
            need: &crate::draw::DrawNeed,
            rng: &mut StdRng,
        ) -> (LogKind, Vec<u8>) {
            use teko_ballot_crypto::draw::{draw_share, hash_to_point};
            let c = m.contest(PRESIDENT).unwrap();
            let hash = contest_hash(c);
            let input = crate::draw::draw_input(&m.election_id, PRESIDENT, &hash, need);
            let h = hash_to_point(&input).unwrap();
            let ctx = crate::draw::draw_ctx(&m.election_id, PRESIDENT, &input, ks.index);
            let (share, proof) = draw_share(&ks.secret, &h, &ctx, rng);
            let e = crate::draw::DrawShareEntry {
                election_id: m.election_id.clone(),
                contest_id: PRESIDENT.into(),
                contest_hash: hash.to_vec(),
                input,
                validator_index: ks.index,
                share: point_to_bytes(&share).to_vec(),
                proof: proof.to_bytes().to_vec(),
            };
            (LogKind::Draw, e.encode_to_vec())
        }

        #[test]
        fn a_tie_is_drawn_from_the_validators_shares_and_any_threshold_of_them_gives_the_same_seed()
        {
            let k = keys(3, 41);
            let pol = policy(&k, 2);
            let (kshares, record) = ceremony(7);
            let mut m = manifest();
            m.tally_key = record.group_key.clone();
            m.signatures = [0usize, 1]
                .iter()
                .map(|i| sign_manifest(&k[*i], &m))
                .collect();
            let st = steps(&m, &k, PRESIDENT, 0);
            let mut rng = StdRng::seed_from_u64(9);
            // 2 votes for option 0, 2 for option 1, 0 for option 2: a tie at the top
            let voted: Vec<Voted> = [0usize, 1, 0, 1]
                .iter()
                .enumerate()
                .map(|(i, c)| enc_ballot(&m, PRESIDENT, i as u8 + 1, *c, &mut rng))
                .collect();
            let sums = sum_ballots(3, &voted.iter().map(|v| &v.eb).collect::<Vec<_>>());
            let mut log: Log = vec![
                (LogKind::Manifest, m.encode_to_vec()),
                (LogKind::Dkg, record.encode_to_vec()),
            ];
            log.extend(st[..3].iter().map(tr));
            for v in &voted {
                log.push((LogKind::Ballot, v.sub.encode_to_vec()));
            }
            let run_t = |log: &Log| {
                let mut t = Tallier::new(pol.clone());
                for (kind, p) in log {
                    t.feed(LogEntry {
                        kind: *kind,
                        payload: p,
                    });
                }
                t
            };
            // the institutions cannot rotate while the election is open (ADR-0045)
            assert!(
                run_t(&log)
                    .rotation_safe()
                    .unwrap_err()
                    .contains("ainda não terminou")
            );
            // before the close and before the totals are revealed there is nothing to draw
            let need = crate::draw::DrawNeed {
                tied: vec![0, 1],
                take: 1,
                goal: crate::draw::DrawGoal::Winner,
                votes: 2,
            };
            let early = draw_entry(&m, &kshares[0], &need, &mut rng);
            let mut l = log.clone();
            l.push(early.clone());
            assert!(run_t(&l).pending_draws(1).is_empty());
            assert_eq!(run_t(&l).draw_progress(PRESIDENT).0, 0);
            // close and reveal (2 of 3 shares): now the tie exists and every validator owes its share
            log.push(tr(&st[3]));
            log.push(shares_entry(&m, &kshares[0], &sums, &mut rng));
            log.push(shares_entry(&m, &kshares[1], &sums, &mut rng));
            let t = run_t(&log);
            assert!(t.snapshot().contest(PRESIDENT).unwrap().revealed);
            // revealed, but the tie still has to be drawn: not yet
            assert!(t.rotation_safe().unwrap_err().contains("sorteio"));
            let pend = t.pending_draws(1);
            assert_eq!(pend.len(), 1);
            assert_eq!(pend[0].2, need);
            assert_eq!(t.draw_seed(PRESIDENT), None);

            // a share for ANOTHER tie, with a forged proof, or from an outsider is refused; one per validator
            let other = crate::draw::DrawNeed {
                votes: 3,
                ..need.clone()
            };
            let mut l = log.clone();
            l.push(draw_entry(&m, &kshares[0], &other, &mut rng));
            let bad = KeyShare {
                index: 2,
                secret: kshares[1].secret + Scalar::ONE,
                group_public_key: kshares[1].group_public_key,
                verification_keys: kshares[1].verification_keys.clone(),
            };
            l.push(draw_entry(&m, &bad, &need, &mut rng));
            let mut outsider = crate::draw::DrawShareEntry::decode(
                draw_entry(&m, &kshares[2], &need, &mut rng).1.as_slice(),
            )
            .unwrap();
            outsider.validator_index = 9;
            l.push((LogKind::Draw, outsider.encode_to_vec()));
            assert_eq!(
                run_t(&l).draw_progress(PRESIDENT).0,
                0,
                "none of those counts"
            );
            // one valid share is not enough; two (the threshold) give a seed; a duplicate adds nothing
            log.push(draw_entry(&m, &kshares[0], &need, &mut rng));
            log.push(draw_entry(&m, &kshares[0], &need, &mut rng));
            let t = run_t(&log);
            assert_eq!(t.draw_progress(PRESIDENT), (1, 2));
            assert!(t.draw_seed(PRESIDENT).is_none());
            assert_eq!(t.pending_draws(1).len(), 0, "validator 1 has published");
            assert_eq!(t.pending_draws(2).len(), 1);
            // whichever two validators contribute, the seed is the same
            let mut seeds = Vec::new();
            for pair in [[0usize, 1], [1, 2], [0, 2]] {
                let mut l = log[..log.len() - 2].to_vec();
                for i in pair {
                    l.push(draw_entry(&m, &kshares[i], &need, &mut rng));
                }
                seeds.push(run_t(&l).draw_seed(PRESIDENT).unwrap());
            }
            assert_eq!(seeds[0], seeds[1]);
            assert_eq!(seeds[1], seeds[2]);
            // ...and with it the contest is DECIDED among the tied, with the same bytes everywhere
            let c = m.contest(PRESIDENT).unwrap();
            let t = run_t(&log);
            let snap = t.snapshot();
            let ct = snap.contest(PRESIDENT).unwrap();
            assert!(crate::outcome::needs_draw(c, ct).is_some());
            let o = crate::outcome::decide_with(
                "v",
                &m.election_id,
                c,
                &contest_hash(c),
                ct,
                Some(&seeds[0]),
            )
            .unwrap();
            assert_eq!(o.kind(), crate::outcome::OutcomeKind::Elected);
            assert!(o.winner_option < 2 && o.drawn && o.draw_seed == seeds[0].to_vec());
            // with the seed on the board nothing depends on the old ceremony any more
            let mut done = log.clone();
            done.push(draw_entry(&m, &kshares[1], &need, &mut rng));
            // (the other contests of this manifest have not even started: they still block)
            let e = run_t(&done).rotation_safe().unwrap_err();
            assert!(
                !e.contains("`presidente`") && e.contains("ainda não terminou"),
                "{e}"
            );
        }

        #[test]
        fn nothing_about_the_distribution_exists_until_enough_validators_decrypt_and_it_adds_up() {
            let k = keys(3, 31);
            let pol = policy(&k, 2);
            let (kshares, record) = ceremony(5);
            let mut m = manifest();
            m.tally_key = record.group_key.clone();
            m.signatures = [0usize, 1]
                .iter()
                .map(|i| sign_manifest(&k[*i], &m))
                .collect();
            let st = steps(&m, &k, PRESIDENT, 0);
            let mut rng = StdRng::seed_from_u64(8);

            // 12 voters; choices over 3 options.
            let choices = [0usize, 2, 2, 1, 2, 0, 2, 1, 2, 2, 1, 2];
            let voted: Vec<Voted> = choices
                .iter()
                .enumerate()
                .map(|(i, c)| enc_ballot(&m, PRESIDENT, i as u8 + 1, *c, &mut rng))
                .collect();
            let mut want = [0u64; 3];
            for c in choices {
                want[c] += 1;
            }
            let sums = sum_ballots(3, &voted.iter().map(|v| &v.eb).collect::<Vec<_>>());

            let mut log: Log = vec![
                (LogKind::Manifest, m.encode_to_vec()),
                (LogKind::Dkg, record.encode_to_vec()),
            ];
            log.extend(st[..3].iter().map(tr));
            for v in &voted {
                log.push((LogKind::Ballot, v.sub.encode_to_vec()));
            }

            // While voting: the count of ballots is known, the distribution is NOT (not even with Live).
            let t = run(&pol, &log);
            let c = t.contest(PRESIDENT).unwrap();
            assert_eq!((c.accepted, c.encrypted, c.revealed), (12, true, false));
            assert!(
                c.options.iter().all(|o| o.votes == 0),
                "no per-option count exists"
            );
            assert!(!c.public_view(ResultsVisibility::Live).distribution);
            assert_eq!(c.rejected.total(), 0);

            // Shares before the close are refused (and counted).
            let early = shares_entry(&m, &kshares[0], &sums, &mut rng);
            let mut l = log.clone();
            l.push(early.clone());
            let t = run(&pol, &l);
            assert_eq!(
                (t.bad_results, t.contest(PRESIDENT).unwrap().revealed),
                (1, false)
            );

            // Close. One validator is not enough; two reveal the exact totals.
            log.push(tr(&st[3]));
            let after_close_ballot = enc_ballot(&m, PRESIDENT, 99, 0, &mut rng);
            log.push((LogKind::Ballot, after_close_ballot.sub.encode_to_vec())); // too late: not counted
            log.push(shares_entry(&m, &kshares[0], &sums, &mut rng));
            let t1 = run(&pol, &log);
            let c = t1.contest(PRESIDENT).unwrap();
            assert_eq!(
                (c.accepted, c.revealed, c.rejected.not_open),
                (12, false, 1),
                "1 of 3 shares: still hidden"
            );
            assert!(!c.public_view(ResultsVisibility::AfterClose).distribution);

            // a duplicate from the same validator, a forged share and a share from an outsider are refused
            let mut l = log.clone();
            l.push(shares_entry(&m, &kshares[0], &sums, &mut rng));
            let bad_secret = KeyShare {
                index: 2,
                secret: kshares[1].secret + Scalar::ONE,
                group_public_key: kshares[1].group_public_key,
                verification_keys: kshares[1].verification_keys.clone(),
            };
            l.push(shares_entry(&m, &bad_secret, &sums, &mut rng));
            let mut out = shares_entry(&m, &kshares[1], &sums, &mut rng);
            let mut r = DecryptionShares::decode(out.1.as_slice()).unwrap();
            r.validator_index = 9;
            out = (LogKind::Result, r.encode_to_vec());
            l.push(out);
            let t = run(&pol, &l);
            assert_eq!(
                (t.bad_results, t.contest(PRESIDENT).unwrap().revealed),
                (3, false)
            );

            // validator 3 completes the quorum (validator 2 was never needed)
            log.push(shares_entry(&m, &kshares[2], &sums, &mut rng));
            let t = run(&pol, &log);
            let c = t.contest(PRESIDENT).unwrap();
            assert!(c.revealed && c.public_view(ResultsVisibility::AfterClose).distribution);
            assert_eq!(c.options.iter().map(|o| o.votes).collect::<Vec<_>>(), want);
            assert_eq!(c.accepted, want.iter().sum::<u64>());
            assert_eq!(t.bad_results, 0);

            // incremental == full replay, step by step
            let mut inc = Tallier::new(pol.clone());
            for (i, (kind, p)) in log.iter().enumerate() {
                inc.feed(LogEntry {
                    kind: *kind,
                    payload: p,
                });
                assert_eq!(inc.snapshot(), run(&pol, &log[..=i].to_vec()), "step {i}");
            }
        }

        /// Feeds the log the way a parallel host does: the expensive checks of each ballot
        /// were computed beforehand (`precheck`, against `against`), then counting is in order.
        fn run_prechecked(
            pol: &GovernancePolicy,
            log: &Log,
            against: &ElectionManifest,
        ) -> ElectionTally {
            let mut t = Tallier::new(pol.clone());
            for (kind, p) in log {
                let pre = (*kind == LogKind::Ballot)
                    .then(|| precheck(against, p).map(Precheck::Ballot))
                    .flatten();
                t.feed_prechecked(
                    LogEntry {
                        kind: *kind,
                        payload: p.as_slice(),
                    },
                    pre,
                );
            }
            t.snapshot()
        }

        #[test]
        fn prechecked_feeding_gives_exactly_the_sequential_result() {
            let k = keys(3, 51);
            let pol = policy(&k, 2);
            let (_, record) = ceremony(7);
            let (_, other_record) = ceremony(8);
            let mut m = manifest();
            m.tally_key = record.group_key.clone();
            m.signatures = [0usize, 1]
                .iter()
                .map(|i| sign_manifest(&k[*i], &m))
                .collect();
            let st = steps(&m, &k, PRESIDENT, 0);
            let mut rng = StdRng::seed_from_u64(12);
            let mut log: Log = vec![
                (LogKind::Manifest, m.encode_to_vec()),
                (LogKind::Dkg, record.encode_to_vec()),
            ];
            log.extend(st[..3].iter().map(tr));
            let resign = |mut sub: SubmittedBallot, voter: u8| {
                let key = derive_voter_key(&[voter; 32], &m.election_id, PRESIDENT).unwrap();
                sub.signature = sign_ballot(&key, &sub.body());
                sub
            };
            // valid ballots, a duplicate of the first voter, and every way to be refused
            for v in 1..=6u8 {
                let b = enc_ballot(&m, PRESIDENT, v, usize::from(v) % 3, &mut rng).sub;
                log.push((LogKind::Ballot, b.encode_to_vec()));
            }
            let again = enc_ballot(&m, PRESIDENT, 1, 2, &mut rng).sub;
            log.push((LogKind::Ballot, again.encode_to_vec())); // duplicate voter
            let mut tampered = enc_ballot(&m, PRESIDENT, 8, 0, &mut rng).sub;
            tampered.encrypted[40] ^= 1;
            log.push((LogKind::Ballot, tampered.encode_to_vec())); // signature broken
            let mut bad_proof = enc_ballot(&m, PRESIDENT, 9, 0, &mut rng).sub;
            bad_proof.encrypted[40] ^= 1;
            log.push((LogKind::Ballot, resign(bad_proof, 9).encode_to_vec())); // proofs broken
            let mut plain = enc_ballot(&m, PRESIDENT, 10, 0, &mut rng).sub;
            plain.encrypted.clear();
            plain.option_index = 1;
            log.push((LogKind::Ballot, resign(plain, 10).encode_to_vec())); // not encrypted
            log.push((LogKind::Ballot, vec![1, 2, 3])); // not even a ballot

            let sequential = run(&pol, &log);
            assert_eq!(sequential.contest(PRESIDENT).unwrap().accepted, 6);
            assert_eq!(run_prechecked(&pol, &log, &m), sequential);

            // verdicts computed against ANOTHER election key are discarded and recomputed
            let mut stale = m.clone();
            stale.tally_key = other_record.group_key.clone();
            assert_eq!(run_prechecked(&pol, &log, &stale), sequential);

            // a verdict that belongs to ANOTHER payload is never applied: hand the check of
            // the ballot at index 5 to the ballot at index 8 (the payload hash differs)
            let mut t = Tallier::new(pol.clone());
            for (i, (kind, p)) in log.iter().enumerate() {
                let given = (i == 8)
                    .then(|| precheck(&m, &log[5].1).map(Precheck::Ballot))
                    .flatten();
                t.feed_prechecked(
                    LogEntry {
                        kind: *kind,
                        payload: p.as_slice(),
                    },
                    given,
                );
            }
            assert_eq!(t.snapshot(), sequential);
        }

        #[test]
        fn invalid_encrypted_ballots_are_rejected_and_never_touch_the_sums() {
            let k = keys(3, 41);
            let pol = policy(&k, 2);
            let (kshares, record) = ceremony(6);
            let mut m = manifest();
            m.tally_key = record.group_key.clone();
            m.signatures = [0usize, 1]
                .iter()
                .map(|i| sign_manifest(&k[*i], &m))
                .collect();
            let st = steps(&m, &k, PRESIDENT, 0);
            let mut rng = StdRng::seed_from_u64(9);
            let good = enc_ballot(&m, PRESIDENT, 1, 1, &mut rng);

            let mut log: Log = vec![
                (LogKind::Manifest, m.encode_to_vec()),
                (LogKind::Dkg, record.encode_to_vec()),
            ];
            log.extend(st[..3].iter().map(tr));
            log.push((LogKind::Ballot, good.sub.encode_to_vec()));
            // another voter's valid ballot, but REPLAYED under a different voter key
            let mut stolen = enc_ballot(&m, PRESIDENT, 2, 0, &mut rng).sub;
            let other_key = derive_voter_key(&[3; 32], &m.election_id, PRESIDENT).unwrap();
            stolen.voter_key = teko_crypto::public_key_bytes(&other_key);
            log.push((LogKind::Ballot, stolen.encode_to_vec()));
            // a tampered ciphertext: the voter's signature no longer covers it
            let mut tampered = enc_ballot(&m, PRESIDENT, 4, 0, &mut rng).sub;
            tampered.encrypted[40] ^= 1;
            log.push((LogKind::Ballot, tampered.encode_to_vec()));
            // a DISHONEST voter signs his own invalid ballot: signature fine, proofs not
            let resign = |mut sub: SubmittedBallot, voter: u8| {
                let key = derive_voter_key(&[voter; 32], &m.election_id, PRESIDENT).unwrap();
                sub.signature = sign_ballot(&key, &sub.body());
                sub
            };
            let mut bad_proof = enc_ballot(&m, PRESIDENT, 7, 0, &mut rng).sub;
            bad_proof.encrypted[40] ^= 1;
            log.push((LogKind::Ballot, resign(bad_proof, 7).encode_to_vec()));
            // a PLAINTEXT ballot in an encrypted election (option named in the clear)
            let mut plain = enc_ballot(&m, PRESIDENT, 5, 0, &mut rng).sub;
            plain.encrypted.clear();
            plain.option_index = 1;
            log.push((LogKind::Ballot, resign(plain, 5).encode_to_vec()));
            // a ballot that names an option next to the ciphertexts
            let mut named = enc_ballot(&m, PRESIDENT, 6, 0, &mut rng).sub;
            named.option_index = 1;
            log.push((LogKind::Ballot, resign(named, 6).encode_to_vec()));
            // the same voter twice
            let again = enc_ballot(&m, PRESIDENT, 1, 2, &mut rng).sub;
            log.push((LogKind::Ballot, again.encode_to_vec()));

            let t = run(&pol, &log);
            let c = t.contest(PRESIDENT).unwrap();
            assert_eq!(c.accepted, 1);
            assert_eq!(
                (
                    c.rejected.bad_proof,
                    c.rejected.bad_signature,
                    c.rejected.duplicate
                ),
                (3, 2, 1)
            );

            // After the close the quorum decrypts: the totals equal ONLY the one valid ballot.
            log.push(tr(&st[3]));
            let sums = sum_ballots(3, &[&good.eb]);
            for ks in [&kshares[0], &kshares[1]] {
                log.push(shares_entry(&m, ks, &sums, &mut rng));
            }
            let t = run(&pol, &log);
            let c = t.contest(PRESIDENT).unwrap();
            assert!(c.revealed);
            assert_eq!(
                c.options.iter().map(|o| o.votes).collect::<Vec<_>>(),
                [0, 1, 0]
            );
        }

        #[test]
        fn a_forged_ceremony_or_a_key_that_is_not_the_elections_reveals_nothing() {
            let k = keys(3, 51);
            let pol = policy(&k, 2);
            let (kshares, record) = ceremony(7);
            let (_, other_record) = ceremony(8);
            let mut m = manifest();
            m.tally_key = record.group_key.clone();
            m.signatures = [0usize, 1]
                .iter()
                .map(|i| sign_manifest(&k[*i], &m))
                .collect();
            let st = steps(&m, &k, PRESIDENT, 0);
            let mut rng = StdRng::seed_from_u64(10);
            let v = enc_ballot(&m, PRESIDENT, 1, 2, &mut rng);
            let sums = sum_ballots(3, &[&v.eb]);

            // the record on the chain is from ANOTHER ceremony (different election key)
            let mut forged = other_record.clone();
            forged.group_key = record.group_key.clone(); // claims our key, packages say otherwise
            let mut log: Log = vec![
                (LogKind::Manifest, m.encode_to_vec()),
                (LogKind::Dkg, forged.encode_to_vec()),
                (LogKind::Dkg, other_record.encode_to_vec()),
            ];
            log.extend(st[..3].iter().map(tr));
            log.push((LogKind::Ballot, v.sub.encode_to_vec()));
            log.push(tr(&st[3]));
            for ks in [&kshares[0], &kshares[1]] {
                log.push(shares_entry(&m, ks, &sums, &mut rng));
            }
            let t = run(&pol, &log);
            let c = t.contest(PRESIDENT).unwrap();
            assert!(!c.revealed, "no ceremony matches this election's key");
            assert!(
                t.bad_results >= 3,
                "forged record + 2 unusable shares: {}",
                t.bad_results
            );

            // with the honest record it works
            let mut ok: Log = vec![
                (LogKind::Manifest, m.encode_to_vec()),
                (LogKind::Dkg, record.encode_to_vec()),
            ];
            ok.extend(log[3..].iter().cloned());
            assert!(run(&pol, &ok).contest(PRESIDENT).unwrap().revealed);
        }
    }
}
