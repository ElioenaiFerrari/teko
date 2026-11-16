//! The RESULT of one contest, decided by a pure rule and certified by the institutions
//! (ADR-0040). Each institution of the vote chain computes the outcome from its own verified tally
//! and signs it; more than 2/3 signing the same bytes is what the economy ledger accepts to issue
//! the `eleito` credential. The vote chain itself only orders: the safety is the quorum.

use alloc::string::String;
use alloc::vec::Vec;

use prost::Message;
use teko_attestation::SignerSignature;
use teko_attestation::multisig::count_distinct_valid;
use teko_crypto::{SigningKey, public_key_bytes, sign_tagged, tags};

use crate::eligibility::Scope;
use crate::lifecycle::Phase;
use crate::manifest::{Contest, Office, OptionKind};
use crate::tally::ContestTally;

#[derive(Clone, Copy, Debug, PartialEq, Eq, prost::Enumeration)]
#[repr(i32)]
pub enum OutcomeKind {
    Unspecified = 0,
    /// One candidate wins: `winner_*` is set.
    Elected = 1,
    /// Nobody has an absolute majority: a SECOND ROUND between `finalists`.
    Runoff = 2,
    /// A tie the rule cannot break (PENDING: governance decides). Nobody is elected.
    Tie = 3,
}

#[derive(Clone, PartialEq, Message)]
pub struct ElectionOutcome {
    #[prost(string, tag = "1")]
    pub vote_chain_id: String,
    #[prost(string, tag = "2")]
    pub election_id: String,
    #[prost(string, tag = "3")]
    pub contest_id: String,
    /// `contest_hash` of the contest the result is about.
    #[prost(bytes = "vec", tag = "4")]
    pub contest_hash: Vec<u8>,
    #[prost(enumeration = "Office", tag = "5")]
    pub office: i32,
    #[prost(message, optional, tag = "6")]
    pub scope: Option<Scope>,
    /// 1 = first round (or a contest without runoff), 2 = second round.
    #[prost(uint32, tag = "7")]
    pub round: u32,
    #[prost(enumeration = "OutcomeKind", tag = "8")]
    pub kind: i32,
    #[prost(uint32, tag = "9")]
    pub winner_option: u32,
    /// The winner's `candidato` credential id (what the `eleito` credential is issued to).
    #[prost(bytes = "vec", tag = "10")]
    pub winner_credential_id: Vec<u8>,
    /// Option indices of the two finalists of a RUNOFF.
    #[prost(uint32, repeated, tag = "11")]
    pub finalists: Vec<u32>,
    #[prost(bytes = "vec", repeated, tag = "12")]
    pub finalist_credentials: Vec<Vec<u8>>,
    /// Votes per option, in ballot order.
    #[prost(uint64, repeated, tag = "13")]
    pub totals: Vec<u64>,
    #[prost(uint64, tag = "14")]
    pub accepted: u64,
    /// Blank and null votes (ADR-0042): reported, but outside the majority's denominator.
    #[prost(uint64, tag = "15")]
    pub blank_votes: u64,
    #[prost(uint64, tag = "16")]
    pub null_votes: u64,
}

/// What an institution publishes on the board: the outcome and its signature. The entry kind is
/// `EntryKind::OUTCOME`; the tally ignores it, the claim of the `eleito` credential reads it.
#[derive(Clone, PartialEq, Message)]
pub struct SignedOutcome {
    #[prost(bytes = "vec", tag = "1")]
    pub outcome: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    pub signer: Vec<u8>,
    #[prost(bytes = "vec", tag = "3")]
    pub signature: Vec<u8>,
}

impl SignedOutcome {
    pub fn new(o: &ElectionOutcome, sig: &SignerSignature) -> Self {
        Self {
            outcome: outcome_bytes(o),
            signer: sig.signer.clone(),
            signature: sig.signature.clone(),
        }
    }
}

