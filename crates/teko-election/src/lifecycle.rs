use alloc::string::String;
use alloc::vec::Vec;

use prost::Message;
use teko_attestation::SignerSignature;
use teko_attestation::multisig::count_distinct_valid;
use teko_crypto::{SigningKey, public_key_bytes, sign_tagged, tagged_hash, tags};

use crate::error::ElectionError;
use crate::manifest::{
    Contest, ElectionManifest, contest_hash, verify_manifest, verify_manifest_chain,
};
use crate::policy::GovernancePolicy;
use crate::text::sanitize_text;

/// Lifecycle phases of ONE contest. Forward-only; `Cancelled` is reachable from
/// the non-terminal phases with a stated reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, prost::Enumeration)]
#[repr(i32)]
pub enum Phase {
    Draft = 0,
    CandidatesPublished = 1,
    RegistrationOpen = 2,
    VotingOpen = 3,
    VotingClosed = 4,
    Tallied = 5,
    Audited = 6,
    Cancelled = 7,
}

impl Phase {
    /// The single legal forward step, if any.
    pub fn next(self) -> Option<Phase> {
        match self {
            Phase::Draft => Some(Phase::CandidatesPublished),
            Phase::CandidatesPublished => Some(Phase::RegistrationOpen),
            Phase::RegistrationOpen => Some(Phase::VotingOpen),
            Phase::VotingOpen => Some(Phase::VotingClosed),
            Phase::VotingClosed => Some(Phase::Tallied),
            Phase::Tallied => Some(Phase::Audited),
            Phase::Audited | Phase::Cancelled => None,
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Phase::Audited | Phase::Cancelled)
    }

    /// Cancellation is possible until the tally is published.
    pub fn can_cancel(self) -> bool {
        matches!(
            self,
            Phase::Draft
                | Phase::CandidatesPublished
                | Phase::RegistrationOpen
                | Phase::VotingOpen
                | Phase::VotingClosed
        )
    }

    pub fn can_register(self) -> bool {
        matches!(self, Phase::RegistrationOpen | Phase::VotingOpen)
    }

    pub fn can_vote(self) -> bool {
        self == Phase::VotingOpen
    }
}

/// A governance decision to move ONE contest to another phase.
#[derive(Clone, PartialEq, Message)]
pub struct Transition {
    #[prost(string, tag = "1")]
    pub election_id: String,
    #[prost(string, tag = "2")]
    pub contest_id: String,
    /// `contest_hash` of the contest as published: binds the decision to those
    /// exact candidates, scope and dates.
    #[prost(bytes = "vec", tag = "3")]
    pub contest_hash: Vec<u8>,
    #[prost(enumeration = "Phase", tag = "4")]
    pub from_phase: i32,
    #[prost(enumeration = "Phase", tag = "5")]
    pub to_phase: i32,
    /// Election-wide order of governance actions: 1, 2, 3 ... with no gaps, so a
    /// replay, a skipped step or a hidden decision is detectable.
    #[prost(uint64, tag = "6")]
    pub seq: u64,
    /// Unix seconds claimed by the signers (not a trusted clock). Must not go
    /// backwards WITHIN a contest; different contests keep their own dates.
    #[prost(uint64, tag = "7")]
    pub at: u64,
    /// Required (and shown) when cancelling.
    #[prost(string, tag = "8")]
    pub reason: String,
    #[prost(message, repeated, tag = "9")]
    pub signatures: Vec<SignerSignature>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContestState {
    pub contest_id: String,
    pub phase: Phase,
    pub at: u64,
}

/// Where every contest of an election currently is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ElectionStatus {
    /// Sequence number of the last applied transition (0 = none).
    pub seq: u64,
    pub contests: Vec<ContestState>,
}

impl ElectionStatus {
    /// Publishing a valid, signed manifest creates every contest in `Draft`.
    pub fn new(manifest: &ElectionManifest) -> Self {
        Self {
            seq: 0,
            contests: manifest
                .contests
                .iter()
                .map(|c| ContestState {
                    contest_id: c.contest_id.clone(),
                    phase: Phase::Draft,
                    at: 0,
                })
                .collect(),
        }
    }

