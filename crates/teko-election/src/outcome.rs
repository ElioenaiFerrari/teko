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

use crate::draw::{DrawGoal, DrawNeed, draw_order};
use crate::eligibility::Scope;
use crate::lifecycle::Phase;
use crate::manifest::{Contest, Office, OptionKind};
use crate::proportional::{self, Allocation};
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
    /// A tie was broken by the verifiable DRAW (ADR-0043): the tied options in the order the seed
    /// drew them, and the seed. Anyone can redo the draw from the board.
    #[prost(bool, tag = "17")]
    pub drawn: bool,
    #[prost(uint32, repeated, tag = "18")]
    pub draw_tied: Vec<u32>,
    #[prost(bytes = "vec", tag = "19")]
    pub draw_seed: Vec<u8>,
    /// A SENATOR contest (ADR-0044): how many seats, and who got them (the options, best first, and
    /// their candidacy credentials). Empty for every other office, whose bytes are unchanged.
    #[prost(uint32, tag = "20")]
    pub seats: u32,
    #[prost(uint32, repeated, tag = "21")]
    pub elected: Vec<u32>,
    #[prost(bytes = "vec", repeated, tag = "22")]
    pub elected_credentials: Vec<Vec<u8>>,
    /// A PROPORTIONAL contest (ADR-0046): the electoral quotient, each party's votes and seats, and the
    /// thresholds that were applied. `seats`/`elected` then hold the whole chamber. Empty/zero for the
    /// other offices, whose bytes are unchanged.
    #[prost(uint64, tag = "23")]
    pub quotient: u64,
    #[prost(message, repeated, tag = "24")]
    pub parties: Vec<PartyResult>,
    #[prost(uint32, tag = "25")]
    pub first_candidate_bp: u32,
    #[prost(uint32, tag = "26")]
    pub party_bp: u32,
    #[prost(uint32, tag = "27")]
    pub candidate_bp: u32,
}

/// One party's result in a proportional contest.
#[derive(Clone, PartialEq, Message)]
pub struct PartyResult {
    /// The party's option index.
    #[prost(uint32, tag = "1")]
    pub option: u32,
    /// Its label votes + the votes of its candidates.
    #[prost(uint64, tag = "2")]
    pub votes: u64,
    #[prost(uint32, tag = "3")]
    pub seats: u32,
    /// The party's number (two digits), so the ledger can check every elected candidacy against it.
    #[prost(string, tag = "4")]
    pub number: String,
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
    decide_with(
        vote_chain_id,
        election_id,
        contest,
        contest_hash,
        tally,
        None,
    )
}

/// What the rule says about a contest, before any draw.
enum Plan {
    Elect(usize),
    Runoff(usize, usize),
    /// A multi-seat contest: these options (best first) take the seats; fewer than the seats when
    /// fewer candidates got votes (the rest stay vacant).
    ElectMany(Vec<usize>),
    /// A tie the rule cannot break: the draw decides.
    Draw {
        need: DrawNeed,
        /// Options that are ahead of the tied group and keep their places (finalists only).
        ahead: Vec<usize>,
    },
    /// A proportional contest (ADR-0046): the allocation of the chamber.
    Proportional(Allocation),
    /// Nobody voted for a candidate.
    Nobody,
}

struct Analysis {
    totals: Vec<u64>,
    valid: u64,
    round: u32,
    plan: Plan,
}

