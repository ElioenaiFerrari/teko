//! The election on the federated test network: publish, register, vote and tally
//! through blocks signed by 2 of 3 validators, with the replica verified here.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::TcpListener;
use std::time::{Duration, Instant};

use election_admin::Admin;
use issuer_cli::{Issuer, scratch_dir};
use prost::Message;
use teko_chain::NetworkKind;
use teko_crypto::{public_key_bytes, voter::derive_voter_key};
use teko_election::{
    BallotBody, ElectionBundle, ElectionManifest, Phase, SubmittedBallot, SubmittedRegistration,
    contest_hash, registration_message, sign_ballot,
};
use teko_node::{
    Node, NodeConfig, NodeHandle, init_network, load_network, load_validator_key, node_dir,
};
use teko_registration::{BlindSignature, blind, finalize};

fn ports() -> u16 {
    loop {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let p = l.local_addr().unwrap().port();
        if p < 60_000 && (1..3).all(|k| TcpListener::bind(("127.0.0.1", p + k)).is_ok()) {
            return p;
        }
    }
}

fn start(dir: &std::path::Path, i: usize) -> NodeHandle {
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
            trust: None,
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
        std::thread::sleep(Duration::from_millis(80));
    }
    panic!("timed out waiting for: {what}");
}

fn manifest(admin: &Admin) -> ElectionManifest {
    let bytes = std::fs::read(admin.dir().join("election.bundle")).unwrap();
    let b = ElectionBundle::decode(bytes.as_slice()).unwrap();
    ElectionManifest::decode(b.manifest.as_slice()).unwrap()
}