    /// A later manifest revision may add contests: start them in `Draft`.
    pub fn extend_for(&mut self, manifest: &ElectionManifest) {
        for c in &manifest.contests {
            if !self.contests.iter().any(|s| s.contest_id == c.contest_id) {
                self.contests.push(ContestState {
                    contest_id: c.contest_id.clone(),
                    phase: Phase::Draft,
                    at: 0,
                });
            }
        }
    }

    pub fn phase_of(&self, contest_id: &str) -> Option<Phase> {
        self.contests
            .iter()
            .find(|c| c.contest_id == contest_id)
            .map(|c| c.phase)
    }
}

fn signing_bytes(t: &Transition) -> Vec<u8> {
    let mut unsigned = t.clone();
    unsigned.signatures = Vec::new();
    unsigned.encode_to_vec()
}

/// Hash identifying a transition on the bulletin board.
pub fn transition_hash(t: &Transition) -> [u8; 32] {
    tagged_hash(tags::ELECTION_TRANSITION, &[&signing_bytes(t)])
}

pub fn sign_transition(key: &SigningKey, t: &Transition) -> SignerSignature {
    SignerSignature {
        signer: public_key_bytes(key),
        signature: sign_tagged(key, tags::ELECTION_TRANSITION, &signing_bytes(t)),
    }
}

/// M-of-N check on one transition (no state rules).
pub fn verify_transition(policy: &GovernancePolicy, t: &Transition) -> Result<(), ElectionError> {
    let valid = count_distinct_valid(
        policy.signers(),
        tags::ELECTION_TRANSITION,
        &signing_bytes(t),
        &t.signatures,
    );
    if valid < policy.threshold() {
        return Err(ElectionError::InsufficientSignatures {
            valid,
            required: policy.threshold(),
        });
    }
    Ok(())
}

pub(crate) fn apply(
    manifest: &ElectionManifest,
    status: &mut ElectionStatus,
    t: &Transition,
    policy: &GovernancePolicy,
) -> Result<(), ElectionError> {
    if t.election_id != manifest.election_id {
        return Err(ElectionError::WrongElection);
    }
    let contest: &Contest = manifest
        .contest(&t.contest_id)
        .ok_or_else(|| ElectionError::UnknownContest(t.contest_id.clone()))?;
    if t.contest_hash.as_slice() != contest_hash(contest).as_slice() {
        return Err(ElectionError::WrongContest);
    }
    verify_transition(policy, t)?;

    let from = Phase::try_from(t.from_phase).map_err(|_| ElectionError::UnknownPhase)?;
    let to = Phase::try_from(t.to_phase).map_err(|_| ElectionError::UnknownPhase)?;
    let expected = status.seq + 1;
    if t.seq != expected {
        return Err(ElectionError::BadSequence {
            expected,
            got: t.seq,
        });
    }
    let state = status
        .contests
        .iter_mut()
        .find(|c| c.contest_id == t.contest_id)
        .ok_or_else(|| ElectionError::UnknownContest(t.contest_id.clone()))?;
    if from != state.phase {
        return Err(ElectionError::WrongFromPhase);
    }
    if t.at < state.at {
        return Err(ElectionError::TimeTravel);
    }

    if to == Phase::Cancelled {
        if !from.can_cancel() {
            return Err(ElectionError::IllegalTransition { from, to });
        }
        if sanitize_text(&t.reason, 200).is_none() {
            return Err(ElectionError::MissingReason);
        }
    } else if from.next() != Some(to) {
        return Err(ElectionError::IllegalTransition { from, to });
    }

    // Schedule gates, per contest: never open early, never close early.
    let s = contest
        .schedule
        .as_ref()
        .ok_or(ElectionError::InvalidManifest("missing schedule"))?;
    let earliest = match to {
        Phase::RegistrationOpen => s.registration_opens,
        Phase::VotingOpen => s.voting_opens,
        Phase::VotingClosed => s.voting_closes,
        _ => 0,
    };
    if t.at < earliest {
        return Err(ElectionError::TooEarly);
    }

    state.phase = to;
    state.at = t.at;
    status.seq = t.seq;
    Ok(())
}

/// Verifies the manifest (M-of-N) and folds the transitions in order.
/// This is exactly what a device or an auditor runs.
pub fn replay(
    policy: &GovernancePolicy,
    manifest: &ElectionManifest,
    transitions: &[Transition],
) -> Result<ElectionStatus, ElectionError> {
    verify_manifest(policy, manifest)?;
    let mut status = ElectionStatus::new(manifest);
    for t in transitions {
        apply(manifest, &mut status, t, policy)?;
    }
    Ok(status)
}

