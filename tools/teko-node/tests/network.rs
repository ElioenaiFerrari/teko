//! Institutions on loopback deciding the vote chain by BFT (ADR-0034): finality, convergence,
//! censorship bypassed, a pause (never a fork) without a quorum, catch-up after a restart, a
//! forged certificate refused, and no block made on its own.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use rand::SeedableRng;

use prost::Message;
use teko_bft::{Commit, Vote, VoteKind, block_id};
use teko_bulletin::{BulletinBoard, Entry, EntryKind};
use teko_chain::{Chain, DecidedBlock, NetworkKind};
use teko_node::wire;
use teko_node::{
    Node, NodeClient, NodeConfig, NodeHandle, init_network, load_network, load_validator_key,
    node_dir,
};

fn free_port_base() -> u16 {
    // Tests run in parallel: hand out disjoint blocks of 4 ports, skipping any in use.
    use std::sync::atomic::{AtomicU16, Ordering};
    static NEXT: AtomicU16 = AtomicU16::new(0);
    loop {
        let off = NEXT.fetch_add(4, Ordering::SeqCst);
        let base = 30_000 + (std::process::id() % 2_000) as u16 * 10 + off;
        if (0..4).all(|k| TcpListener::bind(("127.0.0.1", base + k)).is_ok()) {
            return base;
        }
    }
}

struct Net {
    dir: PathBuf,
    nodes: Vec<Option<NodeHandle>>,
}

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "teko-net-{tag}-{}-{}",
        std::process::id(),
        free_port_base()
    ));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn start_node(dir: &std::path::Path, i: usize) -> NodeHandle {
    let net = load_network(dir).unwrap();
    let profile = net.profile.with_block_interval_ms(150).unwrap();
    let addr = net.peers[i];
    Node::start(
        NodeConfig {
            dir: node_dir(dir, i),
            profile,
            key: load_validator_key(dir, i).unwrap(),
            index: i,
            peers: net.peers,
            allow_remote: false,
            trust: None,
            economy: None,
        },
        addr,
    )
    .unwrap()
}

fn boot(tag: &str) -> Net {
    boot_n(tag, 3)
}

fn boot_n(tag: &str, n: usize) -> Net {
    let d = dir(tag);
    init_network(&d, NetworkKind::Devnet, n, free_port_base(), false).unwrap();
    let nodes = (0..n).map(|i| Some(start_node(&d, i))).collect();
    Net { dir: d, nodes }
}

fn client(net: &Net) -> (NodeClient, teko_chain::NetworkProfile) {
    let n = load_network(&net.dir).unwrap();
    (
        NodeClient::new(n.peers),
        n.profile.with_block_interval_ms(150).unwrap(),
    )
}

fn wait(what: &str, secs: u64, mut f: impl FnMut() -> bool) {
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(secs) {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(60));
    }
    panic!("timed out waiting for: {what}");
}

fn entries(tag: u8, n: u8) -> Vec<Entry> {
    (0..n)
        .map(|i| Entry::new(EntryKind::BALLOT, vec![tag, i, 7, 7]))
        .collect()
}

fn included(chain: &Chain, es: &[Entry]) -> bool {
    es.iter()
        .all(|e| (0..chain.board().len()).any(|i| chain.board().entry(i) == Some(e)))
}

/// Precommits of a block's commit certificate and the round it was decided in.
fn certificate(d: &DecidedBlock) -> (usize, u32) {
    d.commit
        .as_ref()
        .map_or((0, 0), |c| (c.precommits.len(), c.round))
}

