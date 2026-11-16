//! The verifiable DRAW (ADR-0043) on REAL nodes: ties are planted on purpose, the contests close,
//! the validators publish their shares of the draw, the institutions sign the drawn result, anyone
//! redoes the draw from the board, and the economy elects the drawn winner. One institution is
//! down the whole time: the threshold, not everybody, is what the draw needs.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::TcpListener;
use std::path::Path;
use std::time::{Duration, Instant};

use election_admin::Admin;
use election_admin::elected::{build_election, claim_elected, outcomes, start_runoff, verify_draw};
use issuer_cli::{Issuer, scratch_dir};
use teko_chain::NetworkKind;
use teko_election::{ElectionManifest, OutcomeKind, Phase};
use teko_node::{
    ECONOMY_FILE, EconomyConfig, Node, NodeConfig, NodeHandle, init_economy_with_issuer,
    init_network, load_economy, load_network, load_validator_key, node_dir,
};

fn ports() -> u16 {
    loop {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let p = l.local_addr().unwrap().port();
        if p < 60_000 && (1..5).all(|k| TcpListener::bind(("127.0.0.1", p + k)).is_ok()) {
            return p;
        }
    }
}

fn start(
    dir: &Path,
    i: usize,
    ec: &EconomyConfig,
    trust: Option<teko_attestation::TrustPolicy>,
) -> NodeHandle {
    let net = load_network(&dir.join("net")).unwrap();
    let addr = net.peers[i];
    Node::start(
        NodeConfig {
            dir: node_dir(&dir.join("net"), i),
            profile: net.profile.with_block_interval_ms(150).unwrap(),
            key: load_validator_key(&dir.join("net"), i).unwrap(),
            index: i,
            peers: net.peers,
            allow_remote: false,
            trust,
            economy: Some(ec.clone()),
        },
        addr,
    )
    .unwrap()
}