/// Like `replay`, but for an election that went through several manifest
/// revisions (e.g. a run-off added later). Transitions bind to each CONTEST's
/// hash, so history recorded under revision 1 stays valid under revision 2.
pub fn replay_chain(
    policy: &GovernancePolicy,
    revisions: &[ElectionManifest],
    transitions: &[Transition],
) -> Result<ElectionStatus, ElectionError> {
    let latest = verify_manifest_chain(policy, revisions)?;
    let mut status = ElectionStatus::new(latest);
    for t in transitions {
        apply(latest, &mut status, t, policy)?;
    }
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::fixtures::*;
    use crate::manifest::{sign_manifest, validate_manifest};
    use proptest::prelude::*;

    fn world(seed: u64) -> (Vec<SigningKey>, GovernancePolicy, ElectionManifest) {
        let k = keys(3, seed);
        let pol = policy(&k, 2);
        let m = signed(&k, &[0, 1]);
        (k, pol, m)
    }

    fn tr(
        m: &ElectionManifest,
        k: &[SigningKey],
        contest: &str,
        from: Phase,
        to: Phase,
        seq: u64,
        at: u64,
    ) -> Transition {
        let c = m.contest(contest).unwrap();
        let mut t = Transition {
            election_id: m.election_id.clone(),
            contest_id: contest.into(),
            contest_hash: contest_hash(c).to_vec(),
            from_phase: from as i32,
            to_phase: to as i32,
            seq,
            at,
            reason: String::new(),
            signatures: Vec::new(),
        };
        t.signatures = [0usize, 1]
            .iter()
            .map(|i| sign_transition(&k[*i], &t))
            .collect();
        t
    }

    /// Transitions of ONE contest, up to (and including) `upto`, at that
    /// contest's OWN dates. `seq0` = last election-wide seq before these.
    fn path(
        m: &ElectionManifest,
        k: &[SigningKey],
        contest: &str,
        upto: Phase,
        seq0: u64,
    ) -> Vec<Transition> {
        let s = m.contest(contest).unwrap().schedule.clone().unwrap();
        let steps = [
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
            (Phase::VotingClosed, Phase::Tallied, s.voting_closes + 100),
            (Phase::Tallied, Phase::Audited, s.voting_closes + 200),
        ];
        let mut out = Vec::new();
        for (f, to, at) in steps {
            if f as i32 >= upto as i32 {
                break;
            }
            out.push(tr(m, k, contest, f, to, seq0 + out.len() as u64 + 1, at));
        }
        out
    }

    #[test]
    fn contests_run_independently_with_their_own_dates() {
        let (k, pol, m) = world(1);
        let mut all = path(&m, &k, PRESIDENT, Phase::Audited, 0);
        all.extend(path(&m, &k, GOVERNOR_SP, Phase::Audited, all.len() as u64));
        all.extend(path(&m, &k, MAYOR_SP, Phase::VotingOpen, all.len() as u64));
        let s = replay(&pol, &m, &all).unwrap();
        assert_eq!(s.seq, 6 + 6 + 3);
        assert_eq!(s.phase_of(PRESIDENT), Some(Phase::Audited));
        assert_eq!(s.phase_of(GOVERNOR_SP), Some(Phase::Audited));
        assert_eq!(
            s.phase_of(MAYOR_SP),
            Some(Phase::VotingOpen),
            "mayor voting is open on ITS own dates"
        );
        assert!(s.phase_of(PRESIDENT).unwrap().is_terminal());
        assert_eq!(s.phase_of("nao-existe"), None);
        // The mayor's contest can still be voted on later, on ITS dates, while others are done.
        let mut seq_fix = all.clone();
        seq_fix.truncate(12);
        seq_fix.extend(path(&m, &k, MAYOR_SP, Phase::VotingClosed, 12));
        let s = replay(&pol, &m, &seq_fix).unwrap();
        assert_eq!(s.phase_of(MAYOR_SP), Some(Phase::VotingClosed));
        assert!(!Phase::VotingClosed.can_vote());
    }

    #[test]
    fn opening_one_contest_does_not_open_another_and_dates_are_per_contest() {
        let (k, pol, m) = world(2);
        let mut seq = path(&m, &k, PRESIDENT, Phase::VotingOpen, 0);
        let s = replay(&pol, &m, &seq).unwrap();
        assert_eq!(s.phase_of(PRESIDENT), Some(Phase::VotingOpen));
        assert_eq!(
            s.phase_of(GOVERNOR_SP),
            Some(Phase::Draft),
            "governor untouched"
        );
        // The governor's registration opens at 1_500: 1_499 is too early even
        // though the President's contest is already further along.
        let n = seq.len() as u64;
        seq.push(tr(
            &m,
            &k,
            GOVERNOR_SP,
            Phase::Draft,
            Phase::CandidatesPublished,
            n + 1,
            1_000,
        ));
        seq.push(tr(
            &m,
            &k,
            GOVERNOR_SP,
            Phase::CandidatesPublished,
            Phase::RegistrationOpen,
            n + 2,
            1_499,
        ));
        assert_eq!(replay(&pol, &m, &seq), Err(ElectionError::TooEarly));
        // The mayor votes only from 4_000: opening it at 3_999 is refused.
        let mut mayor = path(&m, &k, MAYOR_SP, Phase::RegistrationOpen, 0);
        mayor.push(tr(
            &m,
            &k,
            MAYOR_SP,
            Phase::RegistrationOpen,
            Phase::VotingOpen,
            3,
            3_999,
        ));
        assert_eq!(replay(&pol, &m, &mayor), Err(ElectionError::TooEarly));
    }

    #[test]
    fn transitions_are_bound_to_one_exact_contest() {
        let (k, pol, m) = world(3);
        // President's decision pointing at the governor's contest hash
        let mut t = tr(
            &m,
            &k,
            PRESIDENT,
            Phase::Draft,
            Phase::CandidatesPublished,
            1,
            900,
        );
        t.contest_hash = contest_hash(m.contest(GOVERNOR_SP).unwrap()).to_vec();
        t.signatures = [0usize, 1]
            .iter()
            .map(|i| sign_transition(&k[*i], &t))
            .collect();
        assert_eq!(replay(&pol, &m, &[t]), Err(ElectionError::WrongContest));
        // unknown contest
        let mut u = tr(
            &m,
            &k,
            PRESIDENT,
            Phase::Draft,
            Phase::CandidatesPublished,
            1,
            900,
        );
        u.contest_id = "senador-sp".into();
        u.signatures = [0usize, 1]
            .iter()
            .map(|i| sign_transition(&k[*i], &u))
            .collect();
        assert!(matches!(
            replay(&pol, &m, &[u]),
            Err(ElectionError::UnknownContest(_))
        ));
        // other election
        let mut e = tr(
            &m,
            &k,
            PRESIDENT,
            Phase::Draft,
            Phase::CandidatesPublished,
            1,
            900,
        );
        e.election_id = "outra".into();
        e.signatures = [0usize, 1]
            .iter()
            .map(|i| sign_transition(&k[*i], &e))
            .collect();
        assert_eq!(replay(&pol, &m, &[e]), Err(ElectionError::WrongElection));
    }

    #[test]
    fn unsigned_manifest_creates_no_election_and_transitions_need_m_of_n() {
        let (k, pol, _) = world(4);
        let weak = signed(&k, &[0]);
        assert!(matches!(
            replay(&pol, &weak, &[]),
            Err(ElectionError::InsufficientSignatures {
                valid: 1,
                required: 2
            })
        ));
        let (_, _, m) = world(4);
        let mut one = tr(
            &m,
            &k,
            PRESIDENT,
            Phase::Draft,
            Phase::CandidatesPublished,
            1,
            900,
        );
        one.signatures.truncate(1);
        assert!(matches!(
            replay(&pol, &m, &[one]),
            Err(ElectionError::InsufficientSignatures { .. })
        ));
        let mut tampered = tr(
            &m,
            &k,
            PRESIDENT,
            Phase::Draft,
            Phase::CandidatesPublished,
            1,
            900,
        );
        tampered.at += 1;
        assert!(matches!(
            replay(&pol, &m, &[tampered]),
            Err(ElectionError::InsufficientSignatures { .. })
        ));
    }

    #[test]
    fn skipping_going_back_repeating_and_sequence_gaps_are_refused() {
        let (k, pol, m) = world(5);
        let hp = path(&m, &k, PRESIDENT, Phase::VotingOpen, 0);
        let skip = tr(
            &m,
            &k,
            PRESIDENT,
            Phase::Draft,
            Phase::RegistrationOpen,
            1,
            1_000,
        );
        assert!(matches!(
            replay(&pol, &m, &[skip]),
            Err(ElectionError::IllegalTransition { .. })
        ));
        let mut back = hp.clone();
        back.push(tr(
            &m,
            &k,
            PRESIDENT,
            Phase::VotingOpen,
            Phase::RegistrationOpen,
            4,
            2_500,
        ));
        assert!(matches!(
            replay(&pol, &m, &back),
            Err(ElectionError::IllegalTransition { .. })
        ));
        assert!(matches!(
            replay(&pol, &m, &[hp[0].clone(), hp[0].clone()]),
            Err(ElectionError::BadSequence {
                expected: 2,
                got: 1
            })
        ));
        let gap = tr(
            &m,
            &k,
            PRESIDENT,
            Phase::CandidatesPublished,
            Phase::RegistrationOpen,
            3,
            1_000,
        );
        assert!(matches!(
            replay(&pol, &m, &[hp[0].clone(), gap]),
            Err(ElectionError::BadSequence {
                expected: 2,
                got: 3
            })
        ));
        let bad_from = tr(
            &m,
            &k,
            PRESIDENT,
            Phase::VotingOpen,
            Phase::VotingClosed,
            1,
            3_000,
        );
        assert_eq!(
            replay(&pol, &m, &[bad_from]),
            Err(ElectionError::WrongFromPhase)
        );
        // time cannot go backwards WITHIN a contest
        let back_in_time = tr(
            &m,
            &k,
            PRESIDENT,
            Phase::CandidatesPublished,
            Phase::RegistrationOpen,
            2,
            400,
        );
        assert_eq!(
            replay(&pol, &m, &[hp[0].clone(), back_in_time]),
            Err(ElectionError::TimeTravel)
        );
        // ...but a DIFFERENT contest may carry an earlier date (no global clock)
        let gov = tr(
            &m,
            &k,
            GOVERNOR_SP,
            Phase::Draft,
            Phase::CandidatesPublished,
            2,
            100,
        );
        assert!(replay(&pol, &m, &[hp[0].clone(), gov]).is_ok());
    }

    #[test]
    fn cancelling_one_contest_leaves_the_others_running() {
        let (k, pol, m) = world(6);
        let mut seq = path(&m, &k, PRESIDENT, Phase::VotingOpen, 0);
        let mut cancel = tr(&m, &k, GOVERNOR_SP, Phase::Draft, Phase::Cancelled, 4, 100);
        let mut no_reason = seq.clone();
        no_reason.push(cancel.clone());
        assert_eq!(
            replay(&pol, &m, &no_reason),
            Err(ElectionError::MissingReason)
        );
        cancel.reason = "Candidatura contestada judicialmente".into();
        cancel.signatures = [0usize, 1]
            .iter()
            .map(|i| sign_transition(&k[*i], &cancel))
            .collect();
        seq.push(cancel);
        let s = replay(&pol, &m, &seq).unwrap();
        assert_eq!(s.phase_of(GOVERNOR_SP), Some(Phase::Cancelled));
        assert_eq!(
            s.phase_of(PRESIDENT),
            Some(Phase::VotingOpen),
            "president unaffected"
        );
        // nothing happens to a cancelled contest afterwards
        let mut after = seq.clone();
        after.push(tr(
            &m,
            &k,
            GOVERNOR_SP,
            Phase::Cancelled,
            Phase::CandidatesPublished,
            5,
            200,
        ));
        assert!(matches!(
            replay(&pol, &m, &after),
            Err(ElectionError::IllegalTransition { .. })
        ));
        // cannot cancel after the tally
        let mut late = path(&m, &k, PRESIDENT, Phase::Tallied, 0);
        late.push({
            let mut c = tr(
                &m,
                &k,
                PRESIDENT,
                Phase::Tallied,
                Phase::Cancelled,
                6,
                3_250,
            );
            c.reason = "tarde demais".into();
            c.signatures = [0usize, 1]
                .iter()
                .map(|i| sign_transition(&k[*i], &c))
                .collect();
            c
        });
        assert!(matches!(
            replay(&pol, &m, &late),
            Err(ElectionError::IllegalTransition { .. })
        ));
    }

    #[test]
    fn a_runoff_added_in_a_later_revision_keeps_earlier_history_valid() {
        let (k, pol, rev1) = world(7);
        let history = path(&rev1, &k, MAYOR_SP, Phase::Tallied, 0);

        let mut rev2 = rev1.clone();
        rev2.revision = 2;
        rev2.contests.push(Contest {
            contest_id: "prefeito-3550308-2turno".into(),
            office: crate::manifest::Office::Mayor as i32,
            title: "Prefeito de São Paulo (2º turno)".into(),
            scope: Some(crate::eligibility::Scope::municipality("3550308")),
            options: rev1.contests[2].options.clone(),
            registration_key: Vec::new(),
            schedule: Some(crate::manifest::Schedule {
                registration_opens: 6_000,
                voting_opens: 7_000,
                voting_closes: 8_000,
            }),
        });
        validate_manifest(&rev2).unwrap();
        rev2.signatures = [0usize, 1]
            .iter()
            .map(|i| sign_manifest(&k[*i], &rev2))
            .collect();

        // Round-one history recorded under revision 1 still replays under the chain...
        let s = replay_chain(&pol, &[rev1.clone(), rev2.clone()], &history).unwrap();
        assert_eq!(s.phase_of(MAYOR_SP), Some(Phase::Tallied));
        assert_eq!(s.phase_of("prefeito-3550308-2turno"), Some(Phase::Draft));
        // ...and the run-off proceeds on its own dates.
        let mut with_runoff = history.clone();
        with_runoff.push(tr(
            &rev2,
            &k,
            "prefeito-3550308-2turno",
            Phase::Draft,
            Phase::CandidatesPublished,
            history.len() as u64 + 1,
            5_900,
        ));
        assert!(replay_chain(&pol, &[rev1.clone(), rev2.clone()], &with_runoff).is_ok());
        // A revision that alters an existing contest is refused by the chain.
        let mut evil = rev2.clone();
        evil.contests[0].options[0].name = "Candidato Trocado".into();
        evil.signatures = [0usize, 1]
            .iter()
            .map(|i| sign_manifest(&k[*i], &evil))
            .collect();
        assert!(matches!(
            replay_chain(&pol, &[rev1.clone(), evil], &history),
            Err(ElectionError::InvalidRevision(_))
        ));
        // Revision 2 alone (no chain) must not be accepted as a first publication.
        assert!(matches!(
            replay_chain(&pol, &[rev2.clone()], &[]),
            Err(ElectionError::InvalidRevision(_))
        ));
        assert!(matches!(
            replay_chain(&pol, &[], &[]),
            Err(ElectionError::InvalidRevision(_))
        ));
    }

    proptest! {
        /// Model check: from any phase, an (otherwise valid) transition of a
        /// contest is accepted iff it is the single forward step or a permitted cancel.
        #[test]
        fn only_legal_edges_are_accepted(seed in any::<u64>(), from in 0i32..8, to in 0i32..8, which in 0usize..3) {
            let (k, pol, m) = world(seed);
            let contest = [PRESIDENT, GOVERNOR_SP, MAYOR_SP][which];
            let from_p = Phase::try_from(from).unwrap();
            let to_p = Phase::try_from(to).unwrap();
            let (prefix, seq_no) = match from_p {
                Phase::Cancelled => {
                    let mut c = tr(&m, &k, contest, Phase::Draft, Phase::Cancelled, 1, 100);
                    c.reason = "motivo".into();
                    c.signatures = [0usize, 1].iter().map(|i| sign_transition(&k[*i], &c)).collect();
                    (vec![c], 2)
                }
                p => (path(&m, &k, contest, p, 0), p as u64 + 1),
            };
            let mut t = tr(&m, &k, contest, from_p, to_p, seq_no, 99_999);
            if to_p == Phase::Cancelled {
                t.reason = "motivo".into();
                t.signatures = [0usize, 1].iter().map(|i| sign_transition(&k[*i], &t)).collect();
            }
            let mut all = prefix;
            all.push(t);
            let legal = from_p.next() == Some(to_p) || (to_p == Phase::Cancelled && from_p.can_cancel());
            prop_assert_eq!(replay(&pol, &m, &all).is_ok(), legal, "{} {:?} -> {:?}", contest, from_p, to_p);
        }
    }
}
