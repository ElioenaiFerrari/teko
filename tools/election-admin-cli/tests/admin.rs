//! The admin tool end to end on a real board file: publish, simulate, submit, tally.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::time::Duration;

use election_admin::{Admin, SIM_MARKER, feed_new, feed_step_with, new_tallier, tally_board};
use issuer_cli::{Issuer, scratch_dir};
use teko_bulletin::{BulletinBoard, FileBoard};
use teko_crypto::{public_key_bytes, voter::derive_voter_key};
use teko_election::{
    BallotBody, ElectionManifest, Phase, SubmittedBallot, SubmittedRegistration, contest_hash,
    registration_message, sign_ballot,
};
use teko_registration::{BlindSignature, blind, finalize};

fn setup(tag: &str) -> Admin {
    let issuer = Issuer::open(scratch_dir(tag));
    issuer.dev_setup().unwrap();
    let admin = Admin::open(issuer.dir()).unwrap();
    admin.publish().unwrap();
    admin
}

fn manifest(admin: &Admin) -> ElectionManifest {
    use prost::Message;
    let bytes = fs::read(admin.dir().join("election.bundle")).unwrap();
    let bundle = teko_election::ElectionBundle::decode(bytes.as_slice()).unwrap();
    ElectionManifest::decode(bundle.manifest.as_slice()).unwrap()
}

fn real_ballot(m: &ElectionManifest, contest: &str, root: u8, option: u32) -> SubmittedBallot {
    let key = derive_voter_key(&[root; 32], &m.election_id, contest).unwrap();
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
        voter_key: public_key_bytes(&key),
        signature: sign_ballot(&key, &body),
        encrypted: Vec::new(),
    }
}

/// What the Suite does for a real voter: blind, authority signs, unblind, publish.
fn register(admin: &Admin, m: &ElectionManifest, contest: &str, root: u8) {
    let key = derive_voter_key(&[root; 32], &m.election_id, contest).unwrap();
    let voter_key = public_key_bytes(&key);
    let spki = m.contest(contest).unwrap().registration_key.clone();
    let msg = registration_message(&m.election_id, contest, &voter_key);
    let b = blind(&spki, &msg).unwrap();
    let authority = Issuer::open(admin.dir()).registration_key(contest).unwrap();
    let sig = authority.blind_sign(&b.blind_message).unwrap();
    let token = finalize(&spki, &b.state, &BlindSignature(sig.0), &msg).unwrap();
    admin
        .submit_registration(&SubmittedRegistration {
            election_id: m.election_id.clone(),
            contest_id: contest.into(),
            voter_key,
            token: token.to_bytes(),
        })
        .unwrap();
}

#[test]
fn publish_is_idempotent_and_the_board_replays_to_the_demo_phases() {
    let admin = setup("pub");
    let again = admin.publish().unwrap();
    assert_eq!(again.appended, 0, "nothing is appended twice");
    let t = admin.tally().unwrap();
    assert_eq!(t.contests.len(), 7);
    assert_eq!(t.ignored_governance, 0);
    for id in [
        "presidente",
        "governador-sp",
        "governador-rj",
        "governador-mg",
        "governador-ba",
        "governador-rs",
    ] {
        assert_eq!(t.contest(id).unwrap().phase, Phase::VotingOpen, "{id}");
    }
    assert_eq!(
        t.contest("prefeito-3550308").unwrap().phase,
        Phase::CandidatesPublished
    );
    assert!(!admin.is_simulated());
    let _ = fs::remove_dir_all(admin.dir());
}

#[test]
fn simulated_voters_are_counted_valid_ranked_and_labelled_as_simulated() {
    let admin = setup("sim");
    let r = admin.simulate_batch(200, 7).unwrap();
    assert_eq!(r.contests, 6, "only contests with voting open get ballots");
    assert!(admin.is_simulated(), "the board is marked as simulated");
    let t = admin.tally().unwrap();
    let pres = t.contest("presidente").unwrap();
    assert_eq!((pres.accepted, pres.rejected.total()), (200, 0));
    assert_eq!(
        t.contest("governador-sp").unwrap().accepted,
        44,
        "synthetic 22% share"
    );
    assert_eq!(
        t.contest("prefeito-3550308").unwrap().accepted,
        0,
        "not open: no ballots"
    );
    // 12 candidates in SP: the top 10 + an "others" bucket that adds up
    let sp = t.contest("governador-sp").unwrap();
    let rk = sp.ranked(10);
    assert_eq!((rk.top.len(), rk.others_count), (10, 2));
    assert_eq!(
        rk.top.iter().map(|o| o.votes).sum::<u64>() + rk.others_votes,
        sp.accepted
    );
    assert!(
        rk.top.windows(2).all(|w| w[0].votes >= w[1].votes),
        "sorted high to low"
    );
    // same seed again = NEW voters (the marker advances), so counts add up instead of colliding
    admin.simulate_batch(200, 7).unwrap();
    let t2 = admin.tally().unwrap();
    assert_eq!(t2.contest("presidente").unwrap().accepted, 400);
    assert_eq!(t2.contest("presidente").unwrap().rejected.duplicate, 0);
    let _ = fs::remove_dir_all(admin.dir());
}