#[test]
fn three_nodes_decide_blocks_and_every_replica_verifies_to_the_same_log() {
    let net = boot("conv");
    let (c, profile) = client(&net);
    let es = entries(1, 5);
    assert_eq!(c.submit(&es).unwrap(), 5);
    assert_eq!(c.submit(&es).unwrap(), 0, "exact duplicates are dropped");
    wait("all nodes commit the entries", 20, || {
        c.statuses().iter().all(|(_, s)| {
            s.as_ref()
                .is_some_and(|s| s.log_size == 5 && s.mempool == 0)
        })
    });
    // Each node serves a chain that a client verifies on its own, and they agree.
    let roots: Vec<_> = net
        .nodes
        .iter()
        .enumerate()
        .map(|(i, _)| {
            let one = NodeClient::new(vec![load_network(&net.dir).unwrap().peers[i]]);
            let chain = one.sync_chain(&profile).unwrap();
            assert!(included(&chain, &es));
            // 3 institutions of equal power: a block is final with all 3 precommits (> 2/3)
            assert!(chain.blocks().iter().all(|b| certificate(b).0 == 3));
            (chain.height(), chain.board().root())
        })
        .collect();
    assert!(roots.iter().all(|r| *r == roots[0]), "{roots:?}");
    // The same entries submitted via ANOTHER node are also deduplicated network-wide.
    let other = NodeClient::new(vec![load_network(&net.dir).unwrap().peers[2]]);
    assert_eq!(other.submit(&es).unwrap(), 0);
    for h in net.nodes.into_iter().flatten() {
        h.stop();
    }
}

#[test]
fn no_block_is_made_on_its_own_the_chain_only_grows_with_entries() {
    let net = boot("idle");
    let (c, _) = client(&net);
    std::thread::sleep(Duration::from_secs(3));
    assert!(
        c.statuses()
            .iter()
            .all(|(_, s)| s.as_ref().is_some_and(|s| s.height == 0)),
        "an idle network made a block"
    );
    c.submit(&entries(5, 1)).unwrap();
    wait("one block everywhere", 20, || {
        c.statuses()
            .iter()
            .all(|(_, s)| s.as_ref().is_some_and(|s| s.height == 1))
    });
    std::thread::sleep(Duration::from_secs(3));
    assert!(
        c.statuses()
            .iter()
            .all(|(_, s)| s.as_ref().is_some_and(|s| s.height == 1)),
        "a block appeared with nothing to include"
    );
    for h in net.nodes.into_iter().flatten() {
        h.stop();
    }
}

#[test]
fn with_four_institutions_one_can_be_down_even_when_it_is_the_proposer() {
    let mut net = boot_n("down", 4);
    let (c, profile) = client(&net);
    net.nodes[3].take().unwrap().stop();
    // Keep going until a height whose proposer is the silent institution was decided by the
    // NEXT round (a silent or censoring proposer loses its turn instead of stopping the chain).
    let mut round = 0u8;
    let mut skipped = false;
    while !skipped && round < 40 {
        let es = entries(10 + round, 2);
        c.submit(&es).unwrap();
        wait("entries committed with 3 of 4", 40, || {
            c.sync_chain(&profile).is_ok_and(|ch| included(&ch, &es))
        });
        let chain = c.sync_chain(&profile).unwrap();
        skipped = chain.blocks().iter().any(|b| certificate(b).1 > 0);
        round += 1;
    }
    assert!(skipped, "the silent proposer's turn was never handed over");
    for h in net.nodes.into_iter().flatten() {
        h.stop();
    }
}

#[test]
fn without_a_quorum_the_chain_pauses_it_never_forks_and_resumes_when_the_quorum_returns() {
    // 3 institutions tolerate no failure: with one down there is no quorum (> 2/3)
    let mut net = boot("pause");
    let (c, profile) = client(&net);
    net.nodes[2].take().unwrap().stop();
    let es = entries(40, 2);
    c.submit(&es).unwrap();
    std::thread::sleep(Duration::from_secs(4));
    let statuses: Vec<_> = c.statuses().into_iter().filter_map(|(_, s)| s).collect();
    assert_eq!(statuses.len(), 2);
    assert!(
        statuses.iter().all(|s| s.height == 0 && s.mempool == 2),
        "a block was decided without a quorum: {statuses:?}"
    );
    // the quorum returns: the pending entries are decided, everywhere, on one chain
    net.nodes[2] = Some(start_node(&net.dir, 2));
    wait("decided once the third institution is back", 40, || {
        c.statuses()
            .iter()
            .all(|(_, s)| s.as_ref().is_some_and(|s| s.height == 1 && s.log_size == 2))
    });
    let chain = c.sync_chain(&profile).unwrap();
    assert!(included(&chain, &es));
    for h in net.nodes.into_iter().flatten() {
        h.stop();
    }
}