/// The rule, on the verified totals. `None` when there is nothing to decide yet.
fn analyze(contest: &Contest, tally: &ContestTally) -> Option<Analysis> {
    if !contest.office().is_decided() {
        return None;
    }
    if !matches!(
        tally.phase,
        Phase::VotingClosed | Phase::Tallied | Phase::Audited
    ) || (tally.encrypted && !tally.revealed)
        || tally.options.len() != contest.options.len()
        || tally.options.len() < 2
        || tally.seats != contest.seat_count()
    {
        return None;
    }
    let totals: Vec<u64> = tally.options.iter().map(|o| o.votes).collect();
    if !tally.totals_fit_the_ballots() {
        return None;
    }
    if contest.office().is_proportional() {
        // PROPORTIONAL (ADR-0046): the allocation of the chamber; a tie that changes who is elected is
        // drawn once for the whole contest (the drawn order of every candidate and party)
        contest.proportional.as_ref()?;
        let valid = tally.valid_votes();
        let plan = if valid == 0 {
            Plan::Nobody
        } else if proportional::needs_draw(contest, &totals) {
            Plan::Draw {
                need: DrawNeed {
                    tied: contest
                        .options
                        .iter()
                        .enumerate()
                        .filter(|(_, o)| o.is_valid_choice())
                        .map(|(i, _)| i as u32)
                        .collect(),
                    take: contest.proportional.as_ref().map_or(0, |p| p.seats),
                    goal: DrawGoal::Ranking,
                    votes: valid,
                },
                ahead: Vec::new(),
            }
        } else {
            match proportional::allocate(contest, &totals, &|i| i) {
                Some(a) if !a.elected().is_empty() => Plan::Proportional(a),
                _ => Plan::Nobody,
            }
        };
        return Some(Analysis {
            totals,
            valid,
            round: 1,
            plan,
        });
    }
    // only CANDIDATES compete and count for the majority; blank and null are reported apart (ADR-0042)
    let mut order: Vec<usize> = (0..totals.len())
        .filter(|i| contest.options[*i].is_candidate())
        .collect();
    if order.len() < 2 {
        return None;
    }
    order.sort_by(|a, b| totals[*b].cmp(&totals[*a]).then(a.cmp(b)));
    let valid: u64 = order.iter().map(|i| totals[*i]).sum();
    let round = if contest.parent.is_empty() { 1 } else { 2 };
    // the options tied with the one at `rank`, ascending by ballot index
    let tie_group = |rank: usize| -> Vec<usize> {
        let v = totals[order[rank]];
        let mut g: Vec<usize> = order.iter().copied().filter(|i| totals[*i] == v).collect();
        g.sort_unstable();
        g
    };
    let need = |group: &[usize], take: usize, goal: DrawGoal| DrawNeed {
        tied: group.iter().map(|i| *i as u32).collect(),
        take: take as u32,
        goal,
        votes: totals[group[0]],
    };
    let (first, second) = (order[0], order[1]);
    let plan = if contest.office() == Office::Senator {
        // MULTI-SEAT (ADR-0044): only candidates WITH votes compete; the N best win; a tie at the cut
        // is drawn; fewer than N with votes leaves seats vacant (nobody is drawn among zero votes)
        let n = contest.seat_count();
        let comp: Vec<usize> = order.iter().copied().filter(|i| totals[*i] > 0).collect();
        if comp.is_empty() {
            Plan::Nobody
        } else if comp.len() <= n || totals[comp[n - 1]] > totals[comp[n]] {
            Plan::ElectMany(comp.into_iter().take(n).collect())
        } else {
            let cut = totals[comp[n - 1]];
            let mut group: Vec<usize> =
                comp.iter().copied().filter(|i| totals[*i] == cut).collect();
            group.sort_unstable();
            let ahead: Vec<usize> = comp.iter().copied().filter(|i| totals[*i] > cut).collect();
            let take = n - ahead.len();
            Plan::Draw {
                need: need(&group, take, DrawGoal::Seats),
                ahead,
            }
        }
    } else if valid == 0 {
        Plan::Nobody
    } else if contest.runoff && round == 1 {
        if totals[first] * 2 > valid {
            Plan::Elect(first)
        } else if order.len() == 2 {
            // two candidates, no majority: an exact tie, and a runoff between the same two would
            // change nothing, so the draw ELECTS
            Plan::Draw {
                need: need(&tie_group(0), 1, DrawGoal::Winner),
                ahead: Vec::new(),
            }
        } else if totals[second] == totals[order[2]] {
            // the second place is tied with a third: the draw fills the open finalist slots
            let group = tie_group(1);
            let ahead: Vec<usize> = order
                .iter()
                .copied()
                .filter(|i| totals[*i] > totals[second])
                .collect();
            let take = 2 - ahead.len();
            Plan::Draw {
                need: need(&group, take, DrawGoal::Finalists),
                ahead,
            }
        } else {
            Plan::Runoff(first, second)
        }
    } else if totals[first] > totals[second] {
        Plan::Elect(first)
    } else {
        Plan::Draw {
            need: need(&tie_group(0), 1, DrawGoal::Winner),
            ahead: Vec::new(),
        }
    };
    Some(Analysis {
        totals,
        valid,
        round,
        plan,
    })
}