/// Decides the contest from its VERIFIED tally. `None` when there is nothing to decide yet (not
/// closed, encrypted and not revealed, inconsistent totals) or the office is not decided
/// automatically (several seats: PENDING).
///
/// * a contest that admits a runoff, in its first round: **more than half** of the votes IN CANDIDATES
///   (blank and null are outside the denominator, ADR-0042)
///   elects; otherwise the two most voted go to a RUNOFF. A tie for the second place (with a
///   third candidate), or a tie between only two candidates, is a TIE;
/// * any other contest (no runoff, or the second round): the most voted wins; a tie at the top is
///   a TIE.
pub fn decide(
    vote_chain_id: &str,
    election_id: &str,
    contest: &Contest,
    contest_hash: &[u8],
    tally: &ContestTally,
) -> Option<ElectionOutcome> {
    if !contest.office().is_single_seat() {
        return None;
    }
    if !matches!(
        tally.phase,
        Phase::VotingClosed | Phase::Tallied | Phase::Audited
    ) || (tally.encrypted && !tally.revealed)
        || tally.options.len() != contest.options.len()
        || tally.options.len() < 2
    {
        return None;
    }
    let totals: Vec<u64> = tally.options.iter().map(|o| o.votes).collect();
    if totals.iter().sum::<u64>() != tally.accepted {
        return None;
    }
    // only CANDIDATES compete and count for the majority; blank and null are reported apart (ADR-0042)
    let mut order: Vec<usize> = (0..totals.len())
        .filter(|i| contest.options[*i].is_candidate())
        .collect();
    if order.len() < 2 {
        return None;
    }
    order.sort_by(|a, b| totals[*b].cmp(&totals[*a]).then(a.cmp(b)));
    let (first, second) = (order[0], order[1]);
    let valid: u64 = order.iter().map(|i| totals[*i]).sum();
    let votes_of = |k: OptionKind| -> u64 {
        contest
            .options
            .iter()
            .zip(&totals)
            .filter(|(o, _)| o.kind() == k)
            .map(|(_, v)| *v)
            .sum()
    };
    let round = if contest.parent.is_empty() { 1 } else { 2 };
    let mut out = ElectionOutcome {
        vote_chain_id: String::from(vote_chain_id),
        election_id: String::from(election_id),
        contest_id: contest.contest_id.clone(),
        contest_hash: contest_hash.to_vec(),
        office: contest.office,
        scope: contest.scope.clone(),
        round,
        kind: OutcomeKind::Tie as i32,
        winner_option: 0,
        winner_credential_id: Vec::new(),
        finalists: Vec::new(),
        finalist_credentials: Vec::new(),
        totals: totals.clone(),
        accepted: tally.accepted,
        blank_votes: votes_of(OptionKind::Blank),
        null_votes: votes_of(OptionKind::Null),
    };
    let elect = |i: usize, out: &mut ElectionOutcome| {
        out.kind = OutcomeKind::Elected as i32;
        out.winner_option = i as u32;
        out.winner_credential_id = contest.options[i].credential_id.clone();
    };
    if valid == 0 {
        return Some(out); // nobody voted for a candidate: nothing to elect
    }
    if contest.runoff && round == 1 {
        if totals[first] * 2 > valid {
            elect(first, &mut out);
        } else if order.len() == 2 {
            // two candidates, no majority: an exact tie
        } else if totals[second] == totals[order[2]] {
            // the second place is tied with a third: the finalist cannot be chosen by the rule
        } else {
            out.kind = OutcomeKind::Runoff as i32;
            out.finalists = alloc::vec![first as u32, second as u32];
            out.finalist_credentials = alloc::vec![
                contest.options[first].credential_id.clone(),
                contest.options[second].credential_id.clone()
            ];
        }
    } else if totals[first] > totals[second] {
        elect(first, &mut out);
    }
    Some(out)
}

