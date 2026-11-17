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
        let off = NEXT.fetch_add(16, Ordering::SeqCst);
        let base = 44_000 + (std::process::id() % 2_000) as u16 * 10 + off;
        if (0..16).all(|k| TcpListener::bind(("127.0.0.1", base + k)).is_ok()) {
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
            links: net.links,
            listen_peer: None,
            advertise: None,
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

#[test]
fn simulated_organ_members_cosign_a_credential_proposed_by_the_issuer_wallet_on_real_nodes() {
    use teko_credentials::{CREDENTIALS_MODULE, Op, ProposeOp, Status, credential_id};
    use teko_node::{DEVNET_CRED_TYPES, init_economy_with_issuer};

    let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
    let issuer: SigningKey = generate_signing_key(&mut rand::rngs::StdRng::seed_from_u64(77));
    let issuer_addr = address_of(&public_key_bytes(&issuer));
    let dir: PathBuf = std::env::temp_dir().join(format!(
        "teko-devnet-cosign-{}-{}",
        std::process::id(),
        free_port_base()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    init_network(&dir, NetworkKind::Devnet, 3, free_port_base(), false).unwrap();
    let mut g = init_economy_with_issuer(
        &dir,
        Some(issuer_addr),
        Some(issuer_addr),
        None,
        Some(10_000),
        false,
    )
    .unwrap();
    g.coin.fee_per_byte = 0;
    std::fs::write(dir.join(ECONOMY_FILE), g.to_text()).unwrap();
    let ec = load_economy(&dir).unwrap().unwrap();
    // the issuer wallet is a member of every devnet organ, with 2 simulated co-members
    for (name, _, _, _) in DEVNET_CRED_TYPES {
        let t = teko_credentials::type_info(&ec.genesis.build_ledger().unwrap(), name.as_bytes())
            .unwrap();
        assert_eq!(t.members.len(), 3);
        assert!(t.members.contains(&issuer_addr.to_vec()));
    }
    let nodes: Vec<NodeHandle> = (0..3).map(|i| start_node(&dir, i, &ec)).collect();
    let client = NodeClient::new(load_network(&dir).unwrap().peers);

    let holder = public_key_bytes(&key(8_000));
    // the nodes use the real clock
    let valid_until = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 100_000_000;
    let p = ProposeOp {
        ctype: b"cnh-b".to_vec(),
        subject: holder.clone(),
        commitment: vec![5; 32],
        claims: vec![],
        valid_until,
        sealed: vec![],
    };
    let id = credential_id(
        &p.ctype,
        &p.subject,
        &p.commitment,
        &p.claims,
        p.valid_until,
    );
    // an ISSUER-signed proposal is only 1 of 2: it waits for approvals
    let tx = Tx::new_signed(
        &issuer,
        &ec.genesis.chain_id,
        0,
        CREDENTIALS_MODULE,
        Op::Propose(p).encode(),
    );
    client.econ_submit(&[tx]).unwrap();
    wait("the proposal is in a block", 30, || {
        client
            .econ_sync(&ec.genesis)
            .is_ok_and(|c| teko_credentials::credential(c.ledger(), b"cnh-b", &id).is_some())
    });
    let c = client.econ_sync(&ec.genesis).unwrap();
    assert_eq!(
        teko_credentials::credential(c.ledger(), b"cnh-b", &id)
            .unwrap()
            .status_at(0),
        Status::Proposed
    );

    // the simulator approves with the simulated members, only as many as the quorum needs
    let said = devnet_sim::cosign(&dir, Some("cnh-b")).unwrap();
    assert!(said.contains("cnh-b"), "{said}");
    let c = client.econ_sync(&ec.genesis).unwrap();
    let rec = teko_credentials::credential(c.ledger(), b"cnh-b", &id).unwrap();
    assert_eq!(rec.approvals.len(), 2);
    assert!(rec.active_at > rec.approved_at && rec.approved_at > 0);
    assert!(matches!(rec.status_at(rec.approved_at), Status::Pending));
    // nothing left to approve is a clear error
    assert!(devnet_sim::cosign(&dir, None).is_err());
    for n in nodes {
        n.stop();
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A device on its own TCP port with the given trust policy; `hal` keeps its storage, so a second
/// instance on the same `hal` is the SAME device booted again with another trust file.
fn spawn_device(
    hal: &teko_device_core::testing::ScriptedHal,
    trust: teko_attestation::TrustPolicy,
    factory: teko_attestation::FactoryData,
) -> std::net::SocketAddr {
    use device_emulator::{LinkSlot, Linked, serve};
    use rand::rand_core::UnwrapErr;
    use rand::rngs::SysRng;
    use teko_device_core::{Device, DeviceConfig};
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let hal = hal.clone();
    std::thread::spawn(move || {
        let slot = LinkSlot::default();
        let cfg = DeviceConfig {
            pin_kdf_iterations: 50,
            ..DeviceConfig::default()
        };
        let mut dev = Device::new(
            Linked::new(hal, slot.clone()),
            UnwrapErr(SysRng),
            cfg,
            trust,
            Some(factory),
        )
        .unwrap()
        .with_registration_blinder(Box::new(teko_registration::RsaRegistration));
        let _ = serve(&listener, &mut dev, &slot, |_| {});
    });
    addr
}

#[test]
fn the_civic_credential_is_issued_on_real_nodes_by_the_organ_quorum_and_installed_on_the_device() {
    use issuer_cli::{Issuer, scratch_dir};
    use prost::Message;
    use teko_assets::devnet::World;
    use teko_client::{CivicRecord, CredentialOpArgs, CredentialOpKind, DeviceClient};
    use teko_credentials::civic::{CivicSubject, civic_claims};
    use teko_crypto::civic::{BundleTx, CIVIC_TYPE, CivicBundle, CivicPolicy};
    use teko_crypto::ledger::{CRED_OP_APPROVE, cred_propose_payload, cred_vote_payload};
    use teko_device_core::testing::ScriptedHal;
    use teko_node::init_economy_with_issuer;
    use teko_transport::connect_tcp;

    let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
    // 1) the citizen's device: firmware, PIN and wallet (its wallet will be a MEMBER of the organ)
    let issuer = Issuer::open(scratch_dir("civic-real"));
    let trust0 = issuer.dev_setup().unwrap();
    let factory = issuer.provision_device().unwrap();
    let hal = ScriptedHal::new();
    hal.set_default_pin(Some("482913"));
    hal.set_auto_words(true);
    let addr1 = spawn_device(&hal, trust0.clone(), factory.clone());
    let mut c = DeviceClient::new(connect_tcp(addr1, Duration::from_secs(3)).unwrap());
    let d = issuer.dir();
    let (m, b) = (
        std::fs::read(d.join("firmware-v1.manifest")).unwrap(),
        std::fs::read(d.join("firmware-v1.blob")).unwrap(),
    );
    c.install_firmware(&trust0, &m, &b, &mut |_, _| {}).unwrap();
    c.setup_pin().unwrap();
    let wallet = c.wallet_setup().unwrap();
    let wallet_addr = teko_crypto::parse_wallet_address(&wallet.address).unwrap();
    drop(c);

    // 2) the network: the device wallet is a member of the `civic` organ (with 2 simulated members)
    let net = d.join("net");
    init_network(&net, NetworkKind::Devnet, 3, free_port_base(), false).unwrap();
    let mut g = init_economy_with_issuer(
        &net,
        Some(wallet_addr),
        Some(wallet_addr),
        None,
        Some(10_000),
        false,
    )
    .unwrap();
    for t in &mut g.credtypes {
        t.contest_secs = 2; // short: the credential becomes active within the test
    }
    std::fs::write(net.join(ECONOMY_FILE), g.to_text()).unwrap();
    let ec = load_economy(&net).unwrap().unwrap();
    let nodes: Vec<NodeHandle> = (0..3).map(|i| start_node(&net, i, &ec)).collect();

    // 3) the SAME device boots again with a trust file that now names the organ (sync_civic_trust)
    let trust = teko_attestation::TrustPolicy::parse(
        &std::fs::read_to_string(d.join("trust.txt")).unwrap(),
    )
    .unwrap();
    assert_eq!(trust.civic_chain_id, ec.genesis.chain_id);
    assert_eq!(trust.civic_members.len(), 3);
    let addr2 = spawn_device(&hal, trust.clone(), factory);
    let mut c = DeviceClient::new(connect_tcp(addr2, Duration::from_secs(3)).unwrap());
    assert!(matches!(
        c.unlock().unwrap(),
        teko_client::UnlockOutcome::Unlocked
    ));

    // 4) the request: the organ checks the device is genuine and holds the identity key
    let nonce = vec![0x77; 32];
    let req = c.identity_request(&nonce, "Órgão emissor").unwrap();
    teko_client::verify_identity_request(&trust, &nonce, &req).unwrap();

    // 5) member A (the device wallet) PROPOSES; the claims are shown and signed on the device
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let claims = civic_claims(
        &CivicSubject {
            name: "Maria da Silva".into(),
            birth_date: "1990-05-17".into(),
            nationality: "BR".into(),
            electoral_state: "SP".into(),
            electoral_municipality: "3550308".into(),
        },
        "TK-REAL0001",
        now,
    )
    .unwrap();
    let salt = [9u8; 16];
    let valid_until = now + 100_000_000;
    let mut world = teko_node::NodeWorld::open(&net).unwrap();
    let tx = c
        .wallet_sign_credential_op(&CredentialOpArgs {
            chain_id: world.chain_id(),
            kind: CredentialOpKind::Propose,
            ctype: CIVIC_TYPE,
            subject: &req.pk_identity,
            claims: claims.as_bytes(),
            public: false,
            salt: Some(&salt),
            commitment: None,
            valid_until,
            reason: "",
            contest_secs: 2,
            fee: 0,
            nonce: world.nonce(&wallet_addr),
        })
        .unwrap();
    world.submit(tx).unwrap();
    drop(world);
    // 1 of 2: nothing can be installed yet
    // 6) the other member approves (simulated)
    let said = devnet_sim::cosign(&net, Some("civic")).unwrap();
    assert!(said.contains("civic"), "{said}");

    // 7) the bundle: claims + salt + the signed transactions found ON THE CHAIN
    let world = teko_node::NodeWorld::open(&net).unwrap();
    let commitment = teko_crypto::civic::commitment_of(&teko_crypto::civic::claim_list(
        &salt,
        claims.as_bytes(),
    ));
    let id = teko_crypto::ledger::cred_id(
        CIVIC_TYPE.as_bytes(),
        &req.pk_identity,
        &commitment,
        &[],
        valid_until,
    );
    let propose = cred_propose_payload(
        CIVIC_TYPE.as_bytes(),
        &req.pk_identity,
        &commitment,
        &[],
        valid_until,
    );
    let approve = cred_vote_payload(CRED_OP_APPROVE, CIVIC_TYPE.as_bytes(), &id, "");
    let (mut proposal, mut approvals) = (None, Vec::<BundleTx>::new());
    for d in world.chain().blocks() {
        for raw in &d.block().unwrap().txs {
            let tx = Tx::from_bytes(raw).unwrap();
            let bt = BundleTx {
                chain_id: tx.chain_id.clone(),
                nonce: tx.nonce,
                fee: tx.fee,
                payload: tx.payload.clone(),
                sender: tx.sender.clone(),
                signature: tx.signature.clone(),
            };
            if tx.payload == propose {
                proposal.get_or_insert(bt);
            } else if tx.payload == approve && !approvals.iter().any(|a| a.sender == bt.sender) {
                approvals.push(bt);
            }
        }
    }
    let mut txs = vec![proposal.expect("the proposal is on the chain")];
    txs.extend(approvals);
    let bundle = CivicBundle {
        chain_id: world.chain_id().to_string(),
        ctype: CIVIC_TYPE.into(),
        claims: claims.clone().into_bytes(),
        salt: salt.to_vec(),
        valid_until,
        txs,
    };
    // the device checks the organ's quorum ALONE and installs it
    c.load_credential(&bundle.encode_to_vec()).unwrap();
    let st = c.features().unwrap();
    assert_eq!(
        (st.state.as_str(), st.has_credential),
        ("credentials_issued", true)
    );
    let policy = CivicPolicy {
        chain_id: trust.civic_chain_id.clone(),
        threshold: trust.civic_threshold,
        members: trust.civic_members.clone(),
    };
    assert_eq!(
        teko_crypto::civic::verify_civic_bundle(&bundle, &policy, &req.pk_identity).unwrap(),
        id
    );

    // 8) a verifier reads the chain: the credential is ACTIVE only after the contest window
    let lookup_at = |t: u64| {
        let w = teko_node::NodeWorld::open(&net).unwrap();
        let rec = teko_credentials::credential(w.ledger_ref(), b"civic", &id).unwrap();
        move |i: &[u8]| {
            (i == id.as_slice()).then(|| CivicRecord {
                subject: rec.subject.clone(),
                commitment: rec.commitment.clone(),
                valid_until: rec.valid_until,
                active: rec.status_at(t) == teko_credentials::Status::Active,
            })
        }
    };
    let age18 = || teko_protocol::Predicate {
        kind: teko_protocol::PredicateKind::AgeAtLeast as i32,
        arg: 18,
        name: String::new(),
    };
    let rec = teko_credentials::credential(world.ledger_ref(), b"civic", &id).unwrap();
    drop(world);
    // inside the window: refused (not active yet)
    let err = c
        .prove_attribute(age18(), "Loja", &lookup_at(rec.approved_at))
        .unwrap_err();
    assert!(err.to_string().contains("NÃO está ativa"), "{err}");
    // after the window: the proof of age verifies against the chain and reveals only that claim
    let proof = c
        .prove_attribute(age18(), "Loja", &lookup_at(rec.active_at))
        .unwrap();
    assert_eq!(
        proof.disclosed,
        vec![("age_over_18".to_string(), "true".to_string())]
    );
    // a credential the chain does not know is refused
    assert!(
        c.prove_attribute(age18(), "Loja", &|_: &[u8]| None)
            .is_err()
    );

    for n in nodes {
        n.stop();
    }
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn restoring_the_wallet_on_a_new_device_recovers_the_credentials_from_the_chain_and_opens_them() {
    use issuer_cli::{Issuer, scratch_dir};
    use teko_assets::devnet::World;
    use teko_client::{CredentialOpArgs, CredentialOpKind, DeviceClient};
    use teko_device_core::testing::ScriptedHal;
    use teko_node::init_economy_with_issuer;
    use teko_transport::connect_tcp;

    let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
    let issuer = Issuer::open(scratch_dir("recover-real"));
    let trust0 = issuer.dev_setup().unwrap();
    let factory = issuer.provision_device().unwrap();
    let d = issuer.dir().to_path_buf();
    let (m, b) = (
        std::fs::read(d.join("firmware-v1.manifest")).unwrap(),
        std::fs::read(d.join("firmware-v1.blob")).unwrap(),
    );
    // device A: the citizen's wallet (also a member of the organ, to issue to itself)
    let hal_a = ScriptedHal::new();
    hal_a.set_default_pin(Some("482913"));
    hal_a.set_auto_words(true);
    let addr_a = spawn_device(&hal_a, trust0.clone(), factory.clone());
    let mut a = DeviceClient::new(connect_tcp(addr_a, Duration::from_secs(3)).unwrap());
    a.install_firmware(&trust0, &m, &b, &mut |_, _| {}).unwrap();
    a.setup_pin().unwrap();
    let wallet = a.wallet_setup().unwrap();
    let wallet_addr = teko_crypto::parse_wallet_address(&wallet.address).unwrap();
    let words = hal_a.shown_words();
    assert_eq!(words.len(), 24);

    let net = d.join("net");
    init_network(&net, NetworkKind::Devnet, 3, free_port_base(), false).unwrap();
    let g = init_economy_with_issuer(
        &net,
        Some(wallet_addr),
        Some(wallet_addr),
        None,
        Some(10_000),
        false,
    )
    .unwrap();
    std::fs::write(net.join(ECONOMY_FILE), g.to_text()).unwrap();
    let ec = load_economy(&net).unwrap().unwrap();
    let nodes: Vec<NodeHandle> = (0..3).map(|i| start_node(&net, i, &ec)).collect();

    // the organ issues a PRIVATE credential to the citizen's wallet-derived credential key
    let holder = a.credential_key("cnh-b").unwrap();
    let claims = "nome=Ana Souza;categoria=B;registro=123456".to_string();
    let salt = [4u8; 16];
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let valid_until = now + 100_000_000;
    let mut world = teko_node::NodeWorld::open(&net).unwrap();
    let tx = a
        .wallet_sign_credential_op(&CredentialOpArgs {
            chain_id: world.chain_id(),
            kind: CredentialOpKind::Propose,
            ctype: "cnh-b",
            subject: &holder,
            claims: claims.as_bytes(),
            public: false,
            salt: Some(&salt),
            commitment: None,
            valid_until,
            reason: "",
            contest_secs: 1_200,
            fee: 0,
            nonce: world.nonce(&wallet_addr),
        })
        .unwrap();
    world.submit(tx).unwrap();
    drop(world);
    devnet_sim::cosign(&net, Some("cnh-b")).unwrap();

    // device A is LOST. Device B is brand new: only the 24 words are typed on it
    let hal_b = ScriptedHal::new();
    hal_b.set_default_pin(Some("482913"));
    let addr_b = spawn_device(&hal_b, trust0.clone(), factory);
    let mut bdev = DeviceClient::new(connect_tcp(addr_b, Duration::from_secs(3)).unwrap());
    bdev.install_firmware(&trust0, &m, &b, &mut |_, _| {})
        .unwrap();
    bdev.setup_pin().unwrap();
    for w in &words {
        hal_b.push_word(Some(w));
    }
    assert_eq!(bdev.wallet_restore().unwrap().address, wallet.address);

    // B derives the SAME credential keys from the wallet and finds the credential on the chain
    let types: Vec<String> = vec!["cnh-a".into(), "cnh-b".into(), "candidato".into()];
    let keys = bdev.credential_keys(&types).unwrap();
    let key_b = keys.iter().find(|(t, _)| t == "cnh-b").unwrap().1.clone();
    assert_eq!(
        key_b, holder,
        "the credential key comes back with the wallet"
    );
    let world = teko_node::NodeWorld::open(&net).unwrap();
    let found = teko_credentials::credentials_of(world.ledger_ref(), &key_b);
    assert_eq!(found.len(), 1);
    let (t, id) = &found[0];
    assert_eq!(t.as_slice(), b"cnh-b");
    let rec = teko_credentials::credential(world.ledger_ref(), t, id).unwrap();
    // the chain has the attributes only SEALED; the device of the wallet opens them
    assert!(!rec.sealed.is_empty());
    assert!(!rec.sealed.windows(8).any(|w| w == b"Ana Souz"));
    let (opened, got_salt) = bdev
        .open_credential_bundle("cnh-b", &key_b, &rec.sealed, &rec.commitment)
        .unwrap();
    assert_eq!(opened, claims.as_bytes());
    assert_eq!(got_salt, salt);
    // the other types hold nothing for this wallet
    for (t, k) in &keys {
        if t != "cnh-b" {
            assert!(
                teko_credentials::credentials_of(world.ledger_ref(), k).is_empty(),
                "{t}"
            );
        }
    }
    // a wallet with ANOTHER passphrase is another wallet: its keys find nothing
    drop(world);
    for n in nodes {
        n.stop();
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn the_block_simulator_makes_the_asked_number_of_blocks_on_both_chains() {
    use devnet_sim::{BlocksKind, blocks};
    let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
    let dir: PathBuf = std::env::temp_dir().join(format!(
        "teko-devnet-blocks-{}-{}",
        std::process::id(),
        free_port_base()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    init_network(&dir, NetworkKind::Devnet, 3, free_port_base(), false).unwrap();
    let owner = address_of(&public_key_bytes(&key(61)));
    let mut g = init_economy(&dir, Some(owner), None, None, false).unwrap();
    g.coin.fee_per_byte = 0;
    std::fs::write(dir.join(ECONOMY_FILE), g.to_text()).unwrap();
    let ec = load_economy(&dir).unwrap().unwrap();
    let nodes: Vec<NodeHandle> = (0..3).map(|i| start_node(&dir, i, &ec)).collect();
    let client = NodeClient::new(load_network(&dir).unwrap().peers);
    let a = client.addrs()[0];

    let e0 = client.econ_status(a).unwrap().height;
    let msg = blocks(&dir, BlocksKind::Economy, 3).unwrap();
    assert!(msg.contains("3 bloco"), "{msg}");
    assert!(client.econ_status(a).unwrap().height >= e0 + 3);

    let v0 = client.status(a).unwrap();
    let msg = blocks(&dir, BlocksKind::Votes, 2).unwrap();
    assert!(msg.contains("2 bloco"), "{msg}");
    let v1 = client.status(a).unwrap();
    assert!(v1.height >= v0.height + 2);
    assert!(
        v1.log_size >= v0.log_size + 2,
        "the filler entries are on the board"
    );

    assert!(blocks(&dir, BlocksKind::Economy, 0).is_err());
    assert!(blocks(&dir, BlocksKind::Economy, 1001).is_err());
    drop(nodes);
    let _ = std::fs::remove_dir_all(&dir);
}