fn wait(what: &str, secs: u64, mut f: impl FnMut() -> bool) {
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(secs) {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("timed out waiting for: {what}");
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[test]
fn ties_are_broken_by_a_draw_nobody_controls_that_anyone_can_redo_and_it_works_with_an_institution_down()
 {
    let issuer = Issuer::open(scratch_dir("draw"));
    issuer.dev_setup().unwrap();
    let dir = issuer.dir().to_path_buf();
    let net = dir.join("net");
    init_network(&net, NetworkKind::Devnet, 4, ports(), false).unwrap();
    let mut g = init_economy_with_issuer(&net, None, None, None, None, false).unwrap();
    for t in &mut g.credtypes {
        if t.name == "candidato" {
            t.contest_secs = 2;
        }
    }
    std::fs::write(net.join(ECONOMY_FILE), g.to_text()).unwrap();
    let ec = load_economy(&net).unwrap().unwrap();

    let boot: Vec<NodeHandle> = (0..4).map(|i| start(&dir, i, &ec, None)).collect();
    let netcfg = load_network(&net).unwrap();
    teko_node::run_ceremony(
        &teko_node::NodeClient::new(netcfg.peers.clone()),
        &net,
        b"teko-devnet-1|tally-key",
        2,
    )
    .unwrap();
    devnet_sim::seed_candidates(&net, "geral-2026").unwrap();
    let last = teko_node::NodeWorld::open(&net)
        .unwrap()
        .chain()
        .last_timestamp();
    wait("the clock passes the contest window", 120, || {
        now() > last + 3
    });
    build_election(&dir, "geral-2026", 30 * 86_400, now()).unwrap();
    for n in boot {
        n.stop();
    }
    let bytes = std::fs::read(dir.join("election.bundle")).unwrap();
    let b = <teko_election::ElectionBundle as prost::Message>::decode(bytes.as_slice()).unwrap();
    let m = <ElectionManifest as prost::Message>::decode(b.manifest.as_slice()).unwrap();
    let trust = teko_attestation::TrustPolicy::parse(
        &std::fs::read_to_string(dir.join("trust.txt")).unwrap(),
    )
    .unwrap();
    let mut nodes: Vec<NodeHandle> = (0..4)
        .map(|i| start(&dir, i, &ec, Some(trust.clone())))
        .collect();
    let admin = Admin::open(&dir).unwrap();
    admin.publish().unwrap();
    wait("election published", 40, || {
        admin.tally().is_ok_and(|t| {
            t.contest("governador-mg")
                .is_some_and(|c| c.phase == Phase::VotingOpen)
        })
    });

    // planted ties: two candidates tied (the draw ELECTS), a tie for the second finalist slot, and a
    // three-way tie (the draw picks two finalists)
    let plan: [(&str, &[u64]); 3] = [
        ("governador-mg", &[3, 3]),
        ("governador-sp", &[4, 2, 2]),
        ("governador-rj", &[3, 3, 3]),
    ];
    let mut planted = 0;
    for (id, votes) in plan {
        planted += admin.simulate_exact(id, votes, 5).unwrap().ballots;
    }
    wait("ballots counted", 120, || {
        admin
            .tally()
            .is_ok_and(|t| t.contests.iter().map(|c| c.accepted).sum::<u64>() == planted)
    });
    // an institution goes down before the close: 3 of 4 is still more than 2/3, and a threshold of 2
    nodes.remove(3).stop();
    let closes = m
        .contest("governador-mg")
        .unwrap()
        .schedule
        .clone()
        .unwrap()
        .voting_closes;
    admin.close_due(closes + 1).unwrap();
    wait("totals revealed", 120, || {
        admin.tally().is_ok_and(|t| {
            plan.iter()
                .all(|(id, _)| t.contest(id).is_some_and(|c| c.revealed))
        })
    });
    wait("drawn outcomes certified", 180, || {
        outcomes(&admin).is_ok_and(|o| {
            plan.iter().all(|(id, _)| {
                o.iter()
                    .any(|x| &x.outcome.contest_id == id && x.outcome.drawn && x.is_certified())
            })
        })
    });
    let list = outcomes(&admin).unwrap();
    let get = |id: &str| {
        list.iter()
            .find(|x| x.outcome.contest_id == id && x.outcome.drawn)
            .unwrap_or_else(|| panic!("no drawn outcome for {id}"))
    };

    // two candidates tied: the draw elects one of them
    let mg = get("governador-mg");
    assert_eq!(mg.outcome.kind(), OutcomeKind::Elected);
    assert!(mg.outcome.winner_option < 2);
    assert_eq!(mg.outcome.draw_tied.len(), 2);
    assert_eq!(mg.outcome.winner_option, mg.outcome.draw_tied[0]);
    // a tie for the second slot: the leader and one of the tied two
    let sp = get("governador-sp");
    assert_eq!(sp.outcome.kind(), OutcomeKind::Runoff);
    assert_eq!(sp.outcome.finalists[0], 0);
    assert!(matches!(sp.outcome.finalists[1], 1 | 2));
    // a three-way tie: two different finalists among the three
    let rj = get("governador-rj");
    assert_eq!(rj.outcome.kind(), OutcomeKind::Runoff);
    assert_ne!(rj.outcome.finalists[0], rj.outcome.finalists[1]);
    assert!(rj.outcome.finalists.iter().all(|f| *f < 3));
    for c in [mg, sp, rj] {
        assert!(
            c.signers >= 3,
            "{}: {} signers",
            c.outcome.contest_id,
            c.signers
        );
        // ANYONE redoes the draw from the board and gets the very same bytes
        let why = verify_draw(&admin, &c.outcome).unwrap();
        assert!(why.contains("sorteio refeito"), "{why}");
    }

    // the drawn winner gets the lifetime credential; the drawn finalists go to the second round
    let said = claim_elected(&dir).unwrap();
    assert!(
        said.iter()
            .any(|l| l.contains("governador-mg") && l.contains("eleito registrado")),
        "{said:?}"
    );
    let id = start_runoff(&dir, "governador-sp", now()).unwrap();
    admin.publish().unwrap();
    wait("second round open", 40, || {
        admin
            .tally()
            .is_ok_and(|t| t.contest(&id).is_some_and(|c| c.phase == Phase::VotingOpen))
    });

    for n in nodes {
        n.stop();
    }
    let _ = std::fs::remove_dir_all(&dir);
}
