//! The civil-registry credentials (ADR-0047) on REAL nodes: a marriage is issued to BOTH spouses
//! (private, never expiring), a divorce is issued to both and the organ REVOKES the marriage
//! credentials, so the marriage stops being valid and the divorce stays as the proof.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::TcpListener;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use rand::SeedableRng;
use teko_chain::NetworkKind;
use teko_credentials::civil::{
    DIVORCE_TYPE, Divorce, Forma, MARRIAGE_TYPE, Marriage, Regime, divorce_claims, divorce_ends,
    marriage_claims, parse_divorce, parse_marriage,
};
use teko_credentials::{CREDENTIALS_MODULE, Op, ProposeOp, Status, VoteOp, credential_id};
use teko_crypto::{SigningKey, generate_signing_key, public_key_bytes};
use teko_ledger::{Tx, address_of};
use teko_node::{
    ECONOMY_FILE, EconomyConfig, Node, NodeClient, NodeConfig, NodeHandle,
    init_economy_with_issuer, init_network, load_economy, load_network, load_validator_key,
    node_dir,
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
fn a_divorce_is_issued_to_both_spouses_and_the_organ_revokes_the_marriage_on_real_nodes() {
    let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
    let issuer: SigningKey = key(77);
    let issuer_addr = address_of(&public_key_bytes(&issuer));
    let dir: PathBuf = std::env::temp_dir().join(format!(
        "teko-devnet-civil-{}-{}",
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
    for t in &mut g.credtypes {
        if t.name == MARRIAGE_TYPE || t.name == DIVORCE_TYPE {
            // both are PRIVATE and never expire; the contest window is shortened for the test
            assert!(!t.public && t.validity_secs == 0);
            t.contest_secs = 1;
        }
    }
    std::fs::write(dir.join(ECONOMY_FILE), g.to_text()).unwrap();
    let ec = load_economy(&dir).unwrap().unwrap();
    let nodes: Vec<NodeHandle> = (0..3).map(|i| start_node(&dir, i, &ec)).collect();
    let client = NodeClient::new(load_network(&dir).unwrap().peers);
    let chain_id = ec.genesis.chain_id.clone();
    let mut nonce = 0u64;
    let mut send = |ops: Vec<Op>| {
        let txs: Vec<Tx> = ops
            .into_iter()
            .map(|op| {
                nonce += 1;
                Tx::new_signed(
                    &issuer,
                    &chain_id,
                    nonce - 1,
                    CREDENTIALS_MODULE,
                    op.encode(),
                )
            })
            .collect();
        client.econ_submit(&txs).unwrap();
    };
    let state = |ctype: &str, id: &[u8]| {
        let c = client.econ_sync(&ec.genesis).ok()?;
        teko_credentials::credential(c.ledger(), ctype.as_bytes(), id).map(|r| r.status_at(now()))
    };
    // PRIVATE credentials: the chain keeps only the commitment, the holders keep the claims
    let propose = |ctype: &str, holder: &[u8], claims: &str, salt: [u8; 16]| {
        let commitment =
            teko_crypto::ledger::cred_private_commitment(&salt, claims.as_bytes()).to_vec();
        let id = credential_id(ctype.as_bytes(), holder, &commitment, &[], 0);
        let op = Op::Propose(ProposeOp {
            ctype: ctype.as_bytes().to_vec(),
            subject: holder.to_vec(),
            commitment,
            claims: vec![],
            valid_until: 0,
            sealed: vec![],
        });
        (id.to_vec(), op)
    };
    let (ana, beto) = (public_key_bytes(&key(8_001)), public_key_bytes(&key(8_002)));
    let wedding = |nome: &str, conjuge: &str| Marriage {
        matricula: "2020.0145.77".into(),
        data: "2020-02-29".into(),
        regime: Regime::ComunhaoParcial,
        nome: nome.into(),
        conjuge: conjuge.into(),
        cartorio: "1º Ofício de Registro Civil de São Paulo".into(),
    };
    let split = |nome: &str, conjuge: &str| Divorce {
        matricula: "2020.0145.77".into(),
        data: "2024-05-10".into(),
        forma: Forma::Extrajudicial,
        nome: nome.into(),
        conjuge: conjuge.into(),
        cartorio: "1º Ofício de Registro Civil de São Paulo".into(),
    };

    // 1) the marriage: one credential to EACH spouse, the same registry number
    let (m_ana, m_beto) = (
        wedding("Ana Souza", "Beto Lima"),
        wedding("Beto Lima", "Ana Souza"),
    );
    let (id_ma, op_ma) = propose(
        MARRIAGE_TYPE,
        &ana,
        &marriage_claims(&m_ana).unwrap(),
        [1; 16],
    );
    let (id_mb, op_mb) = propose(
        MARRIAGE_TYPE,
        &beto,
        &marriage_claims(&m_beto).unwrap(),
        [2; 16],
    );
    send(vec![op_ma, op_mb]);
    wait("the marriage proposals are in a block", 30, || {
        state(MARRIAGE_TYPE, &id_ma).is_some() && state(MARRIAGE_TYPE, &id_mb).is_some()
    });
    // the issuer alone is 1 of 2; the simulated members complete the quorum
    devnet_sim::cosign(&dir, Some(MARRIAGE_TYPE)).unwrap();
    wait("the marriage is valid", 40, || {
        state(MARRIAGE_TYPE, &id_ma) == Some(Status::Active)
            && state(MARRIAGE_TYPE, &id_mb) == Some(Status::Active)
    });

    // 2) the divorce: a credential to each former spouse, and the marriage credentials are REVOKED
    let (d_ana, d_beto) = (
        split("Ana Souza", "Beto Lima"),
        split("Beto Lima", "Ana Souza"),
    );
    assert!(divorce_ends(&m_ana, &d_ana).is_ok() && divorce_ends(&m_beto, &d_beto).is_ok());
    let (id_da, op_da) = propose(
        DIVORCE_TYPE,
        &ana,
        &divorce_claims(&d_ana).unwrap(),
        [3; 16],
    );
    let (id_db, op_db) = propose(
        DIVORCE_TYPE,
        &beto,
        &divorce_claims(&d_beto).unwrap(),
        [4; 16],
    );
    send(vec![op_da, op_db]);
    wait("the divorce proposals are in a block", 30, || {
        state(DIVORCE_TYPE, &id_da).is_some() && state(DIVORCE_TYPE, &id_db).is_some()
    });
    devnet_sim::cosign(&dir, Some(DIVORCE_TYPE)).unwrap();
    wait("the divorce is valid", 40, || {
        state(DIVORCE_TYPE, &id_da) == Some(Status::Active)
            && state(DIVORCE_TYPE, &id_db) == Some(Status::Active)
    });
    let revoke = |id: &[u8]| {
        Op::Revoke(VoteOp {
            ctype: MARRIAGE_TYPE.as_bytes().to_vec(),
            id: id.to_vec(),
            reason: "divórcio".into(),
        })
    };
    send(vec![revoke(&id_ma), revoke(&id_mb)]);
    wait("the revocations started", 30, || {
        let c = client.econ_sync(&ec.genesis).unwrap();
        [&id_ma, &id_mb].iter().all(|id| {
            teko_credentials::credential(c.ledger(), MARRIAGE_TYPE.as_bytes(), id)
                .is_some_and(|r| r.revoke_votes.len() == 1)
        })
    });
    // one vote of two: still valid until the quorum completes
    assert_eq!(state(MARRIAGE_TYPE, &id_ma), Some(Status::Active));
    let said = devnet_sim::cosign(&dir, Some(MARRIAGE_TYPE)).unwrap();
    assert!(said.contains("revogou"), "{said}");
    wait("the marriage is revoked", 40, || {
        state(MARRIAGE_TYPE, &id_ma) == Some(Status::Revoked)
            && state(MARRIAGE_TYPE, &id_mb) == Some(Status::Revoked)
    });
    // the divorce stays as the proof
    assert_eq!(state(DIVORCE_TYPE, &id_da), Some(Status::Active));
    assert_eq!(state(DIVORCE_TYPE, &id_db), Some(Status::Active));
    // what the holders keep (the claims) is verifiable offline against their credentials
    assert!(parse_marriage(marriage_claims(&m_ana).unwrap().as_bytes()).is_some());
    assert!(parse_divorce(divorce_claims(&d_ana).unwrap().as_bytes()).is_some());

    for n in nodes {
        n.stop();
    }
    let _ = std::fs::remove_dir_all(&dir);
}
