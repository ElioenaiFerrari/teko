use alloc::string::String;
use alloc::vec::Vec;

use prost::Message;
use teko_attestation::SignerSignature;
use teko_attestation::multisig::count_distinct_valid;
use teko_crypto::{SigningKey, public_key_bytes, sign_tagged, tagged_hash, tags};

use crate::error::ElectionError;
use crate::manifest::{ElectionManifest, manifest_hash, verify_manifest};
use crate::policy::GovernancePolicy;
use crate::text::sanitize_text;

/// Lifecycle phases. Forward-only; `Cancelled` is reachable from the
/// non-terminal phases with a stated reason.
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

/// A governance decision to move an election to another phase.
#[derive(Clone, PartialEq, Message)]
pub struct Transition {
    #[prost(string, tag = "1")]
    pub election_id: String,
    /// `manifest_hash` of the manifest this transition applies to.
    #[prost(bytes = "vec", tag = "2")]
    pub manifest_hash: Vec<u8>,
    #[prost(enumeration = "Phase", tag = "3")]
    pub from_phase: i32,
    #[prost(enumeration = "Phase", tag = "4")]
    pub to_phase: i32,
    /// 1, 2, 3 ... with no gaps: a replay or a skipped step is detectable.
    #[prost(uint64, tag = "5")]
    pub seq: u64,
    /// Unix seconds claimed by the signers (not a trusted clock).
    #[prost(uint64, tag = "6")]
    pub at: u64,
    /// Required (and shown) when cancelling.
    #[prost(string, tag = "7")]
    pub reason: String,
    #[prost(message, repeated, tag = "8")]
    pub signatures: Vec<SignerSignature>,
}

/// Where an election currently is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ElectionState {
    pub phase: Phase,
    pub seq: u64,
    pub at: u64,
}