#[test]
fn a_real_ballot_is_counted_once_and_rejected_outside_the_voting_phase() {
    let admin = setup("real");
    let m = manifest(&admin);
    register(&admin, &m, "presidente", 1);
    admin
        .submit_ballot(&real_ballot(&m, "presidente", 1, 1))
        .unwrap();
    admin
        .submit_ballot(&real_ballot(&m, "presidente", 1, 2))
        .unwrap(); // same voter again
    admin
        .submit_ballot(&real_ballot(&m, "prefeito-3550308", 1, 0))
        .unwrap(); // not open yet
    let t = admin.tally().unwrap();
    let p = t.contest("presidente").unwrap();
    assert_eq!((p.accepted, p.rejected.duplicate), (1, 1));
    assert_eq!(p.options[1].votes, 1, "the first ballot stands");
    let mayor = t.contest("prefeito-3550308").unwrap();
    assert_eq!((mayor.accepted, mayor.rejected.not_open), (0, 1));
    let _ = fs::remove_dir_all(admin.dir());
}

#[test]
fn only_registered_keys_count_and_a_token_is_bound_to_its_contest() {
    let admin = setup("reg");
    let m = manifest(&admin);
    register(&admin, &m, "presidente", 1);
    // voter 2 never registered:
    admin
        .submit_ballot(&real_ballot(&m, "presidente", 1, 0))
        .unwrap();
    admin
        .submit_ballot(&real_ballot(&m, "presidente", 2, 0))
        .unwrap();
    // voter 3 registered for the GOVERNOR only; the token must not work for President
    register(&admin, &m, "governador-sp", 3);
    admin
        .submit_ballot(&real_ballot(&m, "presidente", 3, 0))
        .unwrap();
    admin
        .submit_ballot(&real_ballot(&m, "governador-sp", 3, 1))
        .unwrap();
    let t = admin.tally().unwrap();
    let p = t.contest("presidente").unwrap();
    assert_eq!(
        (p.accepted, p.rejected.unregistered, p.registered),
        (1, 2, 1)
    );
    let g = t.contest("governador-sp").unwrap();
    assert_eq!(
        (g.accepted, g.registered, g.registration_required),
        (1, 1, true)
    );
    let _ = fs::remove_dir_all(admin.dir());
}

#[test]
fn incremental_feeding_follows_the_board_and_equals_a_full_replay() {
    let admin = setup("inc");
    let m = manifest(&admin);
    let mut t = new_tallier(admin.policy());
    let (board, _) = FileBoard::open_tolerant(admin.board_path()).unwrap();
    let consumed = feed_new(&mut t, &board);
    assert_eq!(consumed, board.len());
    for root in 1..=5u8 {
        register(&admin, &m, "governador-sp", root);
        admin
            .submit_ballot(&real_ballot(&m, "governador-sp", root, u32::from(root) % 3))
            .unwrap();
        let (board, _) = FileBoard::open_tolerant(admin.board_path()).unwrap();
        feed_new(&mut t, &board);
        assert_eq!(
            t.snapshot(),
            tally_board(admin.policy(), &board),
            "after ballot {root}"
        );
    }
    assert_eq!(t.snapshot().contest("governador-sp").unwrap().accepted, 5);
    let _ = fs::remove_dir_all(admin.dir());
}

#[test]
fn live_simulation_streams_ballots_that_a_reader_can_follow() {
    let admin = setup("live");
    let mut ticks = Vec::new();
    let r = admin
        .simulate_live(80, Duration::from_millis(900), 3, |n| ticks.push(n))
        .unwrap();
    assert!(r.ballots >= 40, "{r:?}");
    assert!(
        ticks.len() >= 2 && ticks.windows(2).all(|w| w[0] < w[1]),
        "grows tick by tick: {ticks:?}"
    );
    let t = admin.tally().unwrap();
    let total: u64 = t.contests.iter().map(|c| c.accepted).sum();
    assert_eq!(
        total, r.ballots,
        "every streamed ballot is valid and counted"
    );
    assert_eq!(
        t.contests.iter().map(|c| c.rejected.total()).sum::<u64>(),
        0
    );
    assert!(fs::read_to_string(admin.dir().join(SIM_MARKER)).is_ok());
    let _ = fs::remove_dir_all(admin.dir());
}

#[test]
fn tallying_while_a_writer_is_mid_append_still_works() {
    let admin = setup("tail");
    admin.simulate_batch(30, 5).unwrap();
    let total = |a: &Admin| -> u64 { a.tally().unwrap().contests.iter().map(|c| c.accepted).sum() };
    let before = total(&admin);
    assert!(before > 30, "president + governors");
    // A writer is mid-record: cut the last bytes of the file.
    let bytes = fs::read(admin.board_path()).unwrap();
    fs::write(admin.board_path(), &bytes[..bytes.len() - 5]).unwrap();
    assert_eq!(
        total(&admin),
        before - 1,
        "the live reader loses exactly the unfinished record"
    );
    let _ = fs::remove_dir_all(admin.dir());
}

