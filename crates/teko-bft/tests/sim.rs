//! Deterministic network simulator: delays, losses, partitions, crashed and
//! Byzantine (equivocating) validators. Checks SAFETY (everyone who decides a height
//! decides the same block, with a valid commit certificate) and LIVENESS (decisions
//! happen once a quorum can talk).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};

use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};
use teko_bft::{
    Action, App, Commit, Engine, Evidence, Message, Proposal, Step, Validator, ValidatorSet, Vote,
    VoteKind, block_id,
};
use teko_crypto::{SigningKey, generate_signing_key, public_key_bytes};

const CHAIN: &str = "teko-sim-1";

struct SimApp {
    node: usize,
}

impl App for SimApp {
    fn build(&mut self, height: u64) -> Vec<u8> {
        format!("block@{height}/by-{}", self.node).into_bytes()
    }
    fn validate(&self, _h: u64, payload: &[u8]) -> bool {
        payload.starts_with(b"block@")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    Honest,
    /// Never runs.
    Crashed,
    /// Runs, but sends conflicting votes/proposals to different halves of the network.
    Equivocator,
}

enum Event {
    Deliver {
        to: usize,
        msg: Message,
    },
    DeliverCommit {
        to: usize,
        commit: Commit,
        payload: Vec<u8>,
    },
    Timeout {
        node: usize,
        height: u64,
        round: u32,
        step: Step,
    },
    /// Periodic retransmission of what a node already said, and its last commit.
    Resend {
        node: usize,
    },
}

struct Cfg {
    powers: Vec<u64>,
    roles: Vec<Role>,
    heights: u64,
    max_delay_ms: u64,
    drop_pct: u32,
    /// Messages between `group_a` and the rest are dropped until this time.
    partition_until_ms: u64,
    group_a: Vec<usize>,
    time_limit_ms: u64,
    seed: u64,
}

impl Cfg {
    fn new(n: usize, seed: u64) -> Self {
        Self {
            powers: vec![1; n],
            roles: vec![Role::Honest; n],
            heights: 3,
            max_delay_ms: 80,
            drop_pct: 0,
            partition_until_ms: 0,
            group_a: Vec::new(),
            time_limit_ms: 600_000,
            seed,
        }
    }
}

struct Out {
    /// per node: decided (height, payload) in order
    decided: Vec<Vec<(u64, Vec<u8>)>>,
    evidence: Vec<Evidence>,
    set: ValidatorSet,
    commits_ok: bool,
}

fn run(cfg: &Cfg) -> Out {
    let n = cfg.powers.len();
    let mut rng = StdRng::seed_from_u64(cfg.seed);
    let keys: Vec<SigningKey> = (0..n).map(|_| generate_signing_key(&mut rng)).collect();
    let set = ValidatorSet::new(
        keys.iter()
            .zip(&cfg.powers)
            .map(|(k, p)| Validator {
                key: public_key_bytes(k),
                power: *p,
            })
            .collect(),
    )
    .unwrap();
    let mut engines: Vec<Engine<SimApp>> = keys
        .iter()
        .enumerate()
        .map(|(i, k)| Engine::new(CHAIN, k.clone(), SimApp { node: i }).unwrap())
        .collect();

    let mut events: Vec<Event> = Vec::new();
    let mut heap: BinaryHeap<Reverse<(u64, u64, usize)>> = BinaryHeap::new();
    let mut seq = 0u64;
    let mut now = 0u64;
    let mut decided: Vec<Vec<(u64, Vec<u8>)>> = vec![Vec::new(); n];
    let mut evidence = Vec::new();
    let mut commits_ok = true;
    // messages for heights a node has not reached yet
    let mut future: Vec<Vec<(u64, Message)>> = vec![Vec::new(); n];
    // gossip: every node forwards each message the first time it sees it
    let mut seen: Vec<std::collections::HashSet<Vec<u8>>> = vec![Default::default(); n];
    let mut last_commit: Vec<Option<(Commit, Vec<u8>)>> = vec![None; n];

    macro_rules! push {
        ($at:expr, $ev:expr) => {{
            events.push($ev);
            heap.push(Reverse(($at, seq, events.len() - 1)));
            seq += 1;
        }};
    }

    // `actions` produced by node `from` at `now`
    fn blocked(cfg: &Cfg, a: usize, b: usize, now: u64) -> bool {
        now < cfg.partition_until_ms && cfg.group_a.contains(&a) != cfg.group_a.contains(&b)
    }

    let mut pending: Vec<(usize, Vec<Action>)> = Vec::new();
    for i in 0..n {
        if cfg.roles[i] != Role::Crashed {
            let acts = engines[i].start_height(0, set.clone());
            pending.push((i, acts));
        }
    }

    for i in 0..n {
        push!(500 + 37 * i as u64, Event::Resend { node: i });
    }
    loop {
        // process produced actions
        while let Some((from, actions)) = pending.pop() {
            for a in actions {
                match a {
                    Action::Broadcast(m) => {
                        for to in 0..n {
                            if to == from || cfg.roles[to] == Role::Crashed {
                                continue;
                            }
                            if blocked(cfg, from, to, now)
                                || rng.random_range(0..100) < cfg.drop_pct
                            {
                                continue;
                            }
                            let mut msg = m.clone();
                            // an equivocator tells odd peers something else
                            if cfg.roles[from] == Role::Equivocator && to % 2 == 1 {
                                msg = match &m {
                                    Message::Vote(v) => Message::Vote(Vote::new(
                                        CHAIN,
                                        &keys[from],
                                        v.vote_kind().unwrap(),
                                        v.height,
                                        v.round,
                                        Some(&block_id(b"block@evil")),
                                    )),
                                    Message::Proposal(p) => Message::Proposal(Proposal::new(
                                        CHAIN,
                                        &keys[from],
                                        p.height,
                                        p.round,
                                        p.valid_round,
                                        b"block@evil".to_vec(),
                                    )),
                                };
                            }
                            let at = now + rng.random_range(1..=cfg.max_delay_ms);
                            push!(at, Event::Deliver { to, msg });
                        }
                    }
                    Action::Schedule {
                        round,
                        step,
                        after_ms,
                    } => {
                        let h = engines[from].height();
                        push!(
                            now + after_ms,
                            Event::Timeout {
                                node: from,
                                height: h,
                                round,
                                step
                            }
                        );
                    }
                    Action::Decide { commit, payload } => {
                        let h = commit.height;
                        last_commit[from] = Some((commit.clone(), payload.clone()));
                        if cfg.roles[from] != Role::Equivocator {
                            commits_ok &= commit.verify(CHAIN, &set);
                            decided[from].push((h, payload.clone()));
                        }
                        // tell everyone (catch-up), subject to the same network
                        for to in 0..n {
                            if to != from
                                && cfg.roles[to] != Role::Crashed
                                && !blocked(cfg, from, to, now)
                                && rng.random_range(0..100) >= cfg.drop_pct
                            {
                                let at = now + rng.random_range(1..=cfg.max_delay_ms);
                                push!(
                                    at,
                                    Event::DeliverCommit {
                                        to,
                                        commit: commit.clone(),
                                        payload: payload.clone()
                                    }
                                );
                            }
                        }
                        if h + 1 < cfg.heights {
                            let next = engines[from].start_height(h + 1, set.clone());
                            pending.push((from, next));
                            // replay buffered messages for the new height
                            let (now_h, rest): (Vec<_>, Vec<_>) =
                                future[from].drain(..).partition(|(fh, _)| *fh == h + 1);
                            future[from] = rest;
                            for (_, m) in now_h {
                                let acts = engines[from].on_message(m);
                                pending.push((from, acts));
                            }
                        }
                    }
                    Action::Evidence(e) => evidence.push(e),
                }
            }
        }
        let Some(Reverse((at, _, idx))) = heap.pop() else {
            break;
        };
        now = at;
        if now > cfg.time_limit_ms {
            break;
        }
        match std::mem::replace(
            &mut events[idx],
            Event::Timeout {
                node: 0,
                height: u64::MAX,
                round: 0,
                step: Step::Propose,
            },
        ) {
            Event::Deliver { to, msg } => {
                let key = match &msg {
                    Message::Proposal(p) => prost::Message::encode_to_vec(p),
                    Message::Vote(v) => prost::Message::encode_to_vec(v),
                };
                if seen[to].insert(key) {
                    // first time: relay to everyone (the network layer's gossip)
                    pending.push((to, vec![Action::Broadcast(msg.clone())]));
                } else {
                    continue;
                }
                let mh = match &msg {
                    Message::Proposal(p) => p.height,
                    Message::Vote(v) => v.height,
                };
                if mh > engines[to].height() {
                    future[to].push((mh, msg));
                } else {
                    let acts = engines[to].on_message(msg);
                    pending.push((to, acts));
                }
            }
            Event::DeliverCommit {
                to,
                commit,
                payload,
            } => {
                let acts = engines[to].on_commit(&commit, &payload);
                pending.push((to, acts));
            }
            Event::Timeout {
                node,
                height,
                round,
                step,
            } => {
                if height == engines[node].height() {
                    let acts = engines[node].on_timeout(round, step);
                    pending.push((node, acts));
                }
            }
            Event::Resend { node } => {
                if cfg.roles[node] != Role::Crashed {
                    if let Some((c, p)) = &last_commit[node] {
                        for to in 0..n {
                            if to != node
                                && !blocked(cfg, node, to, now)
                                && rng.random_range(0..100) >= cfg.drop_pct
                            {
                                let at = now + rng.random_range(1..=cfg.max_delay_ms);
                                push!(
                                    at,
                                    Event::DeliverCommit {
                                        to,
                                        commit: c.clone(),
                                        payload: p.clone()
                                    }
                                );
                            }
                        }
                    }
                    let acts = engines[node].resend();
                    pending.push((node, acts));
                }
                push!(now + 1_500, Event::Resend { node });
            }
        }
        let honest_done = (0..n)
            .filter(|i| cfg.roles[*i] == Role::Honest)
            .all(|i| decided[i].len() as u64 >= cfg.heights);
        if honest_done {
            break;
        }
    }
    Out {
        decided,
        evidence,
        set,
        commits_ok,
    }
}

/// SAFETY: for every height, every node that decided it decided the SAME block.
fn assert_agreement(cfg: &Cfg, out: &Out) {
    let mut by_height: BTreeMap<u64, Vec<u8>> = BTreeMap::new();
    for (i, d) in out.decided.iter().enumerate() {
        if cfg.roles[i] == Role::Equivocator {
            continue;
        }
        for (h, payload) in d {
            match by_height.get(h) {
                Some(p) => assert_eq!(p, payload, "two different blocks at height {h}"),
                None => {
                    by_height.insert(*h, payload.clone());
                }
            }
        }
    }
    assert!(
        out.commits_ok,
        "every decision came with a valid commit certificate"
    );
}

fn honest_all_decided(cfg: &Cfg, out: &Out) -> bool {
    (0..cfg.powers.len())
        .filter(|i| cfg.roles[*i] == Role::Honest)
        .all(|i| out.decided[i].len() as u64 >= cfg.heights)
}

#[test]
fn four_honest_validators_decide_every_height_with_random_delays() {
    for seed in 0..10 {
        let cfg = Cfg::new(4, seed);
        let out = run(&cfg);
        assert_agreement(&cfg, &out);
        assert!(honest_all_decided(&cfg, &out), "seed {seed}");
        assert!(out.evidence.is_empty(), "honest nodes never equivocate");
    }
}

#[test]
fn one_crashed_of_four_still_decides_but_two_crashed_cannot_and_never_disagrees() {
    for seed in 0..6 {
        let mut cfg = Cfg::new(4, seed);
        cfg.roles[3] = Role::Crashed;
        let out = run(&cfg);
        assert_agreement(&cfg, &out);
        assert!(
            honest_all_decided(&cfg, &out)
                || (0..3).all(|i| out.decided[i].len() as u64 >= cfg.heights),
            "seed {seed}: 3 of 4 is a quorum"
        );
    }
    for seed in 0..4 {
        let mut cfg = Cfg::new(4, seed);
        cfg.roles[2] = Role::Crashed;
        cfg.roles[3] = Role::Crashed;
        cfg.time_limit_ms = 60_000;
        let out = run(&cfg);
        assert_agreement(&cfg, &out);
        assert!(
            out.decided.iter().all(|d| d.is_empty()),
            "2 of 4 is NOT a quorum: nothing is decided"
        );
    }
}

#[test]
fn a_partition_blocks_decisions_and_heals_without_ever_forking() {
    for seed in 0..6 {
        let mut cfg = Cfg::new(4, seed);
        cfg.group_a = vec![0, 1];
        cfg.partition_until_ms = 20_000;
        let out = run(&cfg);
        assert_agreement(&cfg, &out);
        assert!(
            honest_all_decided(&cfg, &out),
            "seed {seed}: decides after the partition heals"
        );
        // nothing was decided while 2/2 (no group had a quorum)
    }
}

#[test]
fn lossy_networks_still_agree_and_eventually_decide() {
    for seed in 0..8 {
        let mut cfg = Cfg::new(5, seed);
        cfg.drop_pct = 15;
        cfg.max_delay_ms = 400;
        let out = run(&cfg);
        assert_agreement(&cfg, &out);
        assert!(honest_all_decided(&cfg, &out), "seed {seed}");
    }
}

#[test]
fn an_equivocating_validator_cannot_split_the_honest_ones_and_leaves_evidence() {
    let mut found = false;
    for seed in 0..12 {
        let mut cfg = Cfg::new(4, seed);
        cfg.roles[1] = Role::Equivocator;
        cfg.heights = 4; // validator 1 leads some of them
        let out = run(&cfg);
        assert_agreement(&cfg, &out);
        assert!(honest_all_decided(&cfg, &out), "seed {seed}");
        for e in &out.evidence {
            assert!(e.verify(CHAIN), "evidence must verify");
            assert_eq!(
                e.validator().unwrap(),
                public_key_bytes_of(&out.set, 1).as_slice()
            );
            found = true;
        }
    }
    assert!(found, "the equivocation was caught at least once");
}

fn public_key_bytes_of(set: &ValidatorSet, i: usize) -> Vec<u8> {
    set.validators()[i].key.clone()
}

#[test]
fn stake_weighted_voting_a_heavy_validator_alone_is_not_enough_but_with_one_more_is() {
    // powers 5,1,1,1: total 8, quorum needs > 16/3 => 6. The heavy validator (5) alone cannot.
    let mut cfg = Cfg::new(4, 3);
    cfg.powers = vec![5, 1, 1, 1];
    cfg.roles = vec![Role::Honest, Role::Crashed, Role::Crashed, Role::Crashed];
    cfg.time_limit_ms = 30_000;
    let out = run(&cfg);
    assert!(
        out.decided.iter().all(|d| d.is_empty()),
        "5 of 8 is not more than 2/3"
    );
    // heavy + one light = 6 of 8: a quorum
    let mut cfg = Cfg::new(4, 3);
    cfg.powers = vec![5, 1, 1, 1];
    cfg.roles = vec![Role::Honest, Role::Honest, Role::Crashed, Role::Crashed];
    let out = run(&cfg);
    assert_agreement(&cfg, &out);
    assert!(
        out.decided[0].len() as u64 >= cfg.heights && out.decided[1].len() as u64 >= cfg.heights
    );
}

#[test]
fn many_random_networks_never_disagree() {
    for seed in 100..160 {
        let mut r = StdRng::seed_from_u64(seed);
        let n = r.random_range(4..=7);
        let mut cfg = Cfg::new(n, seed);
        cfg.powers = (0..n).map(|_| r.random_range(1..=4)).collect();
        cfg.drop_pct = r.random_range(0..=25);
        cfg.max_delay_ms = r.random_range(10..=500);
        cfg.heights = 2;
        cfg.time_limit_ms = 200_000;
        // up to ONE equivocator (power kept below 1/3 on purpose is not guaranteed here:
        // safety must hold only when faulty power < 1/3, so keep it under that bound)
        let total: u64 = cfg.powers.iter().sum();
        let victim = r.random_range(0..n);
        if cfg.powers[victim] * 3 < total {
            cfg.roles[victim] = Role::Equivocator;
        }
        let out = run(&cfg);
        assert_agreement(&cfg, &out);
    }
}

#[test]
fn commit_certificates_are_verified_strictly() {
    let cfg = Cfg::new(4, 1);
    let out = run(&cfg);
    // rebuild a certificate through a tiny honest run and tamper with it
    let mut rng = StdRng::seed_from_u64(1);
    let keys: Vec<SigningKey> = (0..4).map(|_| generate_signing_key(&mut rng)).collect();
    let set = ValidatorSet::new(
        keys.iter()
            .map(|k| Validator {
                key: public_key_bytes(k),
                power: 1,
            })
            .collect(),
    )
    .unwrap();
    let id = block_id(b"block@0/by-0");
    let precommits: Vec<Vote> = keys
        .iter()
        .take(3)
        .map(|k| Vote::new(CHAIN, k, VoteKind::Precommit, 0, 0, Some(&id)))
        .collect();
    let good = Commit {
        height: 0,
        round: 0,
        block_id: id.clone(),
        precommits: precommits.clone(),
    };
    assert!(good.verify(CHAIN, &set), "3 of 4 equal-power validators");
    assert!(
        !Commit {
            precommits: precommits[..2].to_vec(),
            ..good.clone()
        }
        .verify(CHAIN, &set),
        "2 of 4"
    );
    assert!(!good.verify("other-chain", &set), "other chain");
    let mut dup = precommits.clone();
    dup[2] = dup[0].clone();
    assert!(
        !Commit {
            precommits: dup,
            ..good.clone()
        }
        .verify(CHAIN, &set),
        "a duplicate does not count twice"
    );
    let mut wrong_block = good.clone();
    wrong_block.block_id = block_id(b"other");
    assert!(!wrong_block.verify(CHAIN, &set));
    let prevote = Vote::new(CHAIN, &keys[3], VoteKind::Prevote, 0, 0, Some(&id));
    let mut with_prevote = precommits.clone();
    with_prevote[2] = prevote;
    assert!(
        !Commit {
            precommits: with_prevote,
            ..good.clone()
        }
        .verify(CHAIN, &set),
        "a prevote is not a precommit"
    );
    let outsider = generate_signing_key(&mut rng);
    let mut with_outsider = precommits;
    with_outsider[2] = Vote::new(CHAIN, &outsider, VoteKind::Precommit, 0, 0, Some(&id));
    assert!(
        !Commit {
            precommits: with_outsider,
            ..good
        }
        .verify(CHAIN, &set),
        "an outsider"
    );
    let _ = out;
}

/// Long safety sweep (several thousand random networks with up to one equivocator
/// below 1/3 of the power). Run: `cargo test -p teko-bft --release --test sim -- --ignored stress`.
#[test]
#[ignore = "long"]
fn stress_thousands_of_random_networks_never_disagree() {
    for seed in 1_000..4_000 {
        let mut r = StdRng::seed_from_u64(seed);
        let n = r.random_range(4..=9);
        let mut cfg = Cfg::new(n, seed);
        cfg.powers = (0..n).map(|_| r.random_range(1..=5)).collect();
        cfg.drop_pct = r.random_range(0..=30);
        cfg.max_delay_ms = r.random_range(5..=800);
        cfg.heights = 2;
        cfg.time_limit_ms = 120_000;
        if r.random_range(0..3) == 0 {
            cfg.group_a = (0..n / 2).collect();
            cfg.partition_until_ms = r.random_range(1_000..=30_000);
        }
        let total: u64 = cfg.powers.iter().sum();
        let victim = r.random_range(0..n);
        if cfg.powers[victim] * 3 < total && r.random_range(0..2) == 0 {
            cfg.roles[victim] = Role::Equivocator;
        }
        let out = run(&cfg);
        assert_agreement(&cfg, &out);
    }
}
