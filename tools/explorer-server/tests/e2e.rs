//! The explorer against three REAL nodes on loopback: it downloads the chain, verifies
//! it itself and answers the API; a transfer sent through the nodes shows up decoded.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::TcpListener;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use explorer_server::{Shared, handle, sync_once};
use rand::SeedableRng;
use teko_chain::NetworkKind;
use teko_coin::Op;
use teko_crypto::{SigningKey, generate_signing_key, public_key_bytes, wallet_address};
use teko_economy::EconomyGenesis;
use teko_explorer::Index;
use teko_ledger::{Address, Tx, address_of};
use teko_node::{
    ECONOMY_FILE, EconomyConfig, Node, NodeClient, NodeConfig, NodeHandle, init_economy,
    init_network, load_economy, load_network, load_validator_key, node_dir,
};

fn free_port_base() -> u16 {
    use std::sync::atomic::{AtomicU16, Ordering};
    static NEXT: AtomicU16 = AtomicU16::new(0);
    loop {
        let off = NEXT.fetch_add(3, Ordering::SeqCst);
        let base = 40_000 + (std::process::id() % 2_000) as u16 * 10 + off;
        if (0..3).all(|k| TcpListener::bind(("127.0.0.1", base + k)).is_ok()) {
            return base;
        }
    }
}

