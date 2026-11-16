//! ROTATION of the institutions (ADR-0045) on REAL nodes: one institution is replaced by a new one
//! while the chain keeps deciding blocks; a rotation is refused while an election is in progress;
//! the next key ceremony and a whole election run with the NEW set; the economy accepts the result
//! signed by it.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::TcpListener;
use std::path::Path;
use std::time::{Duration, Instant};

use election_admin::Admin;
use election_admin::elected::{build_election, claim_elected, outcomes};
use issuer_cli::{Issuer, scratch_dir};
use teko_chain::NetworkKind;
use teko_election::{ElectionManifest, Phase};
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
fn an_institution_is_replaced_while_the_chain_runs_and_the_next_election_is_decided_by_the_new_set()
{
    use teko_node::rotation;
    let issuer = Issuer::open(scratch_dir("rotation"));
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
    let genesis_set = load_network(&net).unwrap().profile.validators;

    let mut nodes: Vec<Option<NodeHandle>> =
        (0..4).map(|i| Some(start(&dir, i, &ec, None))).collect();
    devnet_sim::seed_candidates(&net, "geral-2026").unwrap();
    let last = teko_node::NodeWorld::open(&net)
        .unwrap()
        .chain()
        .last_timestamp();
    wait("the clock passes the contest window", 120, || {
        now() > last + 3
    });
    // some blocks of the vote chain before the rotation
    devnet_sim::blocks(&net, devnet_sim::BlocksKind::Votes, 3).unwrap();

    // 1) rotate: institution 3 is replaced by a brand-new one (no election yet, so it is allowed)
    let port = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let r = rotation::dev_rotate(&net, 3, port).unwrap();
    assert_eq!(r.change.epoch, 1);
    assert_eq!(r.new_index, 4);
    nodes.push(Some(start(&dir, 4, &ec, None)));
    // the chain keeps deciding blocks through the change and after it
    let client = teko_node::NodeClient::new(load_network(&net).unwrap().peers);
    let target = r.change.effective_height + 3;
    wait("the chain passes the effective height", 120, || {
        let _ = devnet_sim::blocks(&net, devnet_sim::BlocksKind::Votes, 1);
        client
            .status(client.addrs()[0])
            .is_ok_and(|s| s.height >= target)
    });
    // a replica that trusts no node follows the epochs by itself
    let profile = load_network(&net).unwrap().profile;
    let chain = client.sync_chain(&profile).unwrap();
    assert_eq!(chain.epochs().len(), 2);
    let now_set = chain.epoch_at(chain.height() + 1).set.clone();
    assert_eq!(now_set[..3], genesis_set[..3], "the same order");
    assert_eq!(now_set[3], r.new_key, "the newcomer takes the slot");
    assert_ne!(now_set[3], genesis_set[3]);
    assert_eq!(chain.changes_after(0).len(), 1);
    // the outgoing institution goes away: 3 of 4 (2 old + ... wait: 0,1,2 + the newcomer) still decide
    nodes[3].take().unwrap().stop();
    devnet_sim::blocks(&net, devnet_sim::BlocksKind::Votes, 3).unwrap();

    // 2) the key ceremony is among the institutions IN FORCE (0, 1, 2 and the newcomer), with an
    //    epoch-bound session
    let plan = rotation::ceremony_plan(&net).unwrap();
    assert_eq!(plan.epoch, 1);
    assert!(plan.session.contains("|e1|"), "{}", plan.session);
    assert_eq!(plan.participants.len(), 4);
    let rec = teko_node::run_ceremony_among(
        &client,
        &plan.participants,
        &net,
        plan.session.as_bytes(),
        plan.threshold,
    )
    .unwrap();
    assert_eq!(rec.n, 4);

    // 3) an election with the new ceremony's key, built from the chain's candidates
    build_election(&dir, "geral-2026", 30 * 86_400, now()).unwrap();
    for n in nodes.iter_mut().filter_map(Option::take) {
        n.stop();
    }
    let bytes = std::fs::read(dir.join("election.bundle")).unwrap();
    let b = <teko_election::ElectionBundle as prost::Message>::decode(bytes.as_slice()).unwrap();
    let m = <ElectionManifest as prost::Message>::decode(b.manifest.as_slice()).unwrap();
    let trust = teko_attestation::TrustPolicy::parse(
        &std::fs::read_to_string(dir.join("trust.txt")).unwrap(),
    )
    .unwrap();
    // the nodes of the CURRENT set restart (they replay their blocks, epochs included)
    let mut nodes: Vec<Option<NodeHandle>> = [0usize, 1, 2, 4]
        .iter()
        .map(|i| Some(start(&dir, *i, &ec, Some(trust.clone()))))
        .collect();
    let admin = Admin::open(&dir).unwrap();
    admin.publish().unwrap();
    wait("election published", 60, || {
        admin.tally().is_ok_and(|t| {
            t.contest("governador-mg")
                .is_some_and(|c| c.phase == Phase::VotingOpen)
        })
    });
    // the institutions refuse to rotate while the election is open (the proposal is refused)
    let again = rotation::propose(&net, now_set.clone(), 6)
        .unwrap_err()
        .to_string();
    assert!(again.contains("ainda não terminou"), "{again}");
    let rep = admin.simulate_batch(40, 3).unwrap();
    wait("ballots counted", 120, || {
        admin
            .tally()
            .is_ok_and(|t| t.contests.iter().map(|c| c.accepted).sum::<u64>() == rep.ballots)
    });
    let closes = m
        .contest("governador-mg")
        .unwrap()
        .schedule
        .clone()
        .unwrap()
        .voting_closes;
    admin.close_due(closes + 1).unwrap();
    wait("totals revealed by the new set", 120, || {
        admin.tally().is_ok_and(|t| {
            rep.planted
                .iter()
                .all(|(id, _)| t.contest(id).is_some_and(|c| c.revealed))
        })
    });
    // the results are signed by the institutions IN FORCE (more than 2/3 of the NEW set)
    wait("certified outcomes", 180, || {
        outcomes(&admin).is_ok_and(|o| {
            rep.planted.iter().all(|(id, _)| {
                o.iter()
                    .any(|x| &x.outcome.contest_id == id && x.is_certified())
            })
        })
    });
    for c in outcomes(&admin).unwrap() {
        assert_eq!(c.institutions, 4);
        assert!(
            c.signers >= 3,
            "{}: {} signers",
            c.outcome.contest_id,
            c.signers
        );
    }
    // and the economy, which was rotated too, accepts them (the OLD set would not)
    let said = claim_elected(&dir).unwrap();
    assert!(
        said.iter().any(|l| l.contains("eleito registrado")),
        "{said:?}"
    );
    for n in nodes.iter_mut().filter_map(Option::take) {
        n.stop();
    }
    let _ = std::fs::remove_dir_all(&dir);
}
