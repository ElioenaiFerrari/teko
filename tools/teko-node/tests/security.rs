//! The network layer's defences (ADR-0048) on REAL nodes: the peer port talks only to a validator that
//! proves its key, the public port answers only what a client may ask, a key-ceremony share goes only to
//! its owner, floods and a misbehaving institution do not stop the chain, and a validator that moved is
//! found again by its signed record.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rand::SeedableRng;
use teko_bulletin::{Entry, EntryKind};
use teko_chain::NetworkKind;
use teko_crypto::{generate_signing_key, public_key_bytes};
use teko_node::wire::{self, Ack, Msg};
use teko_node::{
    Node, NodeClient, NodeConfig, NodeHandle, PeerClient, PeerLink, genesis_hash_of, init_network,
    load_network, load_validator_key, node_dir,
};
use teko_p2p::{NetIdentity, PeerPolicy, connect};

fn free_port_base() -> u16 {
    use std::sync::atomic::{AtomicU16, Ordering};
    static NEXT: AtomicU16 = AtomicU16::new(0);
    loop {
        let off = NEXT.fetch_add(16, Ordering::SeqCst);
        let base = 36_000 + (std::process::id() % 2_000) as u16 * 10 + off;
        if (0..16).all(|k| TcpListener::bind(("127.0.0.1", base + k)).is_ok()) {
            return base;
        }
    }
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

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "teko-sec-{tag}-{}-{}",
        std::process::id(),
        free_port_base()
    ));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn start_node(dir: &Path, i: usize, links: Option<Vec<PeerLink>>) -> NodeHandle {
    let net = load_network(dir).unwrap();
    let profile = net.profile.with_block_interval_ms(150).unwrap();
    let addr = net.peers[i];
    Node::start(
        NodeConfig {
            dir: node_dir(dir, i),
            profile,
            key: load_validator_key(dir, i).unwrap(),
            index: i,
            links: links.unwrap_or(net.links),
            listen_peer: None,
            advertise: None,
            allow_remote: false,
            trust: None,
            economy: None,
        },
        addr,
    )
    .unwrap()
}

fn four(tag: &str) -> (PathBuf, Vec<NodeHandle>) {
    let d = dir(tag);
    init_network(&d, NetworkKind::Devnet, 4, free_port_base(), false).unwrap();
    let nodes = (0..4).map(|i| start_node(&d, i, None)).collect();
    (d, nodes)
}

