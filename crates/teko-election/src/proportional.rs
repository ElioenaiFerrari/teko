//! The PROPORTIONAL rule (ADR-0046): how the seats of a chamber are shared among parties and who
//! takes them. A pure function of the verified totals, so every institution reaches the same bytes.
//!
//! Open list, ONE vote per ballot: in a candidate (counts for the candidate and for its party) or in
//! a party label (only for the party). Blank and null are outside the valid votes.
//!
//! 1. quotient = valid votes / seats, a fraction above 0.5 rounds up (otherwise it is dropped);
//! 2. each party takes `party votes / quotient` seats (whole), filled by its most voted candidates
//!    that have at least `first_candidate_bp` of the quotient; a seat nobody apt can fill goes to the
//!    leftovers;
//! 3. leftovers by D'HONDT (highest average `votes / (seats held + 1)`) among the parties with at
//!    least `party_bp` of the quotient, each with a candidate that has at least `candidate_bp` of it;
//! 4. what is still open goes by D'Hondt among ALL parties with an available candidate (no
//!    thresholds);
//! 5. if no party reaches the quotient, the most voted candidates take the seats.
//!
//! Only candidates WITH votes take a seat (nobody is elected with zero votes): the chamber may stay
//! incomplete. Ties are broken by a `pos` function (lower wins): the ballot order, the reverse, or
//! the order of the verifiable draw (ADR-0043); `needs_draw` says whether the choice matters.

use alloc::vec::Vec;

use crate::manifest::{Contest, OptionKind, Proportional};

/// One party's share of the chamber.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartySeats {
    /// The party's option index in the contest.
    pub option: usize,
    /// Its votes: label votes + the votes of its candidates.
    pub votes: u64,
    /// The seats it holds.
    pub seats: u32,
    /// The candidates (option indices) that took them, in the order they were seated.
    pub elected: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Allocation {
    pub quotient: u64,
    pub valid: u64,
    /// Every party, ascending by option index.
    pub parties: Vec<PartySeats>,
}

impl Allocation {
    /// All the elected candidates: by party (ascending option index), then in the order seated.
    pub fn elected(&self) -> Vec<usize> {
        self.parties
            .iter()
            .flat_map(|p| p.elected.iter().copied())
            .collect()
    }
}

/// `valid / seats`, a fraction above one half rounds up. `None` when there is no seat.
pub fn quotient(valid: u64, seats: u64) -> Option<u64> {
    if seats == 0 {
        return None;
    }
    let (q, r) = (valid / seats, valid % seats);
    // r/seats > 1/2  <=>  2r > seats
    Some(if r.saturating_mul(2) > seats {
        q + 1
    } else {
        q
    })
}

/// `votes >= bp/10000 * quotient`, in integers.
fn reaches(votes: u64, bp: u32, quotient: u64) -> bool {
    (votes as u128) * 10_000 >= (bp as u128) * (quotient as u128)
}