#[test]
fn a_restarted_node_replays_its_log_and_catches_up() {
    let mut net = boot_n("restart", 4);
    let (c, profile) = client(&net);
    c.submit(&entries(20, 3)).unwrap();
    wait("first commit", 20, || {
        c.sync_chain(&profile).is_ok_and(|ch| ch.board().len() == 3)
    });
    net.nodes[1].take().unwrap().stop();
    c.submit(&entries(21, 3)).unwrap();
    wait("commit while node 1 is down", 30, || {
        c.sync_chain(&profile).is_ok_and(|ch| ch.board().len() == 6)
    });
    net.nodes[1] = Some(start_node(&net.dir, 1));
    let me = NodeClient::new(vec![load_network(&net.dir).unwrap().peers[1]]);
    wait("node 1 catches up", 20, || {
        me.sync_chain(&profile)
            .is_ok_and(|ch| ch.board().len() == 6)
    });
    for h in net.nodes.into_iter().flatten() {
        h.stop();
    }
}

#[test]
fn a_block_without_a_quorum_certificate_is_not_accepted_by_a_node() {
    let net = boot_n("forged", 4);
    let (c, profile) = client(&net);
    c.submit(&entries(30, 2)).unwrap();
    wait("block 1 everywhere", 20, || {
        c.statuses()
            .iter()
            .all(|(_, s)| s.as_ref().is_some_and(|s| s.height >= 1))
    });
    // Two institutions (maliciously) certify a DIFFERENT next block: 2 of 4 is not a quorum.
    let chain = c.sync_chain(&profile).unwrap();
    let set = chain.validator_set().unwrap();
    let who = set.proposer(2, 0).key.clone();
    let proposer_index = (0..4)
        .find(|i| teko_crypto::public_key_bytes(&load_validator_key(&net.dir, *i).unwrap()) == who)
        .unwrap();
    let payload = chain.propose(&who, &entries(99, 2), 4_000_000_000);
    let id = block_id(&payload);
    let precommits = [proposer_index, (proposer_index + 1) % 4]
        .iter()
        .map(|i| {
            Vote::new(
                &profile.chain_id,
                &load_validator_key(&net.dir, *i).unwrap(),
                VoteKind::Precommit,
                2,
                0,
                Some(&id),
            )
        })
        .collect();
    let forged = DecidedBlock {
        payload,
        commit: Some(Commit {
            height: 2,
            round: 0,
            block_id: id,
            precommits,
        }),
    };
    let addr: SocketAddr = load_network(&net.dir).unwrap().peers[(proposer_index + 2) % 4];
    assert!(teko_node_call(addr, wire::msg(wire::COMMIT, &forged)));
    std::thread::sleep(Duration::from_millis(500));
    let s = c.status(addr).unwrap();
    assert_eq!(s.height, 1, "the node applied a block without a quorum");
    assert!(
        c.sync_chain(&profile)
            .unwrap()
            .blocks()
            .iter()
            .filter_map(|d| d.block().ok())
            .all(|b| b.entries.iter().all(|e| e.payload[0] == 30)),
        "the committed block did not change"
    );
    for h in net.nodes.into_iter().flatten() {
        h.stop();
    }
}

fn teko_node_call(addr: SocketAddr, m: wire::Msg) -> bool {
    let mut s = std::net::TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    wire::write_msg(&mut s, &m).unwrap();
    wire::read_msg(&mut s).is_ok()
}

#[test]
fn a_node_refuses_to_listen_outside_loopback_and_a_wrong_key() {
    let d = dir("guard");
    init_network(&d, NetworkKind::Devnet, 3, free_port_base(), false).unwrap();
    let net = load_network(&d).unwrap();
    let cfg = |index: usize, key_of: usize| NodeConfig {
        dir: node_dir(&d, index),
        profile: net.profile.clone(),
        key: load_validator_key(&d, key_of).unwrap(),
        index,
        peers: net.peers.clone(),
        allow_remote: false,
        trust: None,
        economy: None,
    };
    assert!(Node::start(cfg(0, 0), "0.0.0.0:0".parse().unwrap()).is_err());
    assert!(
        Node::start(cfg(0, 1), "127.0.0.1:0".parse().unwrap()).is_err(),
        "key of validator 1 as index 0"
    );
    assert!(init_network(&d, NetworkKind::Mainnet, 3, 9000, true).is_err());
}