fn key(seed: u64) -> SigningKey {
    generate_signing_key(&mut rand::rngs::StdRng::seed_from_u64(seed))
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

fn get(shared: &Shared, url: &str) -> serde_json::Value {
    let r = handle(shared, "GET", url);
    assert_eq!(r.status, 200, "{url}: {}", String::from_utf8_lossy(&r.body));
    serde_json::from_slice(&r.body).unwrap()
}

#[test]
fn the_explorer_verifies_the_chain_of_three_real_nodes_and_decodes_a_transfer() {
    let owner_key = key(7);
    let owner: Address = address_of(&public_key_bytes(&owner_key));
    let owner_text = wallet_address(&owner).unwrap();
    let dir: PathBuf = std::env::temp_dir().join(format!(
        "teko-explorer-e2e-{}-{}",
        std::process::id(),
        free_port_base()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    init_network(&dir, NetworkKind::Devnet, 3, free_port_base(), false).unwrap();
    let mut g: EconomyGenesis = init_economy(&dir, Some(owner), None, false).unwrap();
    g.coin.fee_per_byte = 0;
    g.coin.initial_reward = 1_000;
    g.coin.halving_interval = 100_000;
    std::fs::write(dir.join(ECONOMY_FILE), g.to_text()).unwrap();
    let mut ec = load_economy(&dir).unwrap().unwrap();
    ec.empty_block_interval_ms = 120;
    let nodes: Vec<NodeHandle> = (0..3).map(|i| start_node(&dir, i, &ec)).collect();
    let client = NodeClient::new(load_network(&dir).unwrap().peers);

    let shared = Shared::new(Index::new(ec.genesis.clone()).unwrap(), "devnet")
        .with_store(teko_assets::MetadataStore::new(dir.join("assets-metadata")))
        .with_votes(load_network(&dir).unwrap().profile);
    wait("a few blocks verified by the explorer", 30, || {
        sync_once(&shared, &client);
        shared.index().height() >= 4
    });
    let st = get(&shared, "/api/v1/status");
    assert_eq!(st["chain"]["verified"], true);
    assert_eq!(st["network"], "devnet");
    assert!(st["meta"]["last_error"].is_null(), "{st}");
    assert_eq!(st["meta"]["nodes"].as_array().unwrap().len(), 3);

    // the founder's wallet collected the rewards; send some to Bob through the nodes
    let bob = key(8);
    let bob_text = wallet_address(&address_of(&public_key_bytes(&bob))).unwrap();
    let t = Tx::new_signed(
        &owner_key,
        &ec.genesis.chain_id,
        0,
        teko_coin::COIN_MODULE,
        Op::Transfer {
            to: address_of(&public_key_bytes(&bob)),
            amount: 700,
        }
        .encode(),
    );
    let id = teko_crypto::hex::encode(&t.id());
    client.econ_submit(&[t]).unwrap();
    wait("the transfer verified and indexed", 30, || {
        sync_once(&shared, &client);
        shared.index().tx(&id).is_some()
    });

    let tx = get(&shared, &format!("/api/v1/txs/{id}"));
    assert_eq!(tx["kind"]["kind"], "transfer");
    assert_eq!(tx["kind"]["to"], bob_text);
    assert_eq!(tx["kind"]["amount"]["ypy"], 700);
    assert_eq!(tx["sender"], owner_text);
    let acc = get(&shared, &format!("/api/v1/accounts/{bob_text}"));
    assert_eq!(acc["balance"]["ypy"], 700);
    let hit = get(&shared, &format!("/api/v1/search?q={id}"));
    assert_eq!(hit["type"], "tx");
    let blk = get(&shared, &format!("/api/v1/blocks/{}", tx["height"]));
    assert_eq!(blk["txs"].as_array().unwrap().len(), 1);
    assert_eq!(blk["proposer_owner"], owner_text);
    let vals = get(&shared, "/api/v1/validators");
    assert_eq!(vals.as_array().unwrap().len(), 1);
    // a vehicle registered by its own órgão: the explorer shows the chain's hash and,
    // on a local devnet, the record VERIFIED against that hash
    {
        use teko_assets::devnet::{self, CLASSES, Cast, METADATA_DIR};
        use teko_crypto::asset_meta;
        use teko_node::{NodeWorld, load_class_registrar_keys};
        let mut world = NodeWorld::open(&dir).unwrap();
        let mut cast = Cast::load(&dir).unwrap().with_store(dir.join(METADATA_DIR));
        for class in CLASSES {
            cast = cast.with_registrars(class, load_class_registrar_keys(&dir, class));
        }
        let f = asset_meta::sample("veiculo", 21).unwrap();
        let p: Vec<(&str, &str)> = f.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let rec = asset_meta::canonical("veiculo", &p).unwrap();
        let chassis = devnet::register(&mut world, &cast, &owner, "veiculo", &rec).unwrap();
        wait("the vehicle indexed", 30, || {
            sync_once(&shared, &client);
            shared.index().asset("veiculo", &chassis).is_some()
        });
        let a = get(&shared, &format!("/api/v1/assets/veiculo/{chassis}"));
        assert_eq!(a["asset"]["owner"], owner_text);
        assert_eq!(
            a["asset"]["metadata_hash"],
            teko_crypto::hex::encode(&asset_meta::hash("veiculo", &rec))
        );
        assert_eq!(a["metadata_verified"], true);
        assert!(a["metadata"].to_string().contains("BRA1"), "{a}"); // the plate
        let all = get(&shared, "/api/v1/assets");
        assert_eq!(all["classes"].as_array().unwrap().len(), 2);
        assert_eq!(all["assets"].as_array().unwrap().len(), 1);
        // without the store (a public explorer) only the hash is shown
        let public = Shared::new(Index::new(ec.genesis.clone()).unwrap(), "testnet");
        sync_once(&public, &client);
        let b = get(&public, &format!("/api/v1/assets/veiculo/{chassis}"));
        assert_eq!(b["metadata_available"], false);
        assert_eq!(b["metadata_verified"], false);
        assert_eq!(b["metadata"].as_array().unwrap().len(), 0);
        assert_eq!(
            handle(&public, "GET", "/api/v1/assets/veiculo/nada").status,
            404
        );
    }
    let votes = get(&shared, "/api/v1/votes");
    assert_eq!(votes["available"], true);
    assert_eq!(votes["validators"], 3);
    assert_eq!(votes["threshold"], 2);
    shared.index().audit().unwrap();

    for n in nodes {
        n.stop();
    }
    let _ = std::fs::remove_dir_all(&dir);
}