#[test]
fn at_the_deadline_governance_closes_the_contest_and_the_log_locks_the_vote() {
    let admin = setup("close");
    let m = manifest(&admin);
    register(&admin, &m, "presidente", 1);
    register(&admin, &m, "presidente", 2);
    admin
        .submit_ballot(&real_ballot(&m, "presidente", 1, 1))
        .unwrap();
    let closes = m
        .contest("presidente")
        .unwrap()
        .schedule
        .clone()
        .unwrap()
        .voting_closes;

    // Before the deadline nothing closes.
    assert_eq!(admin.close_due(closes - 1).unwrap(), 0);
    // After it, every open contest is closed (7 contests: 6 open + the mayor only announced).
    assert_eq!(admin.close_due(closes + 1).unwrap(), 6);
    assert_eq!(admin.close_due(closes + 2).unwrap(), 0, "idempotent");

    // A ballot appended AFTER the closing is refused by the public tally.
    admin
        .submit_ballot(&real_ballot(&m, "presidente", 2, 0))
        .unwrap();
    let t = admin.tally().unwrap();
    let p = t.contest("presidente").unwrap();
    assert_eq!(p.phase, Phase::VotingClosed);
    assert_eq!((p.accepted, p.rejected.not_open), (1, 1));
    assert_eq!(
        t.ignored_governance, 0,
        "the closing is a valid signed step"
    );
    let _ = fs::remove_dir_all(admin.dir());
}

/// A board with many ballots of several kinds (valid, duplicate, unregistered, not open).
fn busy_board(tag: &str, voters: u8) -> (Admin, teko_bulletin::FileBoard) {
    let admin = setup(tag);
    let m = manifest(&admin);
    for root in 1..=voters {
        register(&admin, &m, "governador-sp", root);
        admin
            .submit_ballot(&real_ballot(&m, "governador-sp", root, u32::from(root) % 3))
            .unwrap();
        if root % 5 == 0 {
            // the same voter again: a duplicate
            admin
                .submit_ballot(&real_ballot(&m, "governador-sp", root, 1))
                .unwrap();
        }
        if root % 7 == 0 {
            // never registered for the President: refused
            admin
                .submit_ballot(&real_ballot(&m, "presidente", root, 0))
                .unwrap();
        }
    }
    let (board, _) = FileBoard::open_tolerant(admin.board_path()).unwrap();
    (admin, board)
}

#[test]
fn checking_ballots_on_many_threads_gives_the_sequential_result() {
    let (admin, board) = busy_board("par", 60);
    let want = tally_board(admin.policy(), &board);
    assert!(want.contest("governador-sp").unwrap().accepted >= 50);
    assert!(want.contest("governador-sp").unwrap().rejected.duplicate > 0);
    for threads in [1usize, 2, 4, 8] {
        let mut t = new_tallier(admin.policy());
        let n = feed_step_with(&mut t, &board, u64::MAX, threads);
        assert_eq!(n, board.len(), "{threads} threads");
        assert_eq!(t.snapshot(), want, "{threads} threads");
    }
    let _ = fs::remove_dir_all(admin.dir());
}

#[test]
fn feeding_in_steps_reports_progress_and_ends_at_the_same_state() {
    let (admin, board) = busy_board("steps", 30);
    let want = tally_board(admin.policy(), &board);
    let mut t = new_tallier(admin.policy());
    let mut last = 0;
    let mut steps = 0;
    while last < board.len() {
        let now = feed_step_with(&mut t, &board, 7, 4);
        assert!(
            now > last && now <= last + 7,
            "each step moves at most 7 entries"
        );
        last = now;
        steps += 1;
    }
    assert!(steps > 3, "the walk takes several steps");
    assert_eq!(t.snapshot(), want);
    // already caught up: nothing more to do
    assert_eq!(feed_step_with(&mut t, &board, 7, 4), board.len());
    let _ = fs::remove_dir_all(admin.dir());
}

#[test]
fn the_block_simulators_filler_entries_never_change_a_count() {
    let admin = setup("filler");
    let before = admin.tally().unwrap();
    // what `devnet-sim blocks --kind votes` puts on the board (reserved kind 200)
    {
        let mut board = FileBoard::open(admin.board_path()).unwrap();
        for i in 0..5u8 {
            board
                .append(teko_bulletin::Entry::new(
                    teko_bulletin::EntryKind(200),
                    vec![i; 24],
                ))
                .unwrap();
        }
    }
    let after = admin.tally().unwrap();
    for c in &before.contests {
        let a = after.contest(&c.contest_id).unwrap();
        assert_eq!(
            (a.accepted, a.rejected.total()),
            (c.accepted, c.rejected.total())
        );
    }
}
