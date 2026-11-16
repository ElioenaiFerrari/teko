//! The whole election of ADR-0040 on REAL nodes: candidates are `candidato` credentials issued by
//! the organ, the election is built from them, voters vote encrypted ballots, the contests close,
//! the institutions sign the results, and the economy chain issues the lifetime `eleito`
//! credentials (a runoff is opened when nobody has an absolute majority).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::TcpListener;
use std::path::Path;
use std::time::{Duration, Instant};

use election_admin::Admin;
use election_admin::elected::{build_election, claim_elected, outcomes, start_runoff};
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
fn candidates_on_the_chain_become_an_election_whose_certified_results_elect_people_with_lifetime_credentials()
 {
    let issuer = Issuer::open(scratch_dir("elected"));
    issuer.dev_setup().unwrap();
    let dir = issuer.dir().to_path_buf();
    let net = dir.join("net");
    init_network(&net, NetworkKind::Devnet, 4, ports(), false).unwrap();
    let mut g = init_economy_with_issuer(&net, None, None, None, None, false).unwrap();
    for t in &mut g.credtypes {
        if t.name == "candidato" {
            t.contest_secs = 2; // short: the candidacies become active within the test
        }
    }
    std::fs::write(net.join(ECONOMY_FILE), g.to_text()).unwrap();
    let ec = load_economy(&net).unwrap().unwrap();
    assert_eq!(ec.genesis.vote_validators.len(), 4);

    // 1) the validators make the election key; the organ issues the candidacies on the economy chain
    let boot: Vec<NodeHandle> = (0..4).map(|i| start(&dir, i, &ec, None)).collect();
    let netcfg = load_network(&net).unwrap();
    teko_node::run_ceremony(
        &teko_node::NodeClient::new(netcfg.peers.clone()),
        &net,
        b"teko-devnet-1|tally-key",
        2,
    )
    .unwrap();
    let said = devnet_sim::seed_candidates(&net, "geral-2026").unwrap();
    assert!(said.contains("candidaturas"), "{said}");
    // past the (2 s) contest window of the LAST approval: block time moves one second per block, so a
    // burst of transactions runs a little ahead of the wall clock
    let last = teko_node::NodeWorld::open(&net)
        .unwrap()
        .chain()
        .last_timestamp();
    wait("the clock passes the contest window", 120, || {
        now() > last + 3
    });

    // 2) the election is BUILT FROM THE CHAIN: every option binds its candidacy credential
    let report = build_election(&dir, "geral-2026", 30 * 86_400, now()).unwrap();
    assert!(
        report.iter().all(|l| l.contains("candidatos da chain")),
        "{report:?}"
    );
    for n in boot {
        n.stop();
    }
    let bytes = std::fs::read(dir.join("election.bundle")).unwrap();
    let b = <teko_election::ElectionBundle as prost::Message>::decode(bytes.as_slice()).unwrap();
    let m = <ElectionManifest as prost::Message>::decode(b.manifest.as_slice()).unwrap();
    assert_eq!(m.election_id, "geral-2026");
    for c in &m.contests {
        assert!(c.runoff, "{}", c.contest_id);
        assert!(
            c.options.iter().all(|o| o.credential_id.len() == 32),
            "{}",
            c.contest_id
        );
    }

    // 3) the nodes restart with the trust file (they replay their blocks, reload the key shares)
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
            t.contest("presidente")
                .is_some_and(|c| c.phase == Phase::VotingOpen)
        })
    });

    // 4) encrypted ballots, then the contests close and the validators decrypt
    let rep = admin.simulate_batch(60, 21).unwrap();
    wait("ballots counted", 120, || {
        admin
            .tally()
            .is_ok_and(|t| t.contests.iter().map(|c| c.accepted).sum::<u64>() == rep.ballots)
    });
    let closes = m
        .contest("presidente")
        .unwrap()
        .schedule
        .clone()
        .unwrap()
        .voting_closes;
    admin.close_due(closes + 1).unwrap();
    wait("totals revealed", 120, || {
        admin.tally().is_ok_and(|t| {
            rep.planted
                .iter()
                .all(|(id, _)| t.contest(id).is_some_and(|c| c.revealed))
        })
    });

    // 5) every institution signs the result of every single-seat contest; the signatures are on the board
    let diag = || {
        outcomes(&admin)
            .map(|o| {
                o.iter()
                    .map(|c| {
                        format!(
                            "{}:{:?}:{}/{}",
                            c.outcome.contest_id,
                            c.outcome.kind(),
                            c.signers,
                            c.institutions
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    let _ = &diag;
    wait("certified outcomes on the board", 120, || {
        outcomes(&admin).is_ok_and(|o| {
            // the contests that were voted (the mayor's is still only announced)
            rep.planted.iter().all(|(id, _)| {
                o.iter()
                    .any(|x| &x.outcome.contest_id == id && x.is_certified())
            })
        })
    });
    let list = outcomes(&admin).unwrap();
    for c in &list {
        // the institutions decided the SAME bytes and they match the planted totals
        let planted = rep
            .planted
            .iter()
            .find(|(id, _)| *id == c.outcome.contest_id)
            .map(|(_, v)| v.clone())
            .unwrap();
        assert_eq!(c.outcome.totals, planted, "{}", c.outcome.contest_id);
        assert_eq!(c.institutions, 4);
        assert!(
            c.signers >= 3,
            "{}: {} signers",
            c.outcome.contest_id,
            c.signers
        );
    }
    let elected: Vec<_> = list
        .iter()
        .filter(|c| c.outcome.kind() == OutcomeKind::Elected)
        .collect();
    assert!(
        !elected.is_empty(),
        "with 2-candidate contests somebody wins: {list:?}"
    );

    // 6) the economy chain issues the lifetime `eleito` credentials
    let lines = claim_elected(&dir).unwrap();
    assert!(
        lines.iter().any(|l| l.contains("eleito registrado")),
        "{lines:?}"
    );
    let world = teko_node::NodeWorld::open(&net).unwrap();
    let got = teko_credentials::credentials_of_type(world.ledger_ref(), b"eleito");
    assert_eq!(got.len(), elected.len(), "{lines:?}");
    for c in &elected {
        let contest = &c.outcome.contest_id;
        let rec = got
            .iter()
            .map(|(_, r)| r)
            .find(|r| String::from_utf8_lossy(&r.claims).contains(&format!("pleito={contest};")))
            .unwrap_or_else(|| panic!("no eleito for {contest}"));
        assert_eq!(rec.valid_until, 0, "lifetime");
        assert_eq!(
            rec.status_at(now() + 1_000_000_000),
            teko_credentials::Status::Active
        );
        // the holder is the winner's candidacy key
        let cand = teko_credentials::credential(
            world.ledger_ref(),
            b"candidato",
            &c.outcome.winner_credential_id,
        )
        .unwrap();
        assert_eq!(rec.subject, cand.subject);
    }
    drop(world);
    // claiming again changes nothing
    let again = claim_elected(&dir).unwrap();
    assert!(again.iter().all(|l| l.contains("já estava")), "{again:?}");

    // 7) a contest decided by a runoff opens its second round (when the random votes produced one)
    if let Some(r) = list
        .iter()
        .find(|c| c.outcome.kind() == OutcomeKind::Runoff)
    {
        let id = start_runoff(&dir, &r.outcome.contest_id, now()).unwrap();
        admin.publish().unwrap();
        wait("second round open", 40, || {
            admin
                .tally()
                .is_ok_and(|t| t.contest(&id).is_some_and(|c| c.phase == Phase::VotingOpen))
        });
        let t = admin.tally().unwrap();
        assert_eq!(t.contest(&id).unwrap().options.len(), 2);
    }

    for n in nodes {
        n.stop();
    }
    let _ = std::fs::remove_dir_all(&dir);
}