impl ElectionState {
    /// Publishing a valid, signed manifest creates the election in `Draft`.
    pub fn draft() -> Self {
        Self {
            phase: Phase::Draft,
            seq: 0,
            at: 0,
        }
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

fn apply(
    manifest: &ElectionManifest,
    mhash: &[u8; 32],
    state: ElectionState,
    t: &Transition,
    policy: &GovernancePolicy,
) -> Result<ElectionState, ElectionError> {
    if t.election_id != manifest.election_id {
        return Err(ElectionError::WrongElection);
    }
    if t.manifest_hash.as_slice() != mhash.as_slice() {
        return Err(ElectionError::WrongManifest);
    }
    verify_transition(policy, t)?;

    let from = Phase::try_from(t.from_phase).map_err(|_| ElectionError::UnknownPhase)?;
    let to = Phase::try_from(t.to_phase).map_err(|_| ElectionError::UnknownPhase)?;
    let expected = state.seq + 1;
    if t.seq != expected {
        return Err(ElectionError::BadSequence {
            expected,
            got: t.seq,
        });
    }
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

    // Schedule gates: never open early, never close early.
    let s = manifest
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

    Ok(ElectionState {
        phase: to,
        seq: t.seq,
        at: t.at,
    })
}

/// Verifies the manifest (M-of-N) and folds the transitions in order.
/// This is exactly what a device or an auditor runs.
pub fn replay(
    policy: &GovernancePolicy,
    manifest: &ElectionManifest,
    transitions: &[Transition],
) -> Result<ElectionState, ElectionError> {
    verify_manifest(policy, manifest)?;
    let mhash = manifest_hash(manifest);
    let mut state = ElectionState::draft();
    for t in transitions {
        state = apply(manifest, &mhash, state, t, policy)?;
    }
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::fixtures::*;
    use proptest::prelude::*;

    /// Timestamps of the happy path (registration opens at 1000, voting 2000-3000).
    const T: [u64; 6] = [500, 1_000, 2_000, 3_000, 3_100, 3_200];

    fn t(
        m: &ElectionManifest,
        keys: &[SigningKey],
        from: Phase,
        to: Phase,
        seq: u64,
        at: u64,
    ) -> Transition {
        let mut tr = Transition {
            election_id: m.election_id.clone(),
            manifest_hash: manifest_hash(m).to_vec(),
            from_phase: from as i32,
            to_phase: to as i32,
            seq,
            at,
            reason: String::new(),
            signatures: Vec::new(),
        };
        tr.signatures = [0usize, 1]
            .iter()
            .map(|i| sign_transition(&keys[*i], &tr))
            .collect();
        tr
    }

    /// Draft -> ... -> Audited with valid timestamps.
    fn happy_path(m: &ElectionManifest, k: &[SigningKey]) -> Vec<Transition> {
        let steps = [
            (Phase::Draft, Phase::CandidatesPublished),
            (Phase::CandidatesPublished, Phase::RegistrationOpen),
            (Phase::RegistrationOpen, Phase::VotingOpen),
            (Phase::VotingOpen, Phase::VotingClosed),
            (Phase::VotingClosed, Phase::Tallied),
            (Phase::Tallied, Phase::Audited),
        ];
        steps
            .iter()
            .enumerate()
            .map(|(i, (f, to))| t(m, k, *f, *to, i as u64 + 1, T[i]))
            .collect()
    }

    fn setup(seed: u64) -> (Vec<SigningKey>, GovernancePolicy, ElectionManifest) {
        let k = keys(3, seed);
        let pol = policy(&k, 2);
        let m = signed(&k, &[0, 1]);
        (k, pol, m)
    }

    #[test]
    fn full_lifecycle_replays_to_audited() {
        let (k, pol, m) = setup(1);
        let s = replay(&pol, &m, &happy_path(&m, &k)).unwrap();
        assert_eq!((s.phase, s.seq), (Phase::Audited, 6));
        assert!(s.phase.is_terminal());
        // Midway the phase gates behave.
        let mid = replay(&pol, &m, &happy_path(&m, &k)[..3]).unwrap();
        assert_eq!(mid.phase, Phase::VotingOpen);
        assert!(mid.phase.can_vote() && mid.phase.can_register());
        assert!(!Phase::VotingClosed.can_vote());
    }

    #[test]
    fn an_unsigned_or_underspecified_manifest_creates_no_election() {
        let (k, pol, _) = setup(2);
        let weak = signed(&k, &[0]);
        assert!(matches!(
            replay(&pol, &weak, &[]),
            Err(ElectionError::InsufficientSignatures {
                valid: 1,
                required: 2
            })
        ));
    }

    #[test]
    fn skipping_going_back_and_repeating_are_refused() {
        let (k, pol, m) = setup(3);
        let hp = happy_path(&m, &k);
        // skip CandidatesPublished
        let skip = t(&m, &k, Phase::Draft, Phase::RegistrationOpen, 1, 1_000);
        assert!(matches!(
            replay(&pol, &m, &[skip]),
            Err(ElectionError::IllegalTransition { .. })
        ));
        // go back
        let mut seq = hp[..3].to_vec();
        seq.push(t(
            &m,
            &k,
            Phase::VotingOpen,
            Phase::RegistrationOpen,
            4,
            2_500,
        ));
        assert!(matches!(
            replay(&pol, &m, &seq),
            Err(ElectionError::IllegalTransition { .. })
        ));
        // replay the same transition twice
        let twice = vec![hp[0].clone(), hp[0].clone()];
        assert!(matches!(
            replay(&pol, &m, &twice),
            Err(ElectionError::BadSequence {
                expected: 2,
                got: 1
            })
        ));
        // sequence gap
        let gap = vec![
            hp[0].clone(),
            t(
                &m,
                &k,
                Phase::CandidatesPublished,
                Phase::RegistrationOpen,
                3,
                1_000,
            ),
        ];
        assert!(matches!(
            replay(&pol, &m, &gap),
            Err(ElectionError::BadSequence {
                expected: 2,
                got: 3
            })
        ));
        // wrong `from`
        let bad_from = t(&m, &k, Phase::VotingOpen, Phase::VotingClosed, 1, 3_000);
        assert_eq!(
            replay(&pol, &m, &[bad_from]),
            Err(ElectionError::WrongFromPhase)
        );
    }

    #[test]
    fn transitions_are_bound_to_one_election_and_one_manifest() {
        let (k, pol, m) = setup(4);
        let mut other_election = hp0(&m, &k);
        other_election.election_id = "outra-eleicao".into();
        other_election.signatures = [0usize, 1]
            .iter()
            .map(|i| sign_transition(&k[*i], &other_election))
            .collect();
        assert_eq!(
            replay(&pol, &m, &[other_election]),
            Err(ElectionError::WrongElection)
        );

        // A revised manifest has another hash: old transitions do not apply to it.
        let mut revised = m.clone();
        revised.revision = 2;
        revised.signatures = [0usize, 1]
            .iter()
            .map(|i| crate::manifest::sign_manifest(&k[*i], &revised))
            .collect();
        assert_eq!(
            replay(&pol, &revised, &[hp0(&m, &k)]),
            Err(ElectionError::WrongManifest)
        );
    }

    fn hp0(m: &ElectionManifest, k: &[SigningKey]) -> Transition {
        t(m, k, Phase::Draft, Phase::CandidatesPublished, 1, 600)
    }

    #[test]
    fn every_transition_needs_m_of_n_and_tampering_breaks_it() {
        let (k, pol, m) = setup(5);
        let mut one_sig = hp0(&m, &k);
        one_sig.signatures.truncate(1);
        assert!(matches!(
            replay(&pol, &m, &[one_sig]),
            Err(ElectionError::InsufficientSignatures { .. })
        ));
        let mut tampered = hp0(&m, &k);
        tampered.at += 1; // signed over the old timestamp
        assert!(matches!(
            replay(&pol, &m, &[tampered]),
            Err(ElectionError::InsufficientSignatures { .. })
        ));
    }

    #[test]
    fn schedule_gates_opening_and_closing() {
        let (k, pol, m) = setup(6);
        let hp = happy_path(&m, &k);
        // open registration before its scheduled time
        let early_reg = t(
            &m,
            &k,
            Phase::CandidatesPublished,
            Phase::RegistrationOpen,
            2,
            999,
        );
        assert_eq!(
            replay(&pol, &m, &[hp[0].clone(), early_reg]),
            Err(ElectionError::TooEarly)
        );
        // close voting before the scheduled end
        let early_close = t(&m, &k, Phase::VotingOpen, Phase::VotingClosed, 4, 2_999);
        let seq = vec![hp[0].clone(), hp[1].clone(), hp[2].clone(), early_close];
        assert_eq!(replay(&pol, &m, &seq), Err(ElectionError::TooEarly));
        // timestamps cannot go backwards
        let back = t(
            &m,
            &k,
            Phase::CandidatesPublished,
            Phase::RegistrationOpen,
            2,
            400,
        );
        assert!(matches!(
            replay(&pol, &m, &[hp[0].clone(), back]),
            Err(ElectionError::TimeTravel)
        ));
    }

    #[test]
    fn cancellation_needs_a_reason_and_ends_the_election() {
        let (k, pol, m) = setup(7);
        let hp = happy_path(&m, &k);
        let mut cancel = t(&m, &k, Phase::VotingOpen, Phase::Cancelled, 4, 2_500);
        let base = hp[..3].to_vec();
        // no reason
        let mut seq = base.clone();
        seq.push(cancel.clone());
        assert_eq!(replay(&pol, &m, &seq), Err(ElectionError::MissingReason));
        // with reason
        cancel.reason = "Falha grave de segurança detectada".into();
        cancel.signatures = [0usize, 1]
            .iter()
            .map(|i| sign_transition(&k[*i], &cancel))
            .collect();
        let mut seq = base.clone();
        seq.push(cancel.clone());
        let s = replay(&pol, &m, &seq).unwrap();
        assert_eq!(s.phase, Phase::Cancelled);
        assert!(!s.phase.can_vote() && !s.phase.can_register());
        // nothing happens after cancellation
        let after = t(&m, &k, Phase::Cancelled, Phase::VotingClosed, 5, 3_000);
        seq.push(after);
        assert!(matches!(
            replay(&pol, &m, &seq),
            Err(ElectionError::IllegalTransition { .. })
        ));
        // cannot cancel once tallied
        let mut tallied = hp[..5].to_vec();
        let mut late = t(&m, &k, Phase::Tallied, Phase::Cancelled, 6, 3_250);
        late.reason = "tarde demais".into();
        late.signatures = [0usize, 1]
            .iter()
            .map(|i| sign_transition(&k[*i], &late))
            .collect();
        tallied.push(late);
        assert!(matches!(
            replay(&pol, &m, &tallied),
            Err(ElectionError::IllegalTransition { .. })
        ));
    }

    proptest! {
        /// Model check: from any phase, an (otherwise valid) transition is
        /// accepted iff it is the single forward step or a permitted cancel.
        #[test]
        fn only_legal_edges_are_accepted(seed in any::<u64>(), from in 0i32..8, to in 0i32..8) {
            let (k, pol, m) = setup(seed);
            let hp = happy_path(&m, &k);
            let from_p = Phase::try_from(from).unwrap();
            let to_p = Phase::try_from(to).unwrap();
            // Reach `from` through the happy path (Cancelled via a reasoned cancel).
            let (prefix, seq_no, at) = match from_p {
                Phase::Draft => (vec![], 1, 3_300),
                Phase::Cancelled => {
                    let mut c = t(&m, &k, Phase::Draft, Phase::Cancelled, 1, 100);
                    c.reason = "motivo".into();
                    c.signatures = [0usize, 1].iter().map(|i| sign_transition(&k[*i], &c)).collect();
                    (vec![c], 2, 3_300)
                }
                p => (hp[..(p as usize)].to_vec(), p as u64 + 1, 3_300),
            };
            let mut tr = t(&m, &k, from_p, to_p, seq_no, at);
            if to_p == Phase::Cancelled {
                tr.reason = "motivo".into();
                tr.signatures = [0usize, 1].iter().map(|i| sign_transition(&k[*i], &tr)).collect();
            }
            let mut all = prefix;
            all.push(tr);
            let legal = from_p.next() == Some(to_p) || (to_p == Phase::Cancelled && from_p.can_cancel());
            prop_assert_eq!(replay(&pol, &m, &all).is_ok(), legal, "{:?} -> {:?}", from_p, to_p);
        }
    }
}