/// Allocates the chamber from the per-option `totals`. `pos(option)`: the tie-break (lower wins).
/// `None` when there is nothing to allocate (no valid votes, or the contest is not proportional).
pub fn allocate(
    contest: &Contest,
    totals: &[u64],
    pos: &dyn Fn(usize) -> usize,
) -> Option<Allocation> {
    let rule: &Proportional = contest.proportional.as_ref()?;
    if totals.len() != contest.options.len() || rule.seats == 0 {
        return None;
    }
    let seats = rule.seats as usize;
    let valid: u64 = contest
        .options
        .iter()
        .zip(totals)
        .filter(|(o, _)| o.is_valid_choice())
        .map(|(_, v)| *v)
        .sum();
    if valid == 0 {
        return None;
    }
    let q = quotient(valid, seats as u64)?.max(1);
    // the parties and their candidates (ballot order)
    struct Party {
        option: usize,
        votes: u64,
        // candidates, best first: more votes, then the tie-break
        cands: Vec<usize>,
        held: Vec<usize>,
        won: u64,
    }
    let mut parties: Vec<Party> = Vec::new();
    for (i, o) in contest.options.iter().enumerate() {
        if o.kind() != OptionKind::Party {
            continue;
        }
        let mut cands: Vec<usize> = contest
            .options
            .iter()
            .enumerate()
            .filter(|(_, c)| c.is_candidate() && c.party == o.number)
            .map(|(k, _)| k)
            .collect();
        cands.sort_by(|a, b| totals[*b].cmp(&totals[*a]).then(pos(*a).cmp(&pos(*b))));
        let votes = totals[i] + cands.iter().map(|k| totals[*k]).sum::<u64>();
        parties.push(Party {
            option: i,
            votes,
            cands,
            held: Vec::new(),
            won: votes / q,
        });
    }
    let total_won: u64 = parties.iter().map(|p| p.won).sum();
    let mut left = seats;
    if parties.iter().all(|p| p.won == 0) {
        // nobody reaches the quotient: the most voted candidates take the seats, whatever the party
        let mut all: Vec<(usize, usize)> = parties
            .iter()
            .enumerate()
            .flat_map(|(pi, p)| p.cands.iter().map(move |k| (pi, *k)))
            .filter(|(_, k)| totals[*k] > 0)
            .collect();
        all.sort_by(|a, b| totals[b.1].cmp(&totals[a.1]).then(pos(a.1).cmp(&pos(b.1))));
        for (pi, k) in all.into_iter().take(seats) {
            parties[pi].held.push(k);
        }
    } else {
        // 2. the quotient seats, filled by candidates with enough votes
        if total_won <= seats as u64 {
            for p in &mut parties {
                let apt: Vec<usize> = p
                    .cands
                    .iter()
                    .copied()
                    .filter(|k| totals[*k] > 0 && reaches(totals[*k], rule.first_candidate_bp, q))
                    .take(p.won as usize)
                    .collect();
                left -= apt.len();
                p.held = apt;
            }
        }
        // 3 and 4: the leftovers, first with the thresholds, then without
        for strict in [true, false] {
            while left > 0 {
                let mut best: Option<usize> = None;
                for (i, p) in parties.iter().enumerate() {
                    let next = p
                        .cands
                        .iter()
                        .copied()
                        .filter(|k| !p.held.contains(k) && totals[*k] > 0)
                        .find(|k| !strict || reaches(totals[*k], rule.candidate_bp, q));
                    if next.is_none() || (strict && !reaches(p.votes, rule.party_bp, q)) {
                        continue;
                    }
                    best = match best {
                        None => Some(i),
                        Some(b) => {
                            let (x, y) = (&parties[b], p);
                            // x.votes / (hx+1)  <  y.votes / (hy+1)
                            let lhs = (x.votes as u128) * (y.held.len() as u128 + 1);
                            let rhs = (y.votes as u128) * (x.held.len() as u128 + 1);
                            let y_wins = rhs > lhs || (rhs == lhs && pos(y.option) < pos(x.option));
                            Some(if y_wins { i } else { b })
                        }
                    };
                }
                let Some(i) = best else { break };
                let p = &mut parties[i];
                let held: Vec<usize> = p.held.clone();
                if let Some(k) = p
                    .cands
                    .iter()
                    .copied()
                    .filter(|k| !held.contains(k) && totals[*k] > 0)
                    .find(|k| !strict || reaches(totals[*k], rule.candidate_bp, q))
                {
                    p.held.push(k);
                    left -= 1;
                } else {
                    break;
                }
            }
        }
    }
    Some(Allocation {
        quotient: q,
        valid,
        parties: parties
            .into_iter()
            .map(|p| PartySeats {
                option: p.option,
                votes: p.votes,
                seats: p.held.len() as u32,
                elected: p.held,
            })
            .collect(),
    })
}