fn stop(nodes: Vec<NodeHandle>, dir: &Path) {
    for n in nodes {
        n.stop();
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// Accepts the one key we mean to reach.
struct Only(Vec<u8>);
impl PeerPolicy for Only {
    fn accepts(&self, k: &[u8]) -> bool {
        self.0 == k
    }
}

fn status_msg() -> Msg {
    Msg {
        kind: wire::STATUS,
        body: Vec::new(),
    }
}

fn is_nack(r: &Msg) -> bool {
    r.kind == wire::ACK && !wire::decode::<Ack>(r).unwrap().ok
}

#[test]
fn the_peer_port_talks_only_to_a_validator_that_proves_its_key() {
    let (d, nodes) = four("peerport");
    let net = load_network(&d).unwrap();
    let link0 = net.links[0].clone();
    let peer0: SocketAddr = link0.peer.parse().unwrap();

    // a validator is answered
    let me = PeerClient::open_as(&d, 1).unwrap();
    let r = me
        .call(&link0, &status_msg(), Duration::from_secs(5))
        .unwrap();
    assert_eq!(r.kind, wire::STATUS_REPLY);

    // plain bytes, as the old protocol or an attacker would send them, get nothing
    let mut raw = TcpStream::connect(peer0).unwrap();
    raw.set_read_timeout(Some(Duration::from_secs(8))).unwrap();
    wire::write_msg(&mut raw, &status_msg()).unwrap();
    let mut buf = [0u8; 64];
    assert!(
        matches!(raw.read(&mut buf), Ok(0) | Err(_)),
        "a plain request is never answered on the peer port"
    );

    // a stranger on the RIGHT network (same chain and genesis) but with a key outside the set
    let stranger = generate_signing_key(&mut rand::rngs::StdRng::seed_from_u64(4242));
    let id = NetIdentity::generate(
        stranger,
        &net.profile.chain_id,
        &genesis_hash_of(&d).unwrap(),
    )
    .unwrap();
    let stream = TcpStream::connect(peer0).unwrap();
    let mut sess = connect(stream, &id, &Only(link0.key.clone()), Some(&link0.key)).unwrap();
    let r = sess
        .send(wire::STATUS, &[], Duration::from_secs(3))
        .and_then(|()| sess.recv(&|_| 1 << 20, Duration::from_secs(3)));
    assert!(r.is_err(), "a key outside the validator set is cut off");

    // a validator of ANOTHER genesis is refused too
    let ok_key = load_validator_key(&d, 2).unwrap();
    let id = NetIdentity::generate(ok_key, &net.profile.chain_id, b"another genesis").unwrap();
    let stream = TcpStream::connect(peer0).unwrap();
    let r = connect(stream, &id, &Only(link0.key.clone()), Some(&link0.key)).and_then(|mut s| {
        s.send(wire::STATUS, &[], Duration::from_secs(3))?;
        s.recv(&|_| 1 << 20, Duration::from_secs(3))
    });
    assert!(r.is_err(), "another genesis never gets in");
    stop(nodes, &d);
}

#[test]
fn the_public_port_answers_only_what_a_client_may_ask() {
    let (d, nodes) = four("publicport");
    let public = load_network(&d).unwrap().peers[0];
    let ask = |m: &Msg| -> Msg {
        let mut s = TcpStream::connect(public).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        wire::write_msg(&mut s, m).unwrap();
        wire::read_msg(&mut s).unwrap()
    };
    assert_eq!(ask(&status_msg()).kind, wire::STATUS_REPLY);
    for kind in [
        wire::DKG_START,
        wire::DKG_SHARE,
        wire::DKG_PACKAGES,
        wire::VOTE_BFT,
        wire::FORWARD,
        wire::COMMIT,
        wire::ECON_BFT,
        wire::ECON_COMMIT,
        wire::ECON_TX_FWD,
        wire::PEERS,
        999,
    ] {
        let r = ask(&Msg {
            kind,
            body: vec![1, 2, 3],
        });
        assert!(
            is_nack(&r),
            "kind {kind} must not be served on the public port"
        );
    }
    // a lying length prefix costs nothing: the node drops it and keeps answering
    let mut s = TcpStream::connect(public).unwrap();
    s.write_all(&u32::MAX.to_be_bytes()).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut buf = [0u8; 8];
    assert!(matches!(s.read(&mut buf), Ok(0) | Err(_)));
    assert_eq!(ask(&status_msg()).kind, wire::STATUS_REPLY);
    stop(nodes, &d);
}

#[test]
fn a_key_ceremony_share_goes_only_to_the_institution_that_owns_its_index() {
    use teko_ballot_crypto::dkg_wire::Round1Msg;
    let (d, nodes) = four("dkgshare");
    let net = load_network(&d).unwrap();
    let keys: Vec<Vec<u8>> = net.links.iter().map(|l| l.key.clone()).collect();
    let t = Duration::from_secs(5);
    let coordinator = PeerClient::open_as(&d, 0).unwrap();
    // round 1 only: every node makes its polynomial for a ceremony with these participants
    for (pos, link) in net.links.iter().enumerate() {
        let r = coordinator
            .call(
                link,
                &wire::msg(
                    wire::DKG_START,
                    &wire::DkgStart {
                        session: b"s".to_vec(),
                        n: 4,
                        threshold: 3,
                        index: pos as u32 + 1,
                        participants: keys.clone(),
                    },
                ),
                t,
            )
            .unwrap();
        let rep: wire::DkgReply = wire::decode(&r).unwrap();
        assert!(rep.ok, "{}", rep.info);
        let _: Round1Msg = rep.round1.unwrap();
    }
    let ask = |as_node: usize, to: usize, requester: u32| -> wire::DkgReply {
        let me = PeerClient::open_as(&d, as_node).unwrap();
        let r = me
            .call(
                &net.links[to],
                &wire::msg(wire::DKG_SHARE, &wire::DkgShareReq { requester }),
                t,
            )
            .unwrap();
        wire::decode(&r).unwrap()
    };
    // institution 1 (index 2) gets ITS share from node 0, and nobody else gets it
    let mine = ask(1, 0, 2);
    assert!(mine.ok && !mine.share.is_empty());
    assert!(!ask(2, 0, 2).ok, "index 2 is not institution 2's");
    assert!(
        !ask(1, 0, 3).ok,
        "nor is it institution 1's to take index 3"
    );
    assert!(!ask(1, 0, 1).ok, "a node never hands out its own index");
    assert!(!ask(1, 0, 9).ok, "an index outside the ceremony");
    // a node refuses a ceremony that does not place it where the coordinator says
    let r = coordinator
        .call(
            &net.links[1],
            &wire::msg(
                wire::DKG_START,
                &wire::DkgStart {
                    session: b"s2".to_vec(),
                    n: 4,
                    threshold: 3,
                    index: 3,
                    participants: keys.clone(),
                },
            ),
            t,
        )
        .unwrap();
    assert!(!wire::decode::<wire::DkgReply>(&r).unwrap().ok);
    stop(nodes, &d);
}

#[test]
fn floods_and_a_misbehaving_institution_do_not_stop_the_chain() {
    let (d, nodes) = four("flood");
    let net = load_network(&d).unwrap();
    let client = NodeClient::new(net.peers.clone());
    let height = |c: &NodeClient| c.status(c.addrs()[0]).map_or(0, |s| s.height);
    let entry = |n: u8| Entry::new(EntryKind::MANIFEST, vec![n; 40]);
    client.submit(&[entry(1)]).unwrap();
    wait("the first block", 30, || height(&client) >= 1);

    // garbage on both ports from many connections at once
    let targets: Vec<SocketAddr> = vec![net.peers[0], net.links[0].peer.parse().unwrap()];
    let floods: Vec<_> = (0..16)
        .map(|n| {
            let targets = targets.clone();
            std::thread::spawn(move || {
                for k in 0..30u32 {
                    let Ok(mut s) = TcpStream::connect(targets[(n + k as usize) % 2]) else {
                        continue;
                    };
                    let junk: Vec<u8> = (0..(200 + k * 37))
                        .map(|i| (i * 7 + n as u32) as u8)
                        .collect();
                    let _ = s.write_all(&junk);
                    // some of them announce a huge message and stop
                    if k % 5 == 0 {
                        let _ = s.write_all(&u32::MAX.to_be_bytes());
                    }
                }
            })
        })
        .collect();
    for n in 2..6u8 {
        client.submit(&[entry(n)]).unwrap();
    }
    for f in floods {
        f.join().unwrap();
    }
    let before = height(&client);
    client.submit(&[entry(9)]).unwrap();
    wait("the chain keeps deciding blocks", 40, || {
        height(&client) > before
    });

    // an institution that keeps sending what peers must not send is banned by the node it bothers
    let bad = PeerClient::open_as(&d, 1).unwrap();
    let junk = Msg {
        kind: 999,
        body: vec![0],
    };
    let mut refused_at = None;
    for i in 0..20 {
        match bad.call(&net.links[0], &junk, Duration::from_secs(3)) {
            Ok(r) => assert!(is_nack(&r)),
            Err(_) => {
                refused_at = Some(i);
                break;
            }
        }
    }
    let i = refused_at.expect("the institution was never cut off");
    assert!((8..=14).contains(&i), "banned after {i} attempts");
    // a well-behaved institution is still served by the same node
    let good = PeerClient::open_as(&d, 2).unwrap();
    assert_eq!(
        good.call(&net.links[0], &status_msg(), Duration::from_secs(3))
            .unwrap()
            .kind,
        wire::STATUS_REPLY
    );
    // and 3 of 4 institutions are enough to keep going
    let before = height(&client);
    client.submit(&[entry(10)]).unwrap();
    wait("blocks continue", 40, || height(&client) > before);
    stop(nodes, &d);
}

#[test]
fn a_validator_that_moved_is_found_again_by_its_signed_record() {
    let d = dir("moved");
    init_network(&d, NetworkKind::Devnet, 4, free_port_base(), false).unwrap();
    let net = load_network(&d).unwrap();
    // node 0 is told a WRONG address for institution 3 (an old `peers.txt`)
    let mut stale = net.links.clone();
    stale[3].peer = "127.0.0.1:1".to_string();
    let mut nodes = vec![start_node(&d, 0, Some(stale))];
    nodes.extend((1..4).map(|i| start_node(&d, i, None)));
    let client = NodeClient::new(net.peers.clone());
    // node 0 reaches 1 and 2 at once; institution 3 only after a signed record tells it where it is
    wait("node 0 learns where institution 3 is", 90, || {
        client
            .status(net.peers[0])
            .is_ok_and(|s| s.peers_online == 3)
    });
    stop(nodes, &d);
    let _ = public_key_bytes;
}
