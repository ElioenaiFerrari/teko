#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rand::SeedableRng;
use rand::rngs::StdRng;
use teko_assets::*;
use teko_coin::{COIN_MODULE, CoinConfig};
use teko_crypto::{SigningKey, generate_signing_key, public_key_bytes};
use teko_ledger::{Address, Ledger, LedgerError, Tx, TxError, address_of};

const CHAIN: &str = "teko-devnet";
const P: Address = [9; 32];
/// Stand-in for the hash of a metadata record (32 bytes).
const META: [u8; 32] = [7; 32];
const CLASS: &[u8] = b"imovel";

struct W {
    l: Ledger,
    regs: Vec<SigningKey>, // 2-of-3 registrar
}

fn key(seed: u64) -> SigningKey {
    generate_signing_key(&mut StdRng::seed_from_u64(seed))
}
fn addr(k: &SigningKey) -> Address {
    address_of(&public_key_bytes(k))
}

fn world(funded: &[(&SigningKey, u64)]) -> W {
    let regs: Vec<SigningKey> = (0..3).map(|i| key(1000 + i)).collect();
    let mut l = Ledger::new(CHAIN, vec![teko_coin::module(), module()])
        .unwrap()
        .with_fee_module(COIN_MODULE)
        .unwrap();
    let cfg = CoinConfig {
        cap: 1_000_000,
        initial_reward: 0,
        halving_interval: 10,
        fee_per_byte: 0,
        unbonding_period: 5,
        slash_bps: 1000,
        faucet_amount: 0,
        faucet_cooldown: 0,
        ..Default::default()
    };
    let alloc: Vec<(Address, u64)> = funded.iter().map(|(k, v)| (addr(k), *v)).collect();
    teko_coin::genesis(&mut l, cfg, &alloc, true).unwrap();
    let keys: Vec<Vec<u8>> = regs.iter().map(public_key_bytes).collect();
    genesis_class(&mut l, CLASS, &keys, 2).unwrap();
    W { l, regs }
}

fn relayer_tx(l: &Ledger, op: Op) -> Tx {
    let r = key(777);
    Tx::new_signed(&r, CHAIN, l.nonce_of(&addr(&r)), ASSETS_MODULE, op.encode())
}

fn run(w: &mut W, op: Op) -> Result<(), String> {
    let t = relayer_tx(&w.l, op);
    let h = w.l.height() + 1;
    match w.l.apply_block(h, &P, &[t]) {
        Ok(_) => Ok(()),
        Err(LedgerError::Tx {
            error: TxError::Module(m),
            ..
        }) => Err(m),
        Err(e) => Err(format!("{e:?}")),
    }
}

fn sig(keys: &[&SigningKey], i: &Intent) -> Vec<Approval> {
    keys.iter().map(|k| Approval::sign(k, CHAIN, i)).collect()
}

fn register(w: &mut W, owner: &SigningKey, id: &[u8], with: &[usize]) -> Result<(), String> {
    let i = Intent::register(CLASS, id, addr(owner), &META);
    let ks: Vec<&SigningKey> = with.iter().map(|n| &w.regs[*n]).collect();
    let approvals = sig(&ks, &i);
    run(
        w,
        Op::Register(RegisterOp {
            class: CLASS.to_vec(),
            asset: id.to_vec(),
            owner: addr(owner).to_vec(),
            metadata_hash: META.to_vec(),
            approvals,
        }),
    )
}

#[test]
fn registration_needs_the_registrar_quorum() {
    let owner = key(1);
    let mut w = world(&[]);
    assert_eq!(
        register(&mut w, &owner, b"casa-1", &[0]),
        Err("registrar quorum missing".into())
    );
    // the same member signing twice still counts once
    let i = Intent::register(CLASS, b"casa-1", addr(&owner), &META);
    let two = vec![
        Approval::sign(&w.regs[0], CHAIN, &i),
        Approval::sign(&w.regs[0], CHAIN, &i),
    ];
    let r = run(
        &mut w,
        Op::Register(RegisterOp {
            class: CLASS.to_vec(),
            asset: b"casa-1".to_vec(),
            owner: addr(&owner).to_vec(),
            metadata_hash: META.to_vec(),
            approvals: two,
        }),
    );
    assert_eq!(r, Err("registrar quorum missing".into()));
    // a non-registrar cannot complete the quorum
    let outsider = key(50);
    let approvals = sig(&[&w.regs[0], &outsider], &i);
    let r = run(
        &mut w,
        Op::Register(RegisterOp {
            class: CLASS.to_vec(),
            asset: b"casa-1".to_vec(),
            owner: addr(&owner).to_vec(),
            metadata_hash: META.to_vec(),
            approvals,
        }),
    );
    assert_eq!(r, Err("registrar quorum missing".into()));
    register(&mut w, &owner, b"casa-1", &[0, 2]).unwrap();
    assert_eq!(
        assets_of(&w.l, &addr(&owner)),
        vec![(CLASS.to_vec(), b"casa-1".to_vec())]
    );
    assert_eq!(
        register(&mut w, &owner, b"casa-1", &[0, 1]),
        Err("asset already registered".into())
    );
    assert!(register(&mut w, &owner, b"x", &[0, 1]).is_ok());
}

