//! The economy ledger on BFT across three real nodes on loopback (ADR-0028): the founder
//! alone, the founder phase ending when stake exists, new validators joining by stake,
//! a restart that catches up, and a client that verifies everything without trust.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::TcpListener;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use rand::SeedableRng;
use teko_chain::NetworkKind;
use teko_coin::Op;
use teko_crypto::{SigningKey, generate_signing_key, public_key_bytes};
use teko_economy::EconomyGenesis;
use teko_ledger::{Address, Tx, address_of};
use teko_node::{
    ECONOMY_FILE, EconomyConfig, Node, NodeClient, NodeConfig, NodeHandle, init_economy,
    init_network, load_economy, load_network, load_validator_key, node_dir,
};

/// `NodeWorld` keeps a process-wide cache and a counter: the tests that use it take turns.
static WORLD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn free_port_base() -> u16 {
    use std::sync::atomic::{AtomicU16, Ordering};
    static NEXT: AtomicU16 = AtomicU16::new(0);
    loop {
        let off = NEXT.fetch_add(3, Ordering::SeqCst);
        let base = 36_000 + (std::process::id() % 2_000) as u16 * 10 + off;
        if (0..3).all(|k| TcpListener::bind(("127.0.0.1", base + k)).is_ok()) {
            return base;
        }
    }
}

fn key(seed: u64) -> SigningKey {
    generate_signing_key(&mut rand::rngs::StdRng::seed_from_u64(seed))
}

struct Net {
    dir: PathBuf,
    nodes: Vec<Option<NodeHandle>>,
    genesis: EconomyGenesis,
}

fn start_node(dir: &std::path::Path, i: usize, ec: &EconomyConfig) -> NodeHandle {
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
            economy: Some(ec.clone()),
        },
        addr,
    )
    .unwrap()
}

