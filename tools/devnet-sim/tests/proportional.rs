//! PROPORTIONAL chambers (ADR-0046) on REAL nodes: parties and candidacies come from the chain, the
//! votes are planted exactly, the institutions sign the allocation of the seats (a tie that matters is
//! drawn once for the whole contest, ADR-0043) and the economy issues one `eleito` per winner, with
//! its party.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::TcpListener;
use std::path::Path;
use std::time::{Duration, Instant};

use election_admin::Admin;
use election_admin::elected::{build_election, claim_elected, outcomes, verify_draw};
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
fn chambers_are_allocated_by_quotient_and_dhondt_certified_drawn_when_a_tie_matters_and_claimed() {
    let issuer = Issuer::open(scratch_dir("proportional"));
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
    let said = build_election(&dir, "geral-2026", 30 * 86_400, now()).unwrap();
    assert!(
        said.iter()
            .any(|l| l.contains("deputado-estadual-sp") && l.contains("4 partidos, 8 cadeiras")),
        "{said:?}"
    );
    for n in boot {
        n.stop();
    }
    let bytes = std::fs::read(dir.join("election.bundle")).unwrap();
    let b = <teko_election::ElectionBundle as prost::Message>::decode(bytes.as_slice()).unwrap();
    let m = <ElectionManifest as prost::Message>::decode(b.manifest.as_slice()).unwrap();
    teko_election::validate_manifest(&m).unwrap();
    // the device refuses a manifest over 32 KiB: the example election with its chambers must fit
    assert!(b.manifest.len() <= 32 * 1024, "{} bytes", b.manifest.len());
    // the chain's candidacies became options: candidates, then the parties, then blank and null
    let state = m.contest("deputado-estadual-sp").unwrap();
    assert_eq!(state.options.len(), 16);
    assert_eq!(state.proportional.as_ref().unwrap().seats, 8);
    assert!(state.options[10].is_party() && state.options[10].number == "11");
    let trust = teko_attestation::TrustPolicy::parse(
        &std::fs::read_to_string(dir.join("trust.txt")).unwrap(),
    )
    .unwrap();
    let nodes: Vec<NodeHandle> = (0..4)
        .map(|i| start(&dir, i, &ec, Some(trust.clone())))
        .collect();
    let admin = Admin::open(&dir).unwrap();
    admin.publish().unwrap();
    wait("election published", 40, || {
        admin.tally().is_ok_and(|t| {
            t.contest("deputado-estadual-sp")
                .is_some_and(|c| c.phase == Phase::VotingOpen)
        })
    });

    // FEDERAL (6 seats), no tie: quotient 17; verde 53 -> 3 seats, rio 33 -> 1, serra 14 -> 0, then the
    // leftovers D'Hondt: rio 33/2 beats serra 14, then serra (rio has no apt candidate left)
    // options: 1101 1102 1103 2201 2202 2203 3301 3302 | P11 P22 P33 | blank null
    let federal = [30u64, 12, 6, 20, 8, 2, 9, 3, 5, 3, 2, 4, 1];
    // STATE (8 seats): quotient 12; the 8th seat is a tie between two parties' averages (10 = 10) and
    // three candidates of one party tie at the cut: a tie that MATTERS, so it is drawn
    // options: 11011 11012 11013 22021 22022 22023 33031 33032 44041 44042 | P11 P22 P33 P44 | blank null
    let tied = [10u64, 10, 10, 12, 10, 8, 14, 10, 8, 4];
    let mut planted = 0;
    planted += admin
        .simulate_exact("deputado-federal-sp", &federal, 5)
        .unwrap()
        .ballots;
    planted += admin
        .simulate_exact("deputado-estadual-sp", &tied, 6)
        .unwrap()
        .ballots;
    wait("ballots counted", 120, || {
        admin
            .tally()
            .is_ok_and(|t| t.contests.iter().map(|c| c.accepted).sum::<u64>() == planted)
    });
    let closes = m
        .contest("deputado-estadual-sp")
        .unwrap()
        .schedule
        .clone()
        .unwrap()
        .voting_closes;
    admin.close_due(closes + 1).unwrap();
    let ids = ["deputado-federal-sp", "deputado-estadual-sp"];
    wait("totals revealed", 120, || {
        admin.tally().is_ok_and(|t| {
            ids.iter()
                .all(|id| t.contest(id).is_some_and(|c| c.revealed))
        })
    });
    // after the reveal the tally shows the parties (label + candidates)
    let live = admin.tally().unwrap();
    let parties = live.contest("deputado-federal-sp").unwrap().party_totals();
    assert_eq!(
        parties
            .iter()
            .map(|p| (p.number.as_str(), p.votes))
            .collect::<Vec<_>>(),
        vec![("11", 53), ("22", 33), ("33", 14)]
    );
    wait("outcomes certified", 180, || {
        outcomes(&admin).is_ok_and(|o| {
            ids.iter().all(|id| {
                o.iter().any(|x| {
                    &x.outcome.contest_id == id
                        && x.is_certified()
                        && (x.outcome.drawn || *id == "deputado-federal-sp")
                })
            })
        })
    });
    let list = outcomes(&admin).unwrap();
    let of = |id: &str| {
        list.iter()
            .find(|x| x.outcome.contest_id == id && x.is_certified())
            .unwrap_or_else(|| panic!("no certified outcome for {id}"))
    };

    // federal: no draw, the exact allocation
    let f = of("deputado-federal-sp");
    assert_eq!(f.outcome.kind(), OutcomeKind::Elected);
    assert!(!f.outcome.drawn);
    assert_eq!((f.outcome.seats, f.outcome.quotient), (6, 17));
    assert_eq!(f.outcome.elected, vec![0, 1, 2, 3, 4, 6]);
    let seats: Vec<(String, u32)> = f
        .outcome
        .parties
        .iter()
        .map(|p| (p.number.clone(), p.seats))
        .collect();
    assert_eq!(
        seats,
        vec![
            ("11".to_string(), 3),
            ("22".to_string(), 2),
            ("33".to_string(), 1)
        ]
    );
    assert_eq!((f.outcome.blank_votes, f.outcome.null_votes), (4, 1));

    // state: drawn once for the whole contest; whatever the draw says, the chamber is full and the
    // rule holds: serra 2, cerrado 1, and the 8th seat went to mata or rio
    let s = of("deputado-estadual-sp");
    assert!(s.outcome.drawn && !s.outcome.draw_seed.is_empty());
    assert_eq!((s.outcome.seats, s.outcome.quotient), (8, 12));
    assert_eq!(s.outcome.elected.len(), 8);
    let by = |n: &str| {
        s.outcome
            .parties
            .iter()
            .find(|p| p.number == n)
            .unwrap()
            .seats
    };
    assert_eq!((by("33"), by("44")), (2, 1));
    assert_eq!(by("11") + by("22"), 5);
    assert!(matches!((by("11"), by("22")), (3, 2) | (2, 3)));
    assert!(s.signers >= 3, "{} signers", s.signers);
    // ANYONE redoes the draw from the board and gets the very same bytes
    let why = verify_draw(&admin, &s.outcome).unwrap();
    assert!(why.contains("sorteio refeito"), "{why}");

    // the economy issues one `eleito` per winner, with its party
    let said = claim_elected(&dir).unwrap();
    assert!(
        said.iter()
            .any(|l| l.contains("deputado-federal-sp") && l.contains("eleito")),
        "{said:?}"
    );
    let world = teko_node::NodeWorld::open(&net).unwrap();
    let got = teko_credentials::credentials_of_type(world.ledger_ref(), b"eleito");
    let claims_of = |pleito: &str| -> Vec<String> {
        got.iter()
            .map(|(_, r)| String::from_utf8_lossy(&r.claims).to_string())
            .filter(|c| c.contains(&format!("pleito={pleito};")))
            .collect()
    };
    let fed = claims_of("deputado-federal-sp");
    assert_eq!(fed.len(), 6);
    assert_eq!(fed.iter().filter(|c| c.contains("partido=11;")).count(), 3);
    assert!(
        fed.iter()
            .all(|c| c.starts_with("cargo=deputado-federal;uf=SP;partido="))
    );
    assert_eq!(claims_of("deputado-estadual-sp").len(), 8);
    drop(world);

    for n in nodes {
        n.stop();
    }
    let _ = std::fs::remove_dir_all(&dir);
}