/// The tie the draw has to break, if the rule leaves one (`None` if the contest is not decidable
/// yet or has no such tie). The nodes use it to know that they must publish their share.
pub fn needs_draw(contest: &Contest, tally: &ContestTally) -> Option<DrawNeed> {
    match analyze(contest, tally)?.plan {
        Plan::Draw { need, .. } => Some(need),
        _ => None,
    }
}

/// Like `decide`, with the SEED of the verifiable draw when there is one. A tie the rule cannot
/// break stays a `Tie` without a seed; with the seed it is resolved (ADR-0043). With no tie the
/// seed is ignored, so every institution reaches the same bytes.
pub fn decide_with(
    vote_chain_id: &str,
    election_id: &str,
    contest: &Contest,
    contest_hash: &[u8],
    tally: &ContestTally,
    draw_seed: Option<&[u8; 32]>,
) -> Option<ElectionOutcome> {
    let a = analyze(contest, tally)?;
    let votes_of = |k: OptionKind| -> u64 {
        contest
            .options
            .iter()
            .zip(&a.totals)
            .filter(|(o, _)| o.kind() == k)
            .map(|(_, v)| *v)
            .sum()
    };
    let mut out = ElectionOutcome {
        vote_chain_id: String::from(vote_chain_id),
        election_id: String::from(election_id),
        contest_id: contest.contest_id.clone(),
        contest_hash: contest_hash.to_vec(),
        office: contest.office,
        scope: contest.scope.clone(),
        round: a.round,
        kind: OutcomeKind::Tie as i32,
        winner_option: 0,
        winner_credential_id: Vec::new(),
        finalists: Vec::new(),
        finalist_credentials: Vec::new(),
        totals: a.totals.clone(),
        accepted: tally.accepted,
        blank_votes: votes_of(OptionKind::Blank),
        null_votes: votes_of(OptionKind::Null),
        drawn: false,
        draw_tied: Vec::new(),
        draw_seed: Vec::new(),
        seats: if contest.office() == Office::Senator {
            contest.seat_count() as u32
        } else {
            contest.proportional.as_ref().map_or(0, |p| p.seats)
        },
        elected: Vec::new(),
        elected_credentials: Vec::new(),
        quotient: 0,
        parties: Vec::new(),
        first_candidate_bp: 0,
        party_bp: 0,
        candidate_bp: 0,
    };
    let chamber = |alloc: &Allocation, out: &mut ElectionOutcome| {
        let list = alloc.elected();
        out.kind = OutcomeKind::Elected as i32;
        out.elected = list.iter().map(|i| *i as u32).collect();
        out.elected_credentials = list
            .iter()
            .map(|i| contest.options[*i].credential_id.clone())
            .collect();
        out.quotient = alloc.quotient;
        out.parties = alloc
            .parties
            .iter()
            .map(|p| PartyResult {
                option: p.option as u32,
                votes: p.votes,
                seats: p.seats,
                number: contest.options[p.option].number.clone(),
            })
            .collect();
        if let Some(r) = &contest.proportional {
            out.first_candidate_bp = r.first_candidate_bp;
            out.party_bp = r.party_bp;
            out.candidate_bp = r.candidate_bp;
        }
    };
    let elect_many = |list: &[usize], out: &mut ElectionOutcome| {
        out.kind = OutcomeKind::Elected as i32;
        out.elected = list.iter().map(|i| *i as u32).collect();
        out.elected_credentials = list
            .iter()
            .map(|i| contest.options[*i].credential_id.clone())
            .collect();
    };
    let elect = |i: usize, out: &mut ElectionOutcome| {
        out.kind = OutcomeKind::Elected as i32;
        out.winner_option = i as u32;
        out.winner_credential_id = contest.options[i].credential_id.clone();
    };
    let runoff = |f: [usize; 2], out: &mut ElectionOutcome| {
        out.kind = OutcomeKind::Runoff as i32;
        out.finalists = alloc::vec![f[0] as u32, f[1] as u32];
        out.finalist_credentials = alloc::vec![
            contest.options[f[0]].credential_id.clone(),
            contest.options[f[1]].credential_id.clone()
        ];
    };
    let _ = a.valid;
    match a.plan {
        Plan::Nobody => {}
        Plan::Elect(i) => elect(i, &mut out),
        Plan::ElectMany(list) => elect_many(&list, &mut out),
        Plan::Proportional(alloc) => chamber(&alloc, &mut out),
        Plan::Runoff(x, y) => runoff([x, y], &mut out),
        Plan::Draw { need, ahead } => {
            if let Some(seed) = draw_seed {
                let order = draw_order(seed, &need.tied);
                out.drawn = true;
                out.draw_seed = seed.to_vec();
                out.draw_tied = order.clone();
                match need.goal {
                    DrawGoal::Ranking => {
                        // the drawn order is the tie-break of the whole allocation (lower wins)
                        let n = contest.options.len();
                        let mut place = alloc::vec![n; n];
                        for (rank, opt) in order.iter().enumerate() {
                            if let Some(p) = place.get_mut(*opt as usize) {
                                *p = rank;
                            }
                        }
                        let pos = |i: usize| place.get(i).copied().unwrap_or(n) * (n + 1) + i;
                        if let Some(alloc) = proportional::allocate(contest, &a.totals, &pos)
                            && !alloc.elected().is_empty()
                        {
                            chamber(&alloc, &mut out);
                        }
                    }
                    DrawGoal::Winner => elect(order[0] as usize, &mut out),
                    DrawGoal::Seats => {
                        let mut list = ahead;
                        list.extend(order.iter().take(need.take as usize).map(|i| *i as usize));
                        elect_many(&list, &mut out);
                    }
                    DrawGoal::Finalists => {
                        let mut f = ahead;
                        f.extend(order.iter().take(need.take as usize).map(|i| *i as usize));
                        if let [x, y] = f[..] {
                            runoff([x, y], &mut out);
                        }
                    }
                }
            }
        }
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
    use crate::draw::{DrawGoal, DrawNeed};
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
                    ..Default::default()
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
                ..Default::default()
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
                    party: String::new(),
                })
                .collect(),
            seats: c.seat_count(),
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

    fn drawn(c: &Contest, votes: &[u64], seed: u8) -> ElectionOutcome {
        decide_with(
            "teko-devnet-1",
            "geral-2026",
            c,
            &[9; 32],
            &tally_of(c, votes),
            Some(&[seed; 32]),
        )
        .unwrap()
    }

    fn need(c: &Contest, votes: &[u64]) -> Option<DrawNeed> {
        needs_draw(c, &tally_of(c, votes))
    }

    /// A senator contest: `n` candidates, blank and null, with the given seats (ADR-0044).
    fn senator(n: usize, seats: u32) -> Contest {
        let mut c = contest_bn(n, false, "");
        c.office = Office::Senator as i32;
        c.seats = seats;
        c
    }

    #[test]
    fn a_senate_contest_elects_the_n_most_voted_candidates_with_blank_and_null_outside() {
        // 2 seats, 4 candidates + blank + null
        let c = senator(4, 2);
        let o = run(&c, &[30, 25, 20, 10, 6, 3]);
        assert_eq!(o.kind(), OutcomeKind::Elected);
        assert_eq!((o.seats, o.elected.clone()), (2, alloc::vec![0, 1]));
        assert_eq!(
            o.elected_credentials,
            alloc::vec![alloc::vec![1; 32], alloc::vec![2; 32]]
        );
        assert_eq!((o.blank_votes, o.null_votes), (6, 3));
        assert!(!o.drawn && need(&c, &[30, 25, 20, 10, 6, 3]).is_none());
        // best first even when the ballot order differs
        let o = run(&c, &[10, 20, 30, 25, 0, 0]);
        assert_eq!(o.elected, alloc::vec![2, 3]);
        // one seat: the most voted
        let c1 = senator(3, 1);
        let o = run(&c1, &[10, 30, 20, 5, 0]);
        assert_eq!((o.seats, o.elected.clone()), (1, alloc::vec![1]));
        // the other offices keep their bytes: no seats, no list
        let g = run(&contest(3, false, ""), &[10, 30, 20]);
        assert_eq!((g.seats, g.elected.len()), (0, 0));
    }

    #[test]
    fn a_tie_at_the_cut_of_the_seats_is_drawn_and_zero_votes_never_take_a_seat() {
        let c = senator(4, 2);
        // [30,20,20,5]: the leader is in, the draw fills the other seat among the two tied
        let n = need(&c, &[30, 20, 20, 5, 0, 0]).unwrap();
        assert_eq!(
            (n.tied.clone(), n.take, n.goal),
            (alloc::vec![1, 2], 1, DrawGoal::Seats)
        );
        assert_eq!(
            run(&c, &[30, 20, 20, 5, 0, 0]).kind(),
            OutcomeKind::Tie,
            "pending the draw"
        );
        let mut second = alloc::collections::BTreeSet::new();
        for seed in 0..60u8 {
            let o = drawn(&c, &[30, 20, 20, 5, 0, 0], seed);
            assert_eq!(o.kind(), OutcomeKind::Elected);
            assert!(o.drawn && o.elected[0] == 0);
            second.insert(o.elected[1]);
            assert_eq!(o.draw_tied.len(), 2);
        }
        assert_eq!(second, alloc::collections::BTreeSet::from([1, 2]));
        // three tied for two seats: two different ones
        let c3 = senator(3, 2);
        for seed in 0..40u8 {
            let o = drawn(&c3, &[20, 20, 20, 0, 0], seed);
            assert_ne!(o.elected[0], o.elected[1]);
            assert!(o.elected.iter().all(|e| *e < 3));
        }
        // fewer candidates with votes than seats: the rest stay VACANT, nobody is drawn among zeros
        let o = run(&c, &[30, 0, 0, 0, 5, 5]);
        assert_eq!(o.elected, alloc::vec![0]);
        assert!(need(&c, &[30, 0, 0, 0, 5, 5]).is_none());
        // nobody voted for a candidate: nobody is elected
        assert_eq!(run(&c, &[0, 0, 0, 0, 9, 9]).kind(), OutcomeKind::Tie);
        // one seat: a tie at the top is drawn too
        let c1 = senator(3, 1);
        let n = need(&c1, &[20, 20, 5, 0, 0]).unwrap();
        assert_eq!((n.take, n.goal), (1, DrawGoal::Seats));
    }

    #[test]
    fn the_totals_of_a_multi_seat_contest_must_fit_the_ballots_that_were_counted() {
        let c = senator(3, 2);
        // 10 ballots; candidates got 12 votes, blank 1 (weighs 2), null 0: weighted 14 is in 10..=20
        let mut t = tally_of(&c, &[6, 4, 2, 1, 0]);
        t.accepted = 10;
        assert!(decide("v", "e", &c, &[9; 32], &t).is_some());
        // 10 ballots but 25 candidate votes: impossible with 2 seats (at most 20)
        let mut t = tally_of(&c, &[10, 10, 5, 0, 0]);
        t.accepted = 10;
        assert!(decide("v", "e", &c, &[9; 32], &t).is_none());
        // fewer weighted votes than ballots: every ballot has at least one vote
        let mut t = tally_of(&c, &[3, 2, 0, 0, 0]);
        t.accepted = 10;
        assert!(decide("v", "e", &c, &[9; 32], &t).is_none());
        // a one-seat contest still has to add up EXACTLY
        let c1 = contest_bn(3, false, "");
        let mut t = tally_of(&c1, &[6, 3, 2, 0, 0]);
        t.accepted = 10;
        assert!(decide("v", "e", &c1, &[9; 32], &t).is_none());
        // the tally of another seat count is not this contest's
        let mut t = tally_of(&c, &[6, 4, 2, 1, 0]);
        t.accepted = 10;
        t.seats = 1;
        assert!(decide("v", "e", &c, &[9; 32], &t).is_none());
    }

    #[test]
    fn a_tie_at_the_top_is_decided_by_the_draw_and_only_among_the_tied() {
        // two candidates, no majority: the draw ELECTS (a runoff of the same two would change nothing)
        let c = contest(2, true, "");
        let n = need(&c, &[50, 50]).unwrap();
        assert_eq!(
            (n.tied.clone(), n.take, n.goal),
            (alloc::vec![0, 1], 1, DrawGoal::Winner)
        );
        assert_eq!(
            run(&c, &[50, 50]).kind(),
            OutcomeKind::Tie,
            "no seed yet: still pending"
        );
        let mut winners = alloc::collections::BTreeSet::new();
        for seed in 0..60u8 {
            let o = drawn(&c, &[50, 50], seed);
            assert_eq!(o.kind(), OutcomeKind::Elected);
            assert!(o.drawn && o.draw_seed == alloc::vec![seed; 32]);
            assert_eq!(o.draw_tied.len(), 2);
            assert_eq!(o.winner_option, o.draw_tied[0], "the first drawn wins");
            winners.insert(o.winner_option);
            assert_eq!(drawn(&c, &[50, 50], seed), o, "same seed, same bytes");
        }
        assert_eq!(winners.len(), 2, "either can win");
        // plurality contest and second round: a tie at the top, among the TIED only (3rd is not in it)
        let c = contest(3, false, "");
        for seed in 0..30u8 {
            let o = drawn(&c, &[40, 40, 20], seed);
            assert_eq!(o.kind(), OutcomeKind::Elected);
            assert!(o.winner_option < 2);
        }
        assert_eq!(
            drawn(&contest(2, false, "gov-sp-1t"), &[7, 7], 3).kind(),
            OutcomeKind::Elected
        );
        // blank and null never enter the draw, even when they have as many votes
        let c = contest_bn(2, true, "");
        let n = need(&c, &[30, 30, 30, 30]).unwrap();
        assert_eq!(n.tied, alloc::vec![0, 1]);
        for seed in 0..20u8 {
            assert!(drawn(&c, &[30, 30, 30, 30], seed).winner_option < 2);
        }
    }

    #[test]
    fn a_tie_for_the_finalist_slot_is_drawn_and_what_the_rule_decides_is_untouched() {
        let c = contest(3, true, "");
        // [40,30,30]: 0 is a finalist for sure, the draw fills the other slot among 1 and 2
        let n = need(&c, &[40, 30, 30]).unwrap();
        assert_eq!(
            (n.tied.clone(), n.take, n.goal),
            (alloc::vec![1, 2], 1, DrawGoal::Finalists)
        );
        let mut second = alloc::collections::BTreeSet::new();
        for seed in 0..60u8 {
            let o = drawn(&c, &[40, 30, 30], seed);
            assert_eq!(o.kind(), OutcomeKind::Runoff);
            assert_eq!(o.finalists[0], 0);
            second.insert(o.finalists[1]);
        }
        assert_eq!(second, alloc::collections::BTreeSet::from([1, 2]));
        // [40,40,40]: two of the three, never the same twice
        let c = contest(3, true, "");
        for seed in 0..40u8 {
            let o = drawn(&c, &[40, 40, 40], seed);
            assert_eq!(o.kind(), OutcomeKind::Runoff);
            assert_ne!(o.finalists[0], o.finalists[1]);
        }
        let c4 = contest(4, true, "");
        let n = need(&c4, &[30, 30, 30, 10]).unwrap();
        assert_eq!((n.tied.len(), n.take), (3, 2));
        // a tie that does not decide anything needs no draw: the seed is ignored
        assert!(need(&c, &[40, 40, 20]).is_none());
        assert!(need(&c, &[51, 25, 24]).is_none());
        assert!(need(&c, &[40, 35, 25]).is_none());
        for votes in [[40, 40, 20], [51, 25, 24], [40, 35, 25]] {
            assert_eq!(drawn(&c, &votes, 7), run(&c, &votes));
        }
        // nobody voted for a candidate: there is nothing to draw
        let c = contest(2, true, "");
        assert!(need(&c, &[0, 0]).is_none());
        assert_eq!(drawn(&c, &[0, 0], 1).kind(), OutcomeKind::Tie);
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
        // a PROPORTIONAL office without its chamber (ADR-0046) is not decided
        for office in [
            Office::FederalDeputy,
            Office::StateDeputy,
            Office::CityCouncillor,
        ] {
            let mut prop = contest(3, false, "");
            prop.office = office as i32;
            assert!(
                decide("c", "e", &prop, &h, &tally(&[3, 2, 1])).is_none(),
                "{office:?}"
            );
        }
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
        fn a_senate_contest_never_elects_more_than_its_seats_nor_a_candidate_without_votes(
            votes in proptest::collection::vec(0u64..30, 3..7), seats in 1u32..=2, seed in any::<u8>()
        ) {
            let c = senator(votes.len(), seats);
            let mut v = votes.clone();
            v.extend([0, 0]);
            let o = drawn(&c, &v, seed);
            prop_assert!(o.elected.len() <= seats as usize);
            prop_assert!(o.elected.iter().all(|e| votes[*e as usize] > 0));
            let mut sorted = o.elected.clone();
            sorted.sort_unstable();
            sorted.dedup();
            prop_assert_eq!(sorted.len(), o.elected.len(), "no one twice");
            // everyone not elected has no more votes than the LEAST voted of the elected
            if let Some(least) = o.elected.iter().map(|e| votes[*e as usize]).min() {
                for (i, x) in votes.iter().enumerate() {
                    if !o.elected.contains(&(i as u32)) { prop_assert!(*x <= least); }
                }
            }
            if votes.iter().all(|x| *x == 0) { prop_assert_eq!(o.kind(), OutcomeKind::Tie); }
        }

        #[test]
        fn with_a_seed_every_decidable_contest_is_decided_among_the_right_candidates(
            votes in proptest::collection::vec(0u64..40, 2..8), runoff in any::<bool>(), seed in any::<u8>()
        ) {
            let c = contest(votes.len(), runoff, "");
            let o = drawn(&c, &votes, seed);
            let valid: u64 = votes.iter().sum();
            if valid == 0 {
                prop_assert_eq!(o.kind(), OutcomeKind::Tie);
            } else {
                prop_assert_ne!(o.kind(), OutcomeKind::Tie);
                let max = *votes.iter().max().unwrap();
                match o.kind() {
                    OutcomeKind::Elected => {
                        // the winner has the most votes (a drawn one is among those tied for it)
                        prop_assert_eq!(votes[o.winner_option as usize], max);
                        if runoff && !o.drawn { prop_assert!(max * 2 > valid); }
                    }
                    OutcomeKind::Runoff => {
                        prop_assert_eq!(o.finalists.len(), 2);
                        prop_assert_ne!(o.finalists[0], o.finalists[1]);
                        let b = votes[o.finalists[1] as usize];
                        for (i, v) in votes.iter().enumerate() {
                            if !o.finalists.contains(&(i as u32)) { prop_assert!(*v <= b); }
                        }
                    }
                    _ => prop_assert!(false),
                }
            }
        }

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

    #[test]
    fn a_proportional_contest_is_decided_with_the_whole_chamber_and_its_parties() {
        let c = crate::proportional::tests::chamber(2, 2, 4, (0, 0, 0));
        let o = run(&c, &[100, 300, 200, 0, 250, 150, 5, 5]);
        assert_eq!(o.kind(), OutcomeKind::Elected);
        assert_eq!((o.seats, o.quotient), (4, 250));
        assert_eq!(o.elected, alloc::vec![1, 2, 4, 5]);
        assert_eq!(o.elected_credentials.len(), 4);
        let shares: Vec<(u64, u32)> = o.parties.iter().map(|p| (p.votes, p.seats)).collect();
        assert_eq!(shares, alloc::vec![(600, 2), (400, 2)]);
        assert_eq!((o.blank_votes, o.null_votes), (5, 5));
        assert!(!o.drawn);
        // the majoritarian outcomes keep their bytes: none of the new fields is set
        let g = run(&contest(2, false, ""), &[3, 1]);
        assert_eq!((g.quotient, g.parties.len(), g.party_bp), (0, 0, 0));
    }

    #[test]
    fn a_proportional_tie_that_matters_is_drawn_once_for_the_whole_contest() {
        let c = crate::proportional::tests::chamber(2, 2, 1, (0, 0, 0));
        let votes = [0u64, 5, 5, 0, 1, 1, 0, 0];
        let t = tally_of(&c, &votes);
        let need = needs_draw(&c, &t).unwrap();
        assert_eq!(
            (need.goal, need.take, need.votes),
            (DrawGoal::Ranking, 1, 12)
        );
        // every candidate and party takes part (not blank or null)
        assert_eq!(need.tied, alloc::vec![0, 1, 2, 3, 4, 5]);
        let h = [9u8; 32];
        assert_eq!(
            decide_with("v", "e", &c, &h, &t, None).unwrap().kind(),
            OutcomeKind::Tie
        );
        let mut seen = alloc::collections::BTreeSet::new();
        for s in 0..40u8 {
            let o = decide_with("v", "e", &c, &h, &t, Some(&[s; 32])).unwrap();
            assert_eq!(o.kind(), OutcomeKind::Elected);
            assert!(o.drawn && !o.draw_seed.is_empty());
            assert_eq!(o.elected.len(), 1);
            seen.insert(o.elected[0]);
            // the same seed reaches the same bytes
            let again = decide_with("v", "e", &c, &h, &t, Some(&[s; 32])).unwrap();
            assert_eq!(outcome_bytes(&o), outcome_bytes(&again));
        }
        assert_eq!(seen, alloc::collections::BTreeSet::from([1usize as u32, 2]));
        // no tie that matters: no draw, and a seed is ignored
        let clear = tally_of(&c, &[0, 9, 1, 0, 5, 5, 0, 0]);
        assert!(needs_draw(&c, &clear).is_none());
    }
}