fn boot(tag: &str, owner: &Address, tweak: impl Fn(&mut EconomyGenesis)) -> Net {
    let dir = std::env::temp_dir().join(format!(
        "teko-econ-{tag}-{}-{}",
        std::process::id(),
        free_port_base()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    init_network(&dir, NetworkKind::Devnet, 3, free_port_base(), false).unwrap();
    let mut g = init_economy(&dir, Some(*owner), None, false).unwrap();
    g.coin.fee_per_byte = 0;
    g.coin.initial_reward = 1_000;
    g.coin.halving_interval = 100_000;
    g.coin.unbonding_period = 5;
    g.coin.min_total_stake = 1;
    g.coin.min_validator_stake = 100;
    tweak(&mut g);
    std::fs::write(dir.join(ECONOMY_FILE), g.to_text()).unwrap();
    let mut ec = load_economy(&dir).unwrap().unwrap();
    ec.empty_block_interval_ms = 120;
    let nodes = (0..3).map(|i| Some(start_node(&dir, i, &ec))).collect();
    Net {
        dir,
        nodes,
        genesis: ec.genesis,
    }
}

fn client(net: &Net) -> NodeClient {
    NodeClient::new(load_network(&net.dir).unwrap().peers)
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

fn heights(c: &NodeClient) -> Vec<Option<u64>> {
    c.addrs()
        .iter()
        .map(|a| c.econ_status(*a).ok().map(|s| s.height))
        .collect()
}

fn tx(sk: &SigningKey, chain: &str, nonce: u64, op: Op) -> Tx {
    Tx::new_signed(sk, chain, nonce, teko_coin::COIN_MODULE, op.encode())
}

#[test]
fn the_founder_alone_produces_blocks_and_every_node_and_a_client_verify_the_same_chain() {
    let owner = key(1);
    let net = boot("a", &address_of(&public_key_bytes(&owner)), |_| {});
    let c = client(&net);
    wait("all three nodes reach height 5", 40, || {
        heights(&c).iter().all(|h| h.is_some_and(|h| h >= 5))
    });
    // only the founder (node 0) is a validator; the others just replicate
    let st: Vec<_> = c
        .addrs()
        .iter()
        .map(|a| c.econ_status(*a).unwrap())
        .collect();
    assert!(st[0].is_validator && !st[1].is_validator && !st[2].is_validator);
    assert_eq!(st[0].validators.len(), 1);
    assert!(st[0].founder_phase_active);
    // a client trusts nobody: it replays and verifies every certificate
    let chain = c.econ_sync(&net.genesis).unwrap();
    assert!(chain.height() >= 5);
    // the OWNER's account (not the node key) collected the rewards: 1000 per block
    let owner_addr = address_of(&public_key_bytes(&owner));
    assert_eq!(
        teko_coin::balance(chain.ledger(), &owner_addr),
        chain.height() * 1_000
    );
    assert_eq!(teko_coin::minted(chain.ledger()), chain.height() * 1_000); // no pre-mine
    for n in net.nodes.into_iter().flatten() {
        n.stop();
    }
}

#[test]
fn staking_ends_the_founder_phase_new_validators_join_and_a_restarted_node_catches_up() {
    let owner = key(10);
    let owner_addr = address_of(&public_key_bytes(&owner));
    let net = boot("b", &owner_addr, |g| g.coin.founder_target = 3_000);
    let mut net = net;
    let c = client(&net);
    let chain_id = net.genesis.chain_id.clone();
    wait("some rewards", 40, || {
        heights(&c)[0].is_some_and(|h| h >= 4)
    });
    // the owner (a wallet key) bonds 3000 and funds the other two nodes' accounts
    let n1 = load_validator_key(&net.dir, 1).unwrap();
    let n2 = load_validator_key(&net.dir, 2).unwrap();
    let st = |sk: &SigningKey, n: u64, op: Op| tx(sk, &chain_id, n, op);
    c.econ_submit(&[st(&owner, 0, Op::Stake { amount: 3_000 })])
        .unwrap();
    c.econ_submit(&[st(
        &owner,
        1,
        Op::Transfer {
            to: address_of(&public_key_bytes(&n1)),
            amount: 600,
        },
    )])
    .unwrap();
    c.econ_submit(&[st(
        &owner,
        2,
        Op::Transfer {
            to: address_of(&public_key_bytes(&n2)),
            amount: 600,
        },
    )])
    .unwrap();
    wait("the founder phase ends", 40, || {
        c.econ_status(c.addrs()[0])
            .is_ok_and(|s| !s.founder_phase_active)
    });
    // nodes 1 and 2 bond with their own keys (their account key is their consensus key)
    c.econ_submit(&[st(&n1, 0, Op::Stake { amount: 500 })])
        .unwrap();
    c.econ_submit(&[st(&n2, 0, Op::Stake { amount: 500 })])
        .unwrap();
    wait("three validators", 40, || {
        c.econ_status(c.addrs()[0])
            .is_ok_and(|s| s.validators.len() == 3)
    });
    let st0 = c.econ_status(c.addrs()[0]).unwrap();
    let total: u64 = st0.validators.iter().map(|v| v.2).sum();
    assert!(total <= 10_001 && st0.validators.iter().all(|v| v.2 >= 1));
    assert!(st0.total_stake >= 4_000);

    // the three validators keep deciding blocks together
    let h = c.econ_status(c.addrs()[0]).unwrap().height;
    wait("blocks keep coming with three validators", 60, || {
        heights(&c).iter().all(|x| x.is_some_and(|x| x >= h + 4))
    });

    // restart node 2: it replays its log (re-verified) and catches up
    let stopped = net.nodes[2].take().unwrap();
    stopped.stop();
    let target = c.econ_status(c.addrs()[0]).unwrap().height + 3;
    wait("the network moves with node 2 down", 60, || {
        c.econ_status(c.addrs()[0])
            .is_ok_and(|s| s.height >= target)
    });
    let ec = load_economy(&net.dir).unwrap().unwrap();
    let mut ec2 = ec.clone();
    ec2.empty_block_interval_ms = 120;
    net.nodes[2] = Some(start_node(&net.dir, 2, &ec2));
    wait("node 2 catches up", 60, || {
        let a = c.econ_status(c.addrs()[0]).map(|s| s.height).unwrap_or(0);
        c.econ_status(c.addrs()[2])
            .is_ok_and(|s| s.height + 3 >= a && s.height >= target)
    });

    // a client verifies the whole chain, including the validator-set changes
    let chain = c.econ_sync(&net.genesis).unwrap();
    assert!(!teko_coin::founder_phase_active(chain.ledger()));
    assert_eq!(chain.validator_set().unwrap().validators().len(), 3);
    teko_coin::audit(chain.ledger()).unwrap();
    for n in net.nodes.into_iter().flatten() {
        n.stop();
    }
}

#[test]
fn each_asset_class_has_its_own_organ_and_a_vehicle_registers_through_the_nodes() {
    use teko_assets::devnet::{self, CLASSES, Cast, METADATA_DIR};
    use teko_crypto::asset_meta;
    use teko_node::{NodeWorld, load_class_registrar_keys};

    let _turn = WORLD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let owner = key(40);
    let owner_addr = address_of(&public_key_bytes(&owner));
    let net = boot("assets", &owner_addr, |_| {});

    // the genesis has one class per órgão, each with its own keys
    let names: Vec<&str> = net
        .genesis
        .classes
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(names, vec!["imovel", "veiculo"]);
    assert_ne!(
        net.genesis.classes[0].registrars,
        net.genesis.classes[1].registrars
    );

    let mut world = NodeWorld::open(&net.dir).unwrap();
    let mut cast = Cast::load(&net.dir)
        .unwrap()
        .with_store(net.dir.join(METADATA_DIR));
    for class in CLASSES {
        cast = cast.with_registrars(class, load_class_registrar_keys(&net.dir, class));
    }
    let f = asset_meta::sample("veiculo", 12).unwrap();
    let p: Vec<(&str, &str)> = f.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let rec = asset_meta::canonical("veiculo", &p).unwrap();
    let id = devnet::register(&mut world, &cast, &owner_addr, "veiculo", &rec).unwrap();
    assert_eq!(id, "9BWZZZ377VT000012");

    // a client that trusts nobody sees the vehicle with the hash of the record
    let c = client(&net);
    wait("the vehicle on the verified chain", 30, || {
        c.econ_sync(&net.genesis).is_ok_and(|ch| {
            teko_assets::asset(ch.ledger(), b"veiculo", id.as_bytes())
                .is_some_and(|a| a.metadata_hash == asset_meta::hash("veiculo", &rec))
        })
    });
    for n in net.nodes.into_iter().flatten() {
        n.stop();
    }
}

#[test]
fn reopening_the_world_downloads_only_the_new_blocks() {
    use teko_economy::EconomyChain;
    use teko_node::NodeWorld;

    let _turn = WORLD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let owner = key(70);
    let net = boot("cache", &address_of(&public_key_bytes(&owner)), |_| {});
    let c = client(&net);
    wait("a few blocks", 30, || {
        heights(&c).iter().all(|h| h.is_some_and(|h| h >= 3))
    });

    let first = NodeWorld::full_syncs();
    let h1 = NodeWorld::open(&net.dir).unwrap().chain().height(); // dropped: back to the cache
    assert_eq!(
        NodeWorld::full_syncs(),
        first + 1,
        "the first open verifies everything"
    );
    wait("more blocks", 30, || {
        heights(&c).iter().all(|h| h.is_some_and(|h| h > h1))
    });

    let w = NodeWorld::open(&net.dir).unwrap();
    assert_eq!(
        NodeWorld::full_syncs(),
        first + 1,
        "the second open reuses the verified chain"
    );
    assert!(w.chain().height() > h1, "and picked up the new blocks");
    // the same state as verifying that many blocks from the genesis
    let fresh = c.econ_sync(&net.genesis).unwrap();
    let h = w.chain().height();
    assert!(fresh.height() >= h);
    let replayed =
        EconomyChain::replay(net.genesis.clone(), &fresh.blocks()[..h as usize]).unwrap();
    assert_eq!(
        replayed.ledger().state_root(),
        w.chain().ledger().state_root()
    );
    drop(w);
    for n in net.nodes.into_iter().flatten() {
        n.stop();
    }
}
