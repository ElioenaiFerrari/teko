//! The simulated equivocation on three REAL nodes: the penalty hits the validator's
//! OWNER (10% of the bonded stake is burned), every simulation is a new offence, and
//! refusing the simulation on a locked network.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::TcpListener;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use devnet_sim::equivocate;
use rand::SeedableRng;
use teko_chain::NetworkKind;
use teko_coin::Op;
use teko_crypto::{SigningKey, generate_signing_key, public_key_bytes};
use teko_ledger::{Tx, address_of};
use teko_node::{
    ECONOMY_FILE, EconomyConfig, Node, NodeClient, NodeConfig, NodeHandle, init_economy,
    init_network, load_economy, load_network, load_validator_key, node_dir,
};

/// `NodeWorld` keeps a process-wide cache: the tests of this file take turns.
static TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn free_port_base() -> u16 {
    use std::sync::atomic::{AtomicU16, Ordering};
    static NEXT: AtomicU16 = AtomicU16::new(0);
    loop {
        let off = NEXT.fetch_add(3, Ordering::SeqCst);
        let base = 44_000 + (std::process::id() % 2_000) as u16 * 10 + off;
        if (0..3).all(|k| TcpListener::bind(("127.0.0.1", base + k)).is_ok()) {
            return base;
        }
    }
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

fn key(seed: u64) -> SigningKey {
    generate_signing_key(&mut rand::rngs::StdRng::seed_from_u64(seed))
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

#[test]
fn the_simulated_equivocation_burns_a_tenth_of_the_owners_stake_each_time() {
    let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
    let owner_key: SigningKey = generate_signing_key(&mut rand::rngs::StdRng::seed_from_u64(60));
    let owner = address_of(&public_key_bytes(&owner_key));
    let dir: PathBuf = std::env::temp_dir().join(format!(
        "teko-devnet-sim-{}-{}",
        std::process::id(),
        free_port_base()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    init_network(&dir, NetworkKind::Devnet, 3, free_port_base(), false).unwrap();
    let mut g = init_economy(&dir, Some(owner), None, None, false).unwrap();
    g.coin.fee_per_byte = 0;
    g.coin.initial_reward = 1_000;
    g.coin.halving_interval = 100_000;
    g.coin.slash_bps = 1000; // 10%
    std::fs::write(dir.join(ECONOMY_FILE), g.to_text()).unwrap();
    let ec = load_economy(&dir).unwrap().unwrap();
    let nodes: Vec<NodeHandle> = (0..3).map(|i| start_node(&dir, i, &ec)).collect();
    let client = NodeClient::new(load_network(&dir).unwrap().peers);

    // rewards only come with blocks, and blocks only with transactions (ADR-0034)
    bump(&client, &ec.genesis.chain_id, 4);
    // the owner bonds 2.000 of the rewards as stake
    wait("rewards in the owner's wallet", 30, || {
        client
            .econ_sync(&ec.genesis)
            .is_ok_and(|c| teko_coin::balance(c.ledger(), &owner) >= 3_000)
    });
    let stake = Tx::new_signed(
        &owner_key,
        &ec.genesis.chain_id,
        0,
        teko_coin::COIN_MODULE,
        Op::Stake { amount: 2_000 }.encode(),
    );
    client.econ_submit(&[stake]).unwrap();
    wait("stake bonded", 30, || {
        client
            .econ_sync(&ec.genesis)
            .is_ok_and(|c| teko_coin::stake_of(c.ledger(), &owner) == 2_000)
    });

    // first simulation: 10% of 2.000 = 200 burned, taken from the OWNER
    let r = equivocate(&dir, 0, Duration::from_secs(20)).unwrap();
    assert!(r.applied);
    assert_eq!(r.stake_before, 2_000);
    assert_eq!(r.stake_after, 1_800);
    assert_eq!(r.penalty(), 200);
    assert_eq!(r.slash_bps, 1000);

    // a second simulation is a NEW offence (another round): 10% of 1.800 = 180
    std::thread::sleep(Duration::from_millis(1_200));
    let r2 = equivocate(&dir, 0, Duration::from_secs(20)).unwrap();
    assert_eq!(r2.stake_before, 1_800);
    assert_eq!(r2.stake_after, 1_620);
    assert_eq!(r2.penalty(), 180);

    // the chain stays consistent: supply is conserved after the burns
    let chain = client.econ_sync(&ec.genesis).unwrap();
    teko_coin::audit(chain.ledger()).unwrap();

    for n in nodes {
        n.stop();
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn it_refuses_a_network_without_blocks_or_without_economy() {
    let dir: PathBuf = std::env::temp_dir().join(format!(
        "teko-devnet-sim-none-{}-{}",
        std::process::id(),
        free_port_base()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(equivocate(&dir, 0, Duration::from_secs(1)).is_err()); // no network at all
    init_network(&dir, NetworkKind::Devnet, 3, free_port_base(), false).unwrap();
    let e = equivocate(&dir, 0, Duration::from_secs(1)).unwrap_err();
    assert!(e.contains("economia"), "{e}"); // network without an economy
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_simulated_organ_vetoes_a_pending_trade_and_the_buyer_is_refunded_on_real_nodes() {
    use teko_assets::devnet::{self, CLASSES, Cast, METADATA_DIR, TradeAction, TradeRequest};
    use teko_assets::{ASSETS_MODULE, AcceptOp, Op as AssetOp};
    use teko_crypto::asset_meta;
    use teko_node::{NodeWorld, load_class_registrar_keys};

    let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());

    let buyer: SigningKey = generate_signing_key(&mut rand::rngs::StdRng::seed_from_u64(61));
    let buyer_addr = address_of(&public_key_bytes(&buyer));
    let dir: PathBuf = std::env::temp_dir().join(format!(
        "teko-devnet-veto-{}-{}",
        std::process::id(),
        free_port_base()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    init_network(&dir, NetworkKind::Devnet, 3, free_port_base(), false).unwrap();
    let mut g = init_economy(&dir, Some(buyer_addr), None, Some(10_000), false).unwrap();
    g.coin.fee_per_byte = 0;
    g.coin.initial_reward = 1_000;
    for c in &mut g.classes {
        c.window_secs = 600; // long: the veto must come INSIDE it
    }
    std::fs::write(dir.join(ECONOMY_FILE), g.to_text()).unwrap();
    let ec = load_economy(&dir).unwrap().unwrap();
    let nodes: Vec<NodeHandle> = (0..3).map(|i| start_node(&dir, i, &ec)).collect();

    let mut world = NodeWorld::open(&dir).unwrap();
    let mut cast = Cast::load(&dir).unwrap().with_store(dir.join(METADATA_DIR));
    for class in CLASSES {
        cast = cast.with_registrars(class, load_class_registrar_keys(&dir, class));
    }
    let f = asset_meta::sample("veiculo", 33).unwrap();
    let p: Vec<(&str, &str)> = f.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let rec = asset_meta::canonical("veiculo", &p).unwrap();
    let cp = cast.counterparty_address();
    let id = devnet::register(&mut world, &cast, &cp, "veiculo", &rec).unwrap();
    let mut accept = |r: &TradeRequest<'_>| {
        assert_eq!(r.action, TradeAction::Accept);
        let op = AssetOp::Accept(AcceptOp {
            class: r.class.into(),
            asset: r.asset.as_bytes().to_vec(),
            price: r.price,
            version: r.version,
            metadata_hash: asset_meta::hash(r.class, r.metadata).to_vec(),
        });
        Ok(Tx::new_signed_with_fee(
            &buyer,
            r.chain_id,
            r.nonce,
            r.fee,
            ASSETS_MODULE,
            op.encode(),
        ))
    };
    devnet::buy_from_counterparty(
        &mut world,
        &cast,
        &buyer_addr,
        "veiculo",
        &id,
        4_000,
        &mut accept,
    )
    .unwrap();
    let client = NodeClient::new(load_network(&dir).unwrap().peers);
    let before = client.econ_sync(&ec.genesis).unwrap();
    let balance_in_escrow = teko_coin::balance(before.ledger(), &buyer_addr);
    assert!(
        teko_coin::balance(
            before.ledger(),
            &teko_assets::escrow_address(b"veiculo", id.as_bytes())
        ) == 4_000
    );

    // the órgão simulator vetoes it (the same code `make devnet-veto` runs)
    let said = devnet_sim::veto(&dir, "veiculo", &id, "penhora judicial nao lancada").unwrap();
    assert!(said.contains("vetou"));
    let after = client.econ_sync(&ec.genesis).unwrap();
    // refunded in full (the buyer here is also the founder, who earns the reward of the veto block too)
    let got = teko_coin::balance(after.ledger(), &buyer_addr) - balance_in_escrow;
    assert!((4_000..=4_000 + 2_000).contains(&got), "refunded: {got}");
    assert_eq!(
        teko_coin::balance(
            after.ledger(),
            &teko_assets::escrow_address(b"veiculo", id.as_bytes())
        ),
        0
    );
    assert!(teko_assets::offer_of(after.ledger(), b"veiculo", id.as_bytes()).is_none());
    assert_eq!(
        teko_assets::asset(after.ledger(), b"veiculo", id.as_bytes())
            .unwrap()
            .owner,
        cp.to_vec()
    );
    teko_coin::audit(after.ledger()).unwrap();
    // nothing pending is a clear error
    assert!(devnet_sim::veto(&dir, "veiculo", &id, "x").is_err());
    for n in nodes {
        n.stop();
    }
    let _ = std::fs::remove_dir_all(&dir);
}