/// Does the tie-break change who is elected? Compares the ballot order with its reverse: if both give
/// the same seats per party and the same elected set, no tie mattered and no draw is needed.
pub fn needs_draw(contest: &Contest, totals: &[u64]) -> bool {
    let n = contest.options.len();
    let (Some(a), Some(b)) = (
        allocate(contest, totals, &|i| i),
        allocate(contest, totals, &|i| n - i),
    ) else {
        return false;
    };
    let key = |x: &Allocation| -> Vec<(usize, Vec<usize>)> {
        x.parties
            .iter()
            .map(|p| {
                let mut e = p.elected.clone();
                e.sort_unstable();
                (p.option, e)
            })
            .collect()
    };
    key(&a) != key(&b)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
pub(crate) mod tests {
    use super::*;
    use crate::eligibility::Scope;
    use crate::manifest::{ElectionOption, Office};
    use alloc::format;
    use alloc::string::String;

    /// Two parties ("13", "45") with `a` and `b` candidates, then blank and null.
    pub(crate) fn chamber(a: usize, b: usize, seats: u32, bp: (u32, u32, u32)) -> Contest {
        let mut options = Vec::new();
        for (num, name, n) in [("13", "Verde", a), ("45", "Azul", b)] {
            options.push(ElectionOption {
                name: name.into(),
                number: num.into(),
                kind: OptionKind::Party as i32,
                ..Default::default()
            });
            for k in 0..n {
                options.push(ElectionOption {
                    name: format!("{name} {k}"),
                    number: format!("{num}{:02}", k + 1),
                    credential_id: alloc::vec![options.len() as u8; 32],
                    party: num.into(),
                    ..Default::default()
                });
            }
        }
        for (k, name) in [(OptionKind::Blank, "Branco"), (OptionKind::Null, "Nulo")] {
            options.push(ElectionOption {
                name: name.into(),
                kind: k as i32,
                ..Default::default()
            });
        }
        for (i, o) in options.iter_mut().enumerate() {
            o.index = i as u32;
        }
        Contest {
            contest_id: "dep".into(),
            office: Office::StateDeputy as i32,
            scope: Some(Scope::state("SP")),
            options,
            proportional: Some(Proportional {
                seats,
                first_candidate_bp: bp.0,
                party_bp: bp.1,
                candidate_bp: bp.2,
            }),
            ..Default::default()
        }
    }

    /// Votes: [label13, c13..., label45, c45..., blank, null]
    fn run(c: &Contest, votes: &[u64]) -> Allocation {
        allocate(c, votes, &|i| i).unwrap()
    }

    fn seats_of(a: &Allocation) -> Vec<u32> {
        a.parties.iter().map(|p| p.seats).collect()
    }

    #[test]
    fn the_quotient_rounds_a_fraction_above_one_half_up_and_drops_the_rest() {
        assert_eq!(quotient(100, 3), Some(33)); // 33.33
        assert_eq!(quotient(101, 3), Some(34)); // 33.67
        assert_eq!(quotient(7, 2), Some(3)); // 3.5 is not above one half
        assert_eq!(quotient(9, 4), Some(2)); // 2.25
        assert_eq!(quotient(11, 4), Some(3)); // 2.75
        assert_eq!(quotient(5, 0), None);
    }

    #[test]
    fn the_quotient_seats_the_leftovers_by_dhondt_and_the_most_voted_candidates_take_them() {
        // 4 seats; verde 600 (label 100, c: 300, 200), azul 400 (c: 250, 150)
        let c = chamber(2, 2, 4, (0, 0, 0));
        // [L13, 1301, 1302, L45, 4501, 4502, blank, null]
        let a = run(&c, &[100, 300, 200, 0, 250, 150, 5, 5]);
        // valid 1000, quotient 250: verde 600 -> 2 seats, azul 400 -> 1; the 4th seat is a leftover:
        // verde has no candidate left, so azul's second candidate takes it
        assert_eq!(a.quotient, 250);
        assert_eq!(seats_of(&a), vec![2, 2]);
        assert_eq!(a.parties[0].elected, vec![1, 2]);
        assert_eq!(a.parties[1].elected, vec![4, 5]);
    }

    #[test]
    fn a_label_vote_counts_for_the_party_but_elects_nobody_alone() {
        let c = chamber(2, 2, 2, (0, 0, 0));
        // verde: label 400 + candidates 50, 50; azul: 300 + 100, 0
        let a = run(&c, &[400, 50, 50, 300, 100, 0, 0, 0]);
        // valid 900, quotient 450: verde 500 -> 1, azul 400 -> 0; leftover D'Hondt: verde 500/2=250 vs azul 400/1
        assert_eq!(a.parties[0].votes, 500);
        assert_eq!(seats_of(&a), vec![1, 1]);
        assert_eq!(a.parties[0].elected, vec![1]);
        assert_eq!(a.parties[1].elected, vec![4]);
    }

    #[test]
    fn blank_and_null_are_outside_the_valid_votes() {
        let c = chamber(1, 1, 2, (0, 0, 0));
        let with = run(&c, &[0, 60, 0, 40, 500, 500]);
        let without = run(&c, &[0, 60, 0, 40, 0, 0]);
        assert_eq!(
            (with.valid, with.quotient),
            (without.valid, without.quotient)
        );
        assert_eq!(seats_of(&with), seats_of(&without));
    }

    #[test]
    fn a_candidate_below_the_threshold_leaves_the_seat_to_the_leftovers() {
        // verde wins 1 seat by the quotient but its best candidate has only 5% of it (< 10%)
        let c = chamber(2, 1, 2, (1000, 0, 0));
        // verde: label 480, c 20, 0; azul: c 500. valid 1000, quotient 500
        let a = run(&c, &[480, 20, 0, 0, 500, 0, 0]);
        assert_eq!(a.quotient, 500);
        // verde 500 -> 1 seat, but 20 < 10% of 500 = 50: not apt; azul 500 -> 1
        assert_eq!(a.parties[0].elected.len() + a.parties[1].elected.len(), 2);
        assert_eq!(a.parties[1].elected, vec![4]);
        // the leftover seat goes to verde in the next rounds (the candidate is the only one with votes)
        assert_eq!(a.parties[0].elected, vec![1]);
    }

    #[test]
    fn the_leftovers_need_the_party_and_candidate_thresholds_and_then_the_third_round_fills_up() {
        // 3 seats; quotient 100 (valid 300). verde 300 (c: 150,150,0 -> 3 seats?), azul 0
        let c = chamber(3, 1, 3, (0, 8000, 2000));
        let a = run(&c, &[0, 150, 150, 0, 0, 0, 0, 0]);
        assert_eq!(a.quotient, 100);
        assert_eq!(a.parties[0].elected, vec![1, 2]);
        // the third seat: nobody else has votes -> stays vacant (nobody is elected with zero votes)
        assert_eq!(a.parties[0].seats + a.parties[1].seats, 2);
        // a small party under the 80% threshold only gets a leftover in the third round
        let c = chamber(1, 1, 2, (0, 8000, 2000));
        let a = run(&c, &[0, 700, 0, 100, 0, 0]);
        // valid 800, quotient 400: verde 700 -> 1, azul 100 -> 0; leftover: azul has 100 < 80% of 400
        // (strict round skips it), the third round gives it the seat
        assert_eq!(seats_of(&a), vec![1, 1]);
    }

    #[test]
    fn when_nobody_reaches_the_quotient_the_most_voted_candidate_wins_whatever_the_party() {
        // one seat, two parties with 50 each: neither reaches the quotient (100)
        let c = chamber(1, 1, 1, (0, 0, 0));
        // [L13, 1301, L45, 4501, blank, null]
        let a = run(&c, &[10, 40, 5, 45, 0, 0]);
        assert_eq!(a.quotient, 100);
        assert_eq!(a.elected(), vec![3]);
    }

    #[test]
    fn ties_matter_only_when_they_change_who_is_elected() {
        let c = chamber(2, 2, 1, (0, 0, 0));
        // [L13, 1301, 1302, L45, 4501, 4502, blank, null]
        // a clear winner: no draw
        assert!(!needs_draw(&c, &[0, 9, 1, 0, 5, 5, 0, 0]));
        // two candidates tied at the top of the same party for the only seat
        assert!(needs_draw(&c, &[0, 5, 5, 0, 1, 1, 0, 0]));
        // two parties tied for the only seat
        assert!(needs_draw(&c, &[0, 5, 0, 0, 5, 0, 0, 0]));
        // a tie that changes nothing (both are elected anyway)
        let c2 = chamber(2, 2, 3, (0, 0, 0));
        assert!(!needs_draw(&c2, &[0, 5, 5, 0, 9, 0, 0, 0]));
        // the drawn order decides the tie
        let drawn = allocate(&c, &[0, 5, 5, 0, 1, 1, 0, 0], &|i| 10 - i).unwrap();
        assert_eq!(drawn.elected(), vec![2]);
    }

    #[test]
    fn nothing_is_allocated_without_valid_votes_or_for_a_majoritarian_contest() {
        let c = chamber(1, 1, 2, (0, 0, 0));
        assert!(allocate(&c, &[0, 0, 0, 0, 3, 3], &|i| i).is_none());
        let mut m = c.clone();
        m.proportional = None;
        assert!(allocate(&m, &[1, 1, 1, 1, 0, 0], &|i| i).is_none());
        let _ = String::new();
    }

    use proptest::prelude::*;

    proptest! {
        /// Whatever the votes: nobody is seated twice, a party never holds more than the chamber, the
        /// chamber is never overfilled, the elected of a party are its most voted candidates, and
        /// blank and null never change who is elected.
        #[test]
        fn the_chamber_is_never_overfilled_and_each_party_seats_its_most_voted(
            seats in 1u32..12,
            a in 1usize..6,
            b in 1usize..6,
            votes in proptest::collection::vec(0u64..500, 20),
            bp in (0u32..3000, 0u32..10_000, 0u32..3000),
        ) {
            let c = chamber(a, b, seats, bp);
            let n = c.options.len();
            let mut t: Vec<u64> = (0..n).map(|i| votes[i % votes.len()]).collect();
            let r = allocate(&c, &t, &|i| i);
            if let Some(al) = r {
                let el = al.elected();
                let mut sorted = el.clone();
                sorted.sort_unstable();
                sorted.dedup();
                prop_assert_eq!(sorted.len(), el.len());
                prop_assert!(el.len() <= seats as usize);
                for p in &al.parties {
                    prop_assert_eq!(p.seats as usize, p.elected.len());
                    let best_left = c
                        .options
                        .iter()
                        .enumerate()
                        .filter(|(k, o)| o.is_candidate() && o.party == c.options[p.option].number && !p.elected.contains(k))
                        .map(|(k, _)| t[k])
                        .max();
                    if let Some(m) = best_left {
                        prop_assert!(p.elected.iter().all(|k| t[*k] >= m));
                    }
                }
                // blank and null (the last two options) are outside everything
                let before = al.elected();
                t[n - 1] += 1000;
                t[n - 2] += 1000;
                let again = allocate(&c, &t, &|i| i).unwrap();
                prop_assert_eq!(before, again.elected());
            }
        }
    }
}