/// The canonical bytes the institutions sign.
pub fn outcome_bytes(o: &ElectionOutcome) -> Vec<u8> {
    o.encode_to_vec()
}

/// One institution's signature over the outcome, with its vote-chain consensus key.
pub fn sign_outcome(key: &SigningKey, o: &ElectionOutcome) -> SignerSignature {
    SignerSignature {
        signer: public_key_bytes(key),
        signature: sign_tagged(key, tags::ELECTION_OUTCOME, &outcome_bytes(o)),
    }
}

/// How many DISTINCT institutions of `institutions` signed this exact outcome.
pub fn count_outcome_signers(
    institutions: &[Vec<u8>],
    o: &ElectionOutcome,
    sigs: &[SignerSignature],
) -> usize {
    count_distinct_valid(
        institutions,
        tags::ELECTION_OUTCOME,
        &outcome_bytes(o),
        sigs,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{ElectionOption, OptionKind};
    use crate::tally::{ContestTally, OptionTally, RejectCounts};
    use proptest::prelude::*;

    fn contest(n: usize, runoff: bool, parent: &str) -> Contest {
        Contest {
            contest_id: "gov-sp".into(),
            office: Office::Governor as i32,
            scope: Some(Scope::state("SP")),
            runoff,
            parent: parent.into(),
            options: (0..n)
                .map(|i| ElectionOption {
                    index: i as u32,
                    name: alloc::format!("C{i}"),
                    number: alloc::format!("{}", 10 + i),
                    credential_id: alloc::vec![i as u8 + 1; 32],
                    kind: OptionKind::Candidate as i32,
                })
                .collect(),
            ..Default::default()
        }
    }

    /// `n` candidates followed by BLANK and NULL (ADR-0042).
    fn contest_bn(n: usize, runoff: bool, parent: &str) -> Contest {
        let mut c = contest(n, runoff, parent);
        for (k, name) in [(OptionKind::Blank, "Branco"), (OptionKind::Null, "Nulo")] {
            c.options.push(ElectionOption {
                index: c.options.len() as u32,
                name: name.into(),
                number: String::new(),
                credential_id: Vec::new(),
                kind: k as i32,
            });
        }
        c
    }

    fn tally(votes: &[u64]) -> ContestTally {
        tally_of(&contest(votes.len(), false, ""), votes)
    }

    fn tally_of(c: &Contest, votes: &[u64]) -> ContestTally {
        ContestTally {
            contest_id: "gov-sp".into(),
            title: "t".into(),
            office: Office::Governor,
            scope: Scope::state("SP"),
            phase: Phase::VotingClosed,
            accepted: votes.iter().sum(),
            registered: 0,
            registrations_rejected: 0,
            registration_required: false,
            encrypted: false,
            revealed: false,
            rejected: RejectCounts::default(),
            options: votes
                .iter()
                .enumerate()
                .map(|(i, v)| OptionTally {
                    index: i as u32,
                    name: alloc::format!("C{i}"),
                    number: alloc::format!("{}", 10 + i),
                    votes: *v,
                    kind: c.options[i].kind(),
                })
                .collect(),
        }
    }

    fn run(c: &Contest, votes: &[u64]) -> ElectionOutcome {
        decide(
            "teko-devnet-1",
            "geral-2026",
            c,
            &[9; 32],
            &tally_of(c, votes),
        )
        .unwrap()
    }

    #[test]
    fn blank_and_null_are_outside_the_majority_and_never_win_or_lead() {
        // 3 candidates + blank + null: 60 blank, 10 null, candidates 20/15/5 (40 valid)
        let c = contest_bn(3, true, "");
        let o = run(&c, &[20, 15, 5, 60, 10]);
        // 20 of 40 valid is exactly half: not MORE than half, so a runoff; blanks never finalists
        assert_eq!(o.kind(), OutcomeKind::Runoff);
        assert_eq!(o.finalists, alloc::vec![0, 1]);
        assert_eq!((o.blank_votes, o.null_votes), (60, 10));
        assert_eq!(o.accepted, 110);
        // 21 of 40 valid: elected even though blank+null are 70 of 110 ballots
        let o = run(&c, &[21, 14, 5, 60, 10]);
        assert_eq!(o.kind(), OutcomeKind::Elected);
        assert_eq!(o.winner_option, 0);
        assert_eq!(o.winner_credential_id, alloc::vec![1; 32]);
    }

    #[test]
    fn a_contest_where_nobody_votes_for_a_candidate_has_no_winner() {
        let c = contest_bn(2, true, "");
        let o = run(&c, &[0, 0, 80, 20]);
        assert_eq!(o.kind(), OutcomeKind::Tie);
        assert_eq!((o.blank_votes, o.null_votes), (80, 20));
        // a second round too: the most blank votes does not win
        let c2 = contest_bn(2, false, "gov-sp");
        assert_eq!(run(&c2, &[0, 0, 5, 0]).kind(), OutcomeKind::Tie);
        assert_eq!(run(&c2, &[3, 2, 90, 5]).winner_option, 0);
    }

    #[test]
    fn an_absolute_majority_elects_in_the_first_round() {
        let o = run(&contest(3, true, ""), &[51, 30, 19]);
        assert_eq!(o.kind(), OutcomeKind::Elected);
        assert_eq!(o.winner_option, 0);
        assert_eq!(o.winner_credential_id, alloc::vec![1; 32]);
        assert_eq!(o.round, 1);
        assert_eq!(o.totals, alloc::vec![51, 30, 19]);
    }

    #[test]
    fn without_an_absolute_majority_the_two_most_voted_go_to_a_runoff() {
        let o = run(&contest(3, true, ""), &[40, 35, 25]);
        assert_eq!(o.kind(), OutcomeKind::Runoff);
        assert_eq!(o.finalists, alloc::vec![0, 1]);
        assert_eq!(
            o.finalist_credentials,
            alloc::vec![alloc::vec![1; 32], alloc::vec![2; 32]]
        );
        // exactly 50% is not MORE than half
        let o = run(&contest(3, true, ""), &[50, 30, 20]);
        assert_eq!(o.kind(), OutcomeKind::Runoff);
        // the finalists are chosen by votes, not by ballot order
        let o = run(&contest(4, true, ""), &[10, 30, 40, 20]);
        assert_eq!(o.finalists, alloc::vec![2, 1]);
        // a tie for FIRST place between two is fine: they are the finalists
        let o = run(&contest(3, true, ""), &[40, 40, 20]);
        assert_eq!(o.kind(), OutcomeKind::Runoff);
        assert_eq!(o.finalists, alloc::vec![0, 1]);
    }

    #[test]
    fn ties_the_rule_cannot_break_elect_nobody() {
        // second place tied with a third: who is the finalist?
        assert_eq!(
            run(&contest(3, true, ""), &[40, 30, 30]).kind(),
            OutcomeKind::Tie
        );
        // two candidates, an exact tie
        assert_eq!(
            run(&contest(2, true, ""), &[50, 50]).kind(),
            OutcomeKind::Tie
        );
        // plurality contest or second round: a tie at the top
        assert_eq!(
            run(&contest(3, false, ""), &[40, 40, 20]).kind(),
            OutcomeKind::Tie
        );
        assert_eq!(
            run(&contest(2, false, "gov-sp-1t"), &[7, 7]).kind(),
            OutcomeKind::Tie
        );
        // nobody voted
        assert_eq!(run(&contest(2, true, ""), &[0, 0]).kind(), OutcomeKind::Tie);
    }

    #[test]
    fn a_contest_without_runoff_and_a_second_round_elect_by_plurality() {
        let o = run(&contest(3, false, ""), &[40, 35, 25]);
        assert_eq!(o.kind(), OutcomeKind::Elected);
        assert_eq!(o.winner_option, 0);
        let o = run(&contest(2, false, "gov-sp-1t"), &[41, 59]);
        assert_eq!(o.kind(), OutcomeKind::Elected);
        assert_eq!(o.winner_option, 1);
        assert_eq!(o.round, 2);
    }

    #[test]
    fn nothing_is_decided_before_the_count_is_final_or_for_several_seats() {
        let c = contest(2, true, "");
        let h = [9u8; 32];
        let mut t = tally(&[3, 1]);
        t.phase = Phase::VotingOpen;
        assert!(decide("c", "e", &c, &h, &t).is_none());
        let mut t = tally(&[3, 1]);
        t.encrypted = true; // not revealed
        assert!(decide("c", "e", &c, &h, &t).is_none());
        t.revealed = true;
        assert!(decide("c", "e", &c, &h, &t).is_some());
        let mut t = tally(&[3, 1]);
        t.accepted = 99; // totals that do not add up
        assert!(decide("c", "e", &c, &h, &t).is_none());
        // a multi-seat office is PENDING
        let mut sen = contest(3, false, "");
        sen.office = Office::Senator as i32;
        assert!(decide("c", "e", &sen, &h, &tally(&[3, 2, 1])).is_none());
    }

    #[test]
    fn more_than_two_thirds_of_the_institutions_must_sign_the_same_bytes() {
        use rand::SeedableRng;
        use rand::rngs::StdRng;
        let mut rng = StdRng::seed_from_u64(3);
        let keys: Vec<SigningKey> = (0..4)
            .map(|_| teko_crypto::generate_signing_key(&mut rng))
            .collect();
        let inst: Vec<Vec<u8>> = keys.iter().map(public_key_bytes).collect();
        let o = run(&contest(3, true, ""), &[51, 30, 19]);
        let sigs: Vec<SignerSignature> = keys.iter().map(|k| sign_outcome(k, &o)).collect();
        assert_eq!(count_outcome_signers(&inst, &o, &sigs), 4);
        // the same signature twice counts once; a signature over ANOTHER outcome does not count
        let dup = alloc::vec![sigs[0].clone(), sigs[0].clone()];
        assert_eq!(count_outcome_signers(&inst, &o, &dup), 1);
        let mut other = o.clone();
        other.winner_option = 1;
        assert_eq!(count_outcome_signers(&inst, &other, &sigs), 0);
        // a stranger does not count
        let stranger = teko_crypto::generate_signing_key(&mut rng);
        assert_eq!(
            count_outcome_signers(&inst, &o, &[sign_outcome(&stranger, &o)]),
            0
        );
    }

    proptest! {
        #[test]
        fn there_is_never_more_than_one_winner_and_the_winner_has_the_most_votes(
            votes in proptest::collection::vec(0u64..1000, 2..8), runoff in any::<bool>()
        ) {
            let c = contest(votes.len(), runoff, "");
            let o = run(&c, &votes);
            match o.kind() {
                OutcomeKind::Elected => {
                    let w = votes[o.winner_option as usize];
                    prop_assert!(votes.iter().filter(|v| **v == w).count() == 1);
                    prop_assert!(votes.iter().all(|v| *v <= w));
                    if runoff { prop_assert!(w * 2 > o.accepted); }
                }
                OutcomeKind::Runoff => {
                    prop_assert!(runoff);
                    prop_assert_eq!(o.finalists.len(), 2);
                    let (a, b) = (votes[o.finalists[0] as usize], votes[o.finalists[1] as usize]);
                    prop_assert!(a >= b);
                    // nobody outside the finalists has more than the second finalist
                    for (i, v) in votes.iter().enumerate() {
                        if !o.finalists.contains(&(i as u32)) { prop_assert!(*v < b); }
                    }
                    prop_assert!(a * 2 <= o.accepted);
                }
                _ => {}
            }
            prop_assert_eq!(o.totals.iter().sum::<u64>(), o.accepted);
        }
    }
}