#[test]
fn the_key_ceremony_runs_across_the_nodes_and_leaves_a_verifiable_record_on_the_chain() {
    use teko_ballot_crypto::dkg_wire::DkgRecord;
    use teko_ballot_crypto::{
        Point, combine_shares, decrypt_share, encrypt_bit, lagrange_at_zero, point_to_bytes,
        prove_decryption, solve_small_dlog, verify_decryption,
    };
    use teko_node::{load_tally_share, run_ceremony};

    let net = boot("dkg");
    let (c, profile) = client(&net);
    let rec = run_ceremony(&c, &net.dir, b"teko-devnet-1|tally-key", 2).unwrap();
    assert_eq!((rec.n, rec.threshold), (3, 2));

    // Anyone can re-verify the ceremony from the record alone.
    let v = rec.verify().unwrap();
    assert_eq!(point_to_bytes(&v.group_key).to_vec(), rec.group_key);
    assert_eq!(
        std::fs::read_to_string(net.dir.join("tally-key.pub"))
            .unwrap()
            .trim(),
        teko_crypto::hex::encode(&rec.group_key)
    );

    // The record is on the chain (verified replica), as a DKG entry.
    wait("the DKG record is in a block", 20, || {
        c.sync_chain(&profile).is_ok_and(|ch| {
            (0..ch.board().len()).any(|i| {
                ch.board().entry(i).is_some_and(|e| {
                    e.kind == teko_bulletin::EntryKind::DKG
                        && DkgRecord::decode(e.payload.as_slice()).is_ok_and(|r| r == rec)
                })
            })
        })
    });

    // Each node saved its OWN share; the shares agree on the key and decrypt with proofs.
    let shares: Vec<_> = (0..3)
        .map(|i| load_tally_share(&node_dir(&net.dir, i)).unwrap())
        .collect();
    assert!(
        shares
            .iter()
            .all(|s| point_to_bytes(&s.group_public_key).to_vec() == rec.group_key)
    );
    for s in &shares {
        let vk = v
            .verification_keys
            .iter()
            .find(|(j, _)| *j == s.index)
            .unwrap()
            .1;
        assert_eq!(
            vk,
            Point::GENERATOR * s.secret,
            "verification key matches the share"
        );
    }
    let mut rng = rand::rngs::StdRng::seed_from_u64(1);
    let ct = [1u64, 0, 1, 1]
        .iter()
        .map(|m| encrypt_bit(&v.group_key, *m, &p256_random(&mut rng)))
        .fold(teko_ballot_crypto::Ciphertext::zero(), |a, b| a + b);
    // validators 1 and 3 (index 0 and 2) decrypt; validator 2 is not needed
    let mut parts = Vec::new();
    for i in [0usize, 2] {
        let s = &shares[i];
        let d = decrypt_share(&s.secret, &ct);
        let proof = prove_decryption(&s.secret, &ct, b"ctx", &mut rng);
        let vk = v
            .verification_keys
            .iter()
            .find(|(j, _)| *j == s.index)
            .unwrap()
            .1;
        assert!(verify_decryption(&vk, &ct, &d, &proof, b"ctx"));
        parts.push((s.index, d));
    }
    let _ = lagrange_at_zero(&[1, 3], 1).unwrap();
    assert_eq!(
        solve_small_dlog(&combine_shares(&ct, &parts).unwrap(), 10),
        Some(3)
    );

    // Running the ceremony again with wrong parameters is refused and changes nothing.
    assert!(run_ceremony(&c, &net.dir, b"x", 4).is_err());
    for h in net.nodes.into_iter().flatten() {
        h.stop();
    }
}

fn p256_random(rng: &mut rand::rngs::StdRng) -> teko_ballot_crypto::Scalar {
    use p256::elliptic_curve::Field;
    teko_ballot_crypto::Scalar::random(rng)
}
