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
        let off = NEXT.fetch_add(4, Ordering::SeqCst);
        let base = 36_000 + (std::process::id() % 2_000) as u16 * 10 + off;
        if (0..4).all(|k| TcpListener::bind(("127.0.0.1", base + k)).is_ok()) {
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
    let mut g = init_economy(&dir, Some(*owner), None, None, false).unwrap();
    g.coin.fee_per_byte = 0;
    g.coin.initial_reward = 1_000;
    g.coin.halving_interval = 100_000;
    g.coin.unbonding_period = 5;
    g.coin.min_total_stake = 1;
    g.coin.min_validator_stake = 100;
    tweak(&mut g);
    std::fs::write(dir.join(ECONOMY_FILE), g.to_text()).unwrap();
    let ec = load_economy(&dir).unwrap().unwrap();
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

/// Blocks only exist for transactions (ADR-0034), so a test that wants the chain to move has
/// to make activity: a transaction from a throwaway key (it links a fresh key, a harmless
/// state change that needs no balance) and wait until the first node has a higher block.
fn bump(client: &NodeClient, chain_id: &str, times: u64) {
    static SEED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(900_000);
    for _ in 0..times {
        let before = client
            .econ_status(client.addrs()[0])
            .map_or(0, |s| s.height);
        let sk = key(SEED.fetch_add(1, std::sync::atomic::Ordering::SeqCst));
        let t = Tx::new_signed(
            &sk,
            chain_id,
            0,
            teko_coin::COIN_MODULE,
            Op::SetValidatorKey {
                key: public_key_bytes(&sk),
            }
            .encode(),
        );
        client.econ_submit(&[t]).unwrap();
        wait("a block for the activity", 30, || {
            client
                .econ_status(client.addrs()[0])
                .is_ok_and(|s| s.height > before)
        });
    }
}

fn tx(sk: &SigningKey, chain: &str, nonce: u64, op: Op) -> Tx {
    Tx::new_signed(sk, chain, nonce, teko_coin::COIN_MODULE, op.encode())
}

#[test]
fn the_genesis_gives_the_founder_the_first_coins_so_the_network_can_start_with_no_block_at_all() {
    let owner = key(3);
    let owner_addr = address_of(&public_key_bytes(&owner));
    let dir = std::env::temp_dir().join(format!(
        "teko-econ-alloc-{}-{}",
        std::process::id(),
        free_port_base()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    init_network(&dir, NetworkKind::Devnet, 3, free_port_base(), false).unwrap();
    // `--alloc 5000`: the first coins of the network go to the founder's wallet, in the GENESIS
    let mut g = init_economy(&dir, Some(owner_addr), None, Some(5_000), false).unwrap();
    assert_eq!(g.allocations, vec![(owner_addr, 5_000)]);
    g.coin.fee_per_byte = 0;
    g.coin.initial_reward = 1_000;
    g.coin.halving_interval = 100_000;
    g.coin.unbonding_period = 5;
    g.coin.min_total_stake = 1;
    g.coin.min_validator_stake = 100;
    std::fs::write(dir.join(ECONOMY_FILE), g.to_text()).unwrap();
    let ec = load_economy(&dir).unwrap().unwrap();
    let nodes: Vec<_> = (0..3).map(|i| Some(start_node(&dir, i, &ec))).collect();
    let net = Net {
        dir,
        nodes,
        genesis: ec.genesis,
    };
    let c = client(&net);

    // height 0, and a client that replays the genesis already sees the founder's coins
    assert!(heights(&c).iter().all(|h| *h == Some(0)));
    let chain = c.econ_sync(&net.genesis).unwrap();
    assert_eq!(chain.height(), 0);
    assert_eq!(teko_coin::balance(chain.ledger(), &owner_addr), 5_000);
    assert_eq!(teko_coin::minted(chain.ledger()), 5_000);
    teko_coin::audit(chain.ledger()).unwrap();

    // with those coins the founder can act at once: stake 3.000 makes the first block
    let chain_id = net.genesis.chain_id.clone();
    c.econ_submit(&[tx(&owner, &chain_id, 0, Op::Stake { amount: 3_000 })])
        .unwrap();
    wait("the stake block", 20, || {
        heights(&c).iter().all(|h| h.is_some_and(|h| h >= 1))
    });
    let chain = c.econ_sync(&net.genesis).unwrap();
    assert_eq!(teko_coin::stake_of(chain.ledger(), &owner_addr), 3_000);
    // minted = genesis allocation + the one block reward the founder earned
    assert_eq!(teko_coin::minted(chain.ledger()), 5_000 + 1_000);
    teko_coin::audit(chain.ledger()).unwrap();
    for n in net.nodes.into_iter().flatten() {
        n.stop();
    }
}

/// Sends one wire message to a node (a forged consensus message, as a malicious peer would).
fn send_raw(addr: std::net::SocketAddr, m: &teko_node::wire::Msg) {
    let mut s = std::net::TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    teko_node::wire::write_msg(&mut s, m).unwrap();
    let _ = teko_node::wire::read_msg(&mut s);
}

#[test]
fn an_institution_that_equivocates_on_the_vote_chain_loses_ten_percent_of_its_bond_in_the_economy()
{
    use prost::Message;
    use teko_bft::{Vote, VoteKind};
    use teko_node::wire::{self, BftEnvelope};

    let (founder, w1) = (key(5), key(70));
    let (founder_addr, w1_addr) = (
        address_of(&public_key_bytes(&founder)),
        address_of(&public_key_bytes(&w1)),
    );
    let dir = std::env::temp_dir().join(format!(
        "teko-econ-bond-{}-{}",
        std::process::id(),
        free_port_base()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    init_network(&dir, NetworkKind::Devnet, 4, free_port_base(), false).unwrap();
    let mut g = init_economy(&dir, Some(founder_addr), None, None, false).unwrap();
    assert_eq!(
        g.coin.vote_chain_id, "teko-devnet-1",
        "the economy knows the vote chain"
    );
    g.coin.fee_per_byte = 0;
    g.coin.unbonding_period = 1_000;
    g.coin.founder_target = 1_000_000_000; // the founder phase lasts: node 0 alone decides the economy
    g.allocations.push((w1_addr, 5_000)); // institution 1's owner has coins for the bond
    std::fs::write(dir.join(ECONOMY_FILE), g.to_text()).unwrap();
    let ec = load_economy(&dir).unwrap().unwrap();
    let nodes: Vec<_> = (0..4).map(|i| Some(start_node(&dir, i, &ec))).collect();
    let mut net = Net {
        dir,
        nodes,
        genesis: ec.genesis,
    };
    let c = client(&net);
    let chain_id = net.genesis.chain_id.clone();

    // institution 1 (node key 1) is linked to its owner's wallet and puts a bond of 3000
    let node1 = load_validator_key(&net.dir, 1).unwrap();
    c.econ_submit(&[
        tx(
            &w1,
            &chain_id,
            0,
            Op::SetValidatorKey {
                key: public_key_bytes(&node1),
            },
        ),
        tx(&w1, &chain_id, 1, Op::Stake { amount: 3_000 }),
    ])
    .unwrap();
    wait("the bond is in place", 20, || {
        c.econ_sync(&net.genesis)
            .is_ok_and(|ch| teko_coin::stake_of(ch.ledger(), &w1_addr) == 3_000)
    });

    // the vote chain loses its quorum (2 of 4 up) so a height stays open on nodes 0 and 1
    net.nodes[2].take().unwrap().stop();
    net.nodes[3].take().unwrap().stop();
    let es = vec![teko_bulletin::Entry::new(
        teko_bulletin::EntryKind::BALLOT,
        vec![1, 2, 3],
    )];
    c.submit(&es).unwrap();
    std::thread::sleep(Duration::from_millis(1_500));

    // institution 1 signs two conflicting prevotes for the same height and round
    let vote_chain = load_network(&net.dir).unwrap().profile.chain_id;
    let target = load_network(&net.dir).unwrap().peers[0];
    for tag in [1u8, 2u8] {
        let v = Vote::new(
            &vote_chain,
            &node1,
            VoteKind::Prevote,
            1,
            0,
            Some(&[tag; 32]),
        );
        let env = BftEnvelope {
            proposal: Vec::new(),
            vote: v.encode_to_vec(),
        };
        send_raw(target, &wire::msg(wire::VOTE_BFT, &env));
    }

    // node 0 detected it live, took it to the economy, and 10% of the bond was BURNED
    wait("the bond is slashed", 30, || {
        c.econ_sync(&net.genesis)
            .is_ok_and(|ch| teko_coin::stake_of(ch.ledger(), &w1_addr) == 2_700)
    });
    let chain = c.econ_sync(&net.genesis).unwrap();
    assert_eq!(teko_coin::burned(chain.ledger()), 300);
    // the founder (not accused) lost nothing, and the books still balance
    assert_eq!(teko_coin::stake_of(chain.ledger(), &founder_addr), 0);
    teko_coin::audit(chain.ledger()).unwrap();
    for n in net.nodes.into_iter().flatten() {
        n.stop();
    }
}

#[test]
fn no_block_is_made_on_its_own_the_chain_only_grows_with_transactions() {
    let owner = key(2);
    let net = boot("idle", &address_of(&public_key_bytes(&owner)), |_| {});
    let c = client(&net);
    // nothing to do: three seconds later there is still no block, on any node
    std::thread::sleep(Duration::from_secs(3));
    assert!(
        heights(&c).iter().all(|h| *h == Some(0)),
        "{:?}",
        heights(&c)
    );
    // one transaction makes exactly one block, on every node
    bump(&c, &net.genesis.chain_id, 1);
    wait("every node has the block", 20, || {
        heights(&c).iter().all(|h| *h == Some(1))
    });
    // and then it stands still again
    std::thread::sleep(Duration::from_secs(3));
    assert!(
        heights(&c).iter().all(|h| *h == Some(1)),
        "{:?}",
        heights(&c)
    );
    for n in net.nodes.into_iter().flatten() {
        n.stop();
    }
}

#[test]
fn the_founder_alone_produces_blocks_and_every_node_and_a_client_verify_the_same_chain() {
    let owner = key(1);
    let net = boot("a", &address_of(&public_key_bytes(&owner)), |_| {});
    let c = client(&net);
    bump(&c, &net.genesis.chain_id, 5);
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
    bump(&c, &chain_id, 4);
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
    bump(&c, &chain_id, 4);
    wait("blocks keep coming with three validators", 60, || {
        heights(&c).iter().all(|x| x.is_some_and(|x| x >= h + 4))
    });

    // restart node 2: it replays its log (re-verified) and catches up
    let stopped = net.nodes[2].take().unwrap();
    stopped.stop();
    let target = c.econ_status(c.addrs()[0]).unwrap().height + 3;
    bump(&c, &chain_id, 3);
    wait("the network moves with node 2 down", 60, || {
        c.econ_status(c.addrs()[0])
            .is_ok_and(|s| s.height >= target)
    });
    let ec = load_economy(&net.dir).unwrap().unwrap();
    net.nodes[2] = Some(start_node(&net.dir, 2, &ec));
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
    bump(&c, &net.genesis.chain_id, 3);
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
    bump(&c, &net.genesis.chain_id, 2);
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