#[test]
fn an_election_runs_on_the_network_and_the_tally_is_rebuilt_from_verified_blocks() {
    let issuer = Issuer::open(scratch_dir("net"));
    issuer.dev_setup().unwrap(); // also creates the demo election (unbound)
    let dir = issuer.dir().to_path_buf();
    init_network(&dir.join("net"), NetworkKind::Devnet, 3, ports(), false).unwrap();
    let nodes: Vec<_> = (0..3).map(|i| start(&dir, i)).collect();

    // Binding the election to the network also pins the device trust file to it.
    issuer
        .dev_election(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        )
        .unwrap();
    let admin = Admin::open(&dir).unwrap();
    assert!(admin.network().is_some(), "a running network is detected");
    let m = manifest(&admin);
    assert_eq!(m.chain_id, "teko-devnet-1");
    assert!(
        std::fs::read_to_string(dir.join("trust.txt"))
            .unwrap()
            .contains("chain_id=teko-devnet-1")
    );

    admin.publish().unwrap();
    wait("election visible in the verified replica", 30, || {
        admin.tally().is_ok_and(|t| {
            t.contest("presidente")
                .is_some_and(|c| c.phase == Phase::VotingOpen)
        })
    });
    assert_eq!(
        admin.publish().unwrap().appended,
        0,
        "publishing twice adds nothing"
    );

    // A registered voter votes; each submit returns where the entry really landed.
    let key = derive_voter_key(&[1; 32], &m.election_id, "presidente").unwrap();
    let voter_key = public_key_bytes(&key);
    let spki = m.contest("presidente").unwrap().registration_key.clone();
    let msg = registration_message(&m.election_id, "presidente", &voter_key);
    let b = blind(&spki, &msg).unwrap();
    let sig = issuer
        .registration_key("presidente")
        .unwrap()
        .blind_sign(&b.blind_message)
        .unwrap();
    let token = finalize(&spki, &b.state, &BlindSignature(sig.0), &msg).unwrap();
    let r1 = admin
        .submit_registration(&SubmittedRegistration {
            election_id: m.election_id.clone(),
            contest_id: "presidente".into(),
            voter_key: voter_key.clone(),
            token: token.to_bytes(),
        })
        .unwrap();
    let body = BallotBody {
        election_id: m.election_id.clone(),
        contest_id: "presidente".into(),
        contest_hash: contest_hash(m.contest("presidente").unwrap()).to_vec(),
        option_index: 1,
        ciphertext_hash: Vec::new(),
    };
    let r2 = admin
        .submit_ballot(&SubmittedBallot {
            election_id: body.election_id.clone(),
            contest_id: body.contest_id.clone(),
            contest_hash: body.contest_hash.clone(),
            option_index: 1,
            voter_key,
            signature: sign_ballot(&key, &body),
            encrypted: Vec::new(),
        })
        .unwrap();
    assert!(
        r2.index > r1.index,
        "the ballot sits after its registration in the log"
    );

    let t = admin.tally().unwrap();
    let p = t.contest("presidente").unwrap();
    assert_eq!((p.accepted, p.registered, p.rejected.total()), (1, 1, 0));
    assert_eq!(t.ignored_governance, 0);

    // The simulator also writes through the network; its voters are counted too.
    admin.simulate_batch(30, 5).unwrap();
    wait("simulated ballots committed", 40, || {
        admin
            .tally()
            .is_ok_and(|t| t.contest("presidente").is_some_and(|c| c.accepted == 31))
    });

    // Stopping every node falls back to the local file (no silent half-state).
    for n in nodes {
        n.stop();
    }
    assert!(Admin::open(&dir).unwrap().network().is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

fn start_with_trust(dir: &std::path::Path, i: usize) -> NodeHandle {
    let net = load_network(&dir.join("net")).unwrap();
    let addr = net.peers[i];
    let trust = teko_attestation::TrustPolicy::parse(
        &std::fs::read_to_string(dir.join("trust.txt")).unwrap(),
    )
    .unwrap();
    Node::start(
        NodeConfig {
            dir: node_dir(&dir.join("net"), i),
            profile: net.profile.with_block_interval_ms(150).unwrap(),
            key: load_validator_key(&dir.join("net"), i).unwrap(),
            index: i,
            peers: net.peers,
            allow_remote: false,
            trust: Some(trust),
        },
        addr,
    )
    .unwrap()
}

#[test]
fn encrypted_ballots_stay_hidden_until_the_close_then_two_of_three_validators_reveal_the_exact_totals()
 {
    let issuer = Issuer::open(scratch_dir("enc"));
    issuer.dev_setup().unwrap();
    let dir = issuer.dir().to_path_buf();
    init_network(&dir.join("net"), NetworkKind::Devnet, 3, ports(), false).unwrap();

    // 1) The validators make the election key together (nobody holds it whole).
    let boot: Vec<_> = (0..3).map(|i| start(&dir, i)).collect();
    let net = load_network(&dir.join("net")).unwrap();
    let rec = teko_node::run_ceremony(
        &teko_node::NodeClient::new(net.peers.clone()),
        &dir.join("net"),
        b"teko-devnet-1|tally-key",
        2,
    )
    .unwrap();
    for n in boot {
        n.stop();
    }

    // 2) The election carries that key and is bound to the network. Nodes restart
    //    (replaying their blocks and reloading their key shares) WITH the trust file.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    issuer.dev_election(now).unwrap();
    let mut nodes: Vec<Option<NodeHandle>> =
        (0..3).map(|i| Some(start_with_trust(&dir, i))).collect();
    let admin = Admin::open(&dir).unwrap();
    assert!(admin.network().is_some());
    let m = manifest(&admin);
    assert_eq!(
        m.tally_key, rec.group_key,
        "the election carries the ceremony's key"
    );
    admin.publish().unwrap();
    wait("election published", 30, || {
        admin.tally().is_ok_and(|t| {
            t.contest("presidente")
                .is_some_and(|c| c.phase == Phase::VotingOpen)
        })
    });

    // 3) Encrypted ballots from synthetic voters (real proofs, real registrations).
    let report = admin.simulate_batch(25, 11).unwrap();
    assert!(report.ballots >= 25);
    let planted: std::collections::BTreeMap<String, Vec<u64>> =
        report.planted.iter().cloned().collect();
    let total_planted: u64 = planted.values().map(|v| v.iter().sum::<u64>()).sum();
    assert_eq!(total_planted, report.ballots);
    wait("ballots counted", 60, || {
        admin
            .tally()
            .is_ok_and(|t| t.contests.iter().map(|c| c.accepted).sum::<u64>() == report.ballots)
    });
    // While voting is open: the ballot COUNT is known, the distribution does not exist.
    let t = admin.tally().unwrap();
    let pres = t.contest("presidente").unwrap();
    assert!(pres.encrypted && !pres.revealed);
    assert!(
        pres.options.iter().all(|o| o.votes == 0),
        "no per-option count while open"
    );
    assert!(
        !pres
            .public_view(teko_election::ResultsVisibility::Live)
            .distribution
    );
    assert_eq!(t.bad_results, 0);

    // 4) One validator goes down; the contests close; the other two decrypt.
    nodes[2].take().unwrap().stop();
    let closes = m
        .contest("presidente")
        .unwrap()
        .schedule
        .clone()
        .unwrap()
        .voting_closes;
    assert_eq!(admin.close_due(closes + 1).unwrap(), 6);
    wait("totals revealed by 2 of 3 validators", 90, || {
        admin.tally().is_ok_and(|t| {
            planted
                .keys()
                .all(|id| t.contest(id).is_some_and(|c| c.revealed))
        })
    });
    let t = admin.tally().unwrap();
    for (id, want) in &planted {
        let c = t.contest(id).unwrap();
        let got: Vec<u64> = c.options.iter().map(|o| o.votes).collect();
        assert_eq!(&got, want, "{id}: decrypted totals equal what was planted");
        assert!(
            c.public_view(teko_election::ResultsVisibility::AfterClose)
                .distribution
        );
    }
    assert_eq!(t.bad_results, 0);
    assert_eq!(t.ignored_governance, 0);
    for n in nodes.into_iter().flatten() {
        n.stop();
    }
    let _ = std::fs::remove_dir_all(&dir);
}