#[test]
fn unknown_class_is_refused() {
    let owner = key(1);
    let mut w = world(&[]);
    let r = run(
        &mut w,
        Op::Register(RegisterOp {
            class: b"nave".to_vec(),
            asset: b"a".to_vec(),
            owner: addr(&owner).to_vec(),
            metadata_hash: META.to_vec(),
            approvals: vec![],
        }),
    );
    assert_eq!(r, Err("unknown class".into()));
}

#[test]
fn registrars_can_be_rotated_only_by_the_current_quorum() {
    let owner = key(1);
    let mut w = world(&[]);
    let new_keys: Vec<SigningKey> = (0..2).map(|i| key(2000 + i)).collect();
    let pubs: Vec<Vec<u8>> = new_keys.iter().map(public_key_bytes).collect();
    let i = Intent::update_registrars(CLASS, 0, &pubs, 2);
    let (o0, o1) = (w.regs[0].clone(), w.regs[1].clone());
    let one = sig(&[&o0], &i);
    let both = sig(&[&o0, &o1], &i);
    // one old member is not enough
    let r = run(
        &mut w,
        Op::UpdateRegistrars(RegistrarsOp {
            class: CLASS.to_vec(),
            registrars: pubs.clone(),
            threshold: 2,
            approvals: one,
        }),
    );
    assert_eq!(r, Err("registrar quorum missing".into()));
    run(
        &mut w,
        Op::UpdateRegistrars(RegistrarsOp {
            class: CLASS.to_vec(),
            registrars: pubs,
            threshold: 2,
            approvals: both,
        }),
    )
    .unwrap();
    assert_eq!(class_info(&w.l, CLASS).unwrap().version, 1);
    // the old registrars no longer count; the new ones do
    assert_eq!(
        register(&mut w, &owner, b"a", &[0, 1]),
        Err("registrar quorum missing".into())
    );
    w.regs = new_keys;
    register(&mut w, &owner, b"a", &[0, 1]).unwrap();
}

#[test]
fn bad_genesis_classes_and_payloads_are_refused() {
    let mut l = Ledger::new(CHAIN, vec![module()]).unwrap();
    let k = public_key_bytes(&key(1));
    assert!(genesis_class(&mut l, b"x", &[], 1).is_err());
    assert!(genesis_class(&mut l, b"x", std::slice::from_ref(&k), 2).is_err());
    assert!(genesis_class(&mut l, b"x", &[k.clone(), k.clone()], 1).is_err());
    assert!(genesis_class(&mut l, b"", std::slice::from_ref(&k), 1).is_err());
    assert!(genesis_class(&mut l, b"x", &[vec![1, 2, 3]], 1).is_err());
    let mut w = world(&[]);
    let t = Tx::new_signed(&key(5), CHAIN, 0, ASSETS_MODULE, vec![9, 9]);
    assert!(matches!(
        w.l.apply_block(1, &P, &[t]),
        Err(LedgerError::Tx { .. })
    ));
}

#[test]
fn registration_needs_a_32_byte_metadata_hash() {
    let owner = key(1);
    let mut w = world(&[]);
    for bad in [vec![], b"meta".to_vec(), vec![1u8; 33]] {
        let i = Intent::register(CLASS, b"casa-1", addr(&owner), &bad);
        let approvals = sig(&[&w.regs[0].clone(), &w.regs[1].clone()], &i);
        let r = run(
            &mut w,
            Op::Register(RegisterOp {
                class: CLASS.to_vec(),
                asset: b"casa-1".to_vec(),
                owner: addr(&owner).to_vec(),
                metadata_hash: bad,
                approvals,
            }),
        );
        assert_eq!(r, Err("metadata hash must be 32 bytes".into()));
    }
}
