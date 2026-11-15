#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use proptest::prelude::*;
use rand::SeedableRng;
use rand::rngs::StdRng;
use teko_assets::*;
use teko_coin::{COIN_MODULE, CoinConfig};
use teko_crypto::{SigningKey, generate_signing_key, public_key_bytes};
use teko_ledger::{Address, Ledger, LedgerError, Tx, TxError, address_of};

const CHAIN: &str = "teko-devnet";
const P: Address = [9; 32];
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
    let i = Intent::register(CLASS, id, addr(owner), b"meta");
    let ks: Vec<&SigningKey> = with.iter().map(|n| &w.regs[*n]).collect();
    let approvals = sig(&ks, &i);
    run(
        w,
        Op::Register(RegisterOp {
            class: CLASS.to_vec(),
            asset: id.to_vec(),
            owner: addr(owner).to_vec(),
            metadata_hash: b"meta".to_vec(),
            approvals,
        }),
    )
}

fn version(w: &W, id: &[u8]) -> u64 {
    asset(&w.l, CLASS, id).unwrap().version
}

fn transfer(
    w: &mut W,
    from: &SigningKey,
    to: &SigningKey,
    id: &[u8],
    signers: &[&SigningKey],
) -> Result<(), String> {
    let i = Intent::transfer(CLASS, id, addr(from), addr(to), version(w, id));
    let approvals = sig(signers, &i);
    run(
        w,
        Op::Transfer(TransferOp {
            class: CLASS.to_vec(),
            asset: id.to_vec(),
            to: addr(to).to_vec(),
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
    let i = Intent::register(CLASS, b"casa-1", addr(&owner), b"meta");
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
            metadata_hash: b"meta".to_vec(),
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
            metadata_hash: b"meta".to_vec(),
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
            metadata_hash: vec![],
            approvals: vec![],
        }),
    );
    assert_eq!(r, Err("unknown class".into()));
}

#[test]
fn transfer_needs_owner_buyer_and_registrar() {
    let (seller, buyer) = (key(1), key(2));
    let mut w = world(&[]);
    register(&mut w, &seller, b"casa", &[0, 1]).unwrap();
    let (r0, r1) = (w.regs[0].clone(), w.regs[1].clone());
    // missing the buyer
    assert_eq!(
        transfer(&mut w, &seller, &buyer, b"casa", &[&seller, &r0, &r1]),
        Err("buyer consent missing".into())
    );
    // missing the owner
    assert_eq!(
        transfer(&mut w, &seller, &buyer, b"casa", &[&buyer, &r0, &r1]),
        Err("owner consent missing".into())
    );
    // missing the registrar quorum
    assert_eq!(
        transfer(&mut w, &seller, &buyer, b"casa", &[&seller, &buyer, &r0]),
        Err("registrar quorum missing".into())
    );
    // a stranger's signature is not the owner's
    let thief = key(9);
    assert_eq!(
        transfer(
            &mut w,
            &seller,
            &buyer,
            b"casa",
            &[&thief, &buyer, &r0, &r1]
        ),
        Err("owner consent missing".into())
    );
    assert_eq!(
        asset(&w.l, CLASS, b"casa").unwrap().owner,
        addr(&seller).to_vec()
    );
    // all three: works, even though a RELAYER submitted the transaction
    transfer(
        &mut w,
        &seller,
        &buyer,
        b"casa",
        &[&seller, &buyer, &r0, &r1],
    )
    .unwrap();
    assert_eq!(
        asset(&w.l, CLASS, b"casa").unwrap().owner,
        addr(&buyer).to_vec()
    );
    assert!(assets_of(&w.l, &addr(&seller)).is_empty());
    assert_eq!(assets_of(&w.l, &addr(&buyer)).len(), 1);
}

#[test]
fn consents_cannot_be_replayed_after_a_change() {
    let (a, b, c) = (key(1), key(2), key(3));
    let mut w = world(&[]);
    register(&mut w, &a, b"casa", &[0, 1]).unwrap();
    let (r0, r1) = (w.regs[0].clone(), w.regs[1].clone());
    let old = Intent::transfer(CLASS, b"casa", addr(&a), addr(&b), version(&w, b"casa"));
    let stale = sig(&[&a, &b, &r0, &r1], &old);
    transfer(&mut w, &a, &b, b"casa", &[&a, &b, &r0, &r1]).unwrap();
    // b sells back to a, then the old (a -> b) approvals are submitted again
    transfer(&mut w, &b, &a, b"casa", &[&a, &b, &r0, &r1]).unwrap();
    let r = run(
        &mut w,
        Op::Transfer(TransferOp {
            class: CLASS.to_vec(),
            asset: b"casa".to_vec(),
            to: addr(&b).to_vec(),
            approvals: stale,
        }),
    );
    assert_eq!(r, Err("owner consent missing".into())); // version moved on
    // approvals for another chain do not count either
    let other = Intent::transfer(CLASS, b"casa", addr(&a), addr(&c), version(&w, b"casa"));
    let foreign: Vec<Approval> = [&a, &c, &r0, &r1]
        .iter()
        .map(|k| Approval::sign(k, "other-chain", &other))
        .collect();
    let r = run(
        &mut w,
        Op::Transfer(TransferOp {
            class: CLASS.to_vec(),
            asset: b"casa".to_vec(),
            to: addr(&c).to_vec(),
            approvals: foreign,
        }),
    );
    assert_eq!(r, Err("owner consent missing".into()));
}

#[test]
fn a_transfer_consent_cannot_be_used_as_a_swap() {
    let (a, b) = (key(1), key(2));
    let mut w = world(&[(&b, 5000)]);
    register(&mut w, &a, b"casa", &[0, 1]).unwrap();
    let (r0, r1) = (w.regs[0].clone(), w.regs[1].clone());
    let as_transfer = Intent::transfer(CLASS, b"casa", addr(&a), addr(&b), 0);
    let approvals = sig(&[&a, &b, &r0, &r1], &as_transfer);
    let r = run(
        &mut w,
        Op::Swap(SwapOp {
            class: CLASS.to_vec(),
            asset: b"casa".to_vec(),
            buyer: addr(&b).to_vec(),
            price: 1,
            approvals,
        }),
    );
    assert_eq!(r, Err("owner consent missing".into()));
}

#[test]
fn liens_block_transfers_until_released() {
    let (a, b) = (key(1), key(2));
    let mut w = world(&[]);
    register(&mut w, &a, b"casa", &[0, 1]).unwrap();
    let (r0, r1) = (w.regs[0].clone(), w.regs[1].clone());
    let i = Intent::add_lien(CLASS, b"casa", addr(&a), version(&w, b"casa"), "penhora");
    run(
        &mut w,
        Op::AddLien(LienOp {
            class: CLASS.to_vec(),
            asset: b"casa".to_vec(),
            kind: "penhora".into(),
            approvals: sig(&[&r0, &r1], &i),
        }),
    )
    .unwrap();
    assert_eq!(asset(&w.l, CLASS, b"casa").unwrap().liens.len(), 1);
    assert_eq!(
        transfer(&mut w, &a, &b, b"casa", &[&a, &b, &r0, &r1]),
        Err("asset has active liens".into())
    );
    // the owner cannot add or release liens alone
    let rel = Intent::release_lien(CLASS, b"casa", addr(&a), version(&w, b"casa"), 0);
    let r = run(
        &mut w,
        Op::ReleaseLien(ReleaseOp {
            class: CLASS.to_vec(),
            asset: b"casa".to_vec(),
            lien_id: 0,
            approvals: sig(&[&a], &rel),
        }),
    );
    assert_eq!(r, Err("registrar quorum missing".into()));
    run(
        &mut w,
        Op::ReleaseLien(ReleaseOp {
            class: CLASS.to_vec(),
            asset: b"casa".to_vec(),
            lien_id: 0,
            approvals: sig(&[&r0, &r1], &rel),
        }),
    )
    .unwrap();
    transfer(&mut w, &a, &b, b"casa", &[&a, &b, &r0, &r1]).unwrap();
}

fn swap(w: &mut W, a: &SigningKey, b: &SigningKey, id: &[u8], price: u64) -> Result<(), String> {
    let (r0, r1) = (w.regs[0].clone(), w.regs[1].clone());
    let i = Intent::swap(CLASS, id, addr(a), addr(b), price, version(w, id));
    run(
        w,
        Op::Swap(SwapOp {
            class: CLASS.to_vec(),
            asset: id.to_vec(),
            buyer: addr(b).to_vec(),
            price,
            approvals: sig(&[a, b, &r0, &r1], &i),
        }),
    )
}

#[test]
fn swap_is_atomic_delivery_versus_payment() {
    let (seller, buyer, poor) = (key(1), key(2), key(3));
    let mut w = world(&[(&buyer, 5000), (&poor, 10)]);
    register(&mut w, &seller, b"casa", &[0, 1]).unwrap();
    // the buyer cannot pay: nothing moves
    assert!(
        swap(&mut w, &seller, &poor, b"casa", 3000)
            .unwrap_err()
            .starts_with("payment failed")
    );
    assert_eq!(
        asset(&w.l, CLASS, b"casa").unwrap().owner,
        addr(&seller).to_vec()
    );
    assert_eq!(teko_coin::balance(&w.l, &addr(&poor)), 10);
    // the buyer pays: asset and coin move together
    swap(&mut w, &seller, &buyer, b"casa", 3000).unwrap();
    assert_eq!(
        asset(&w.l, CLASS, b"casa").unwrap().owner,
        addr(&buyer).to_vec()
    );
    assert_eq!(teko_coin::balance(&w.l, &addr(&seller)), 3000);
    assert_eq!(teko_coin::balance(&w.l, &addr(&buyer)), 2000);
    teko_coin::audit(&w.l).unwrap();
    // the price is part of the consent: signing for 3000 does not authorise 1
    let (r0, r1) = (w.regs[0].clone(), w.regs[1].clone());
    let i = Intent::swap(
        CLASS,
        b"casa",
        addr(&buyer),
        addr(&seller),
        3000,
        version(&w, b"casa"),
    );
    let r = run(
        &mut w,
        Op::Swap(SwapOp {
            class: CLASS.to_vec(),
            asset: b"casa".to_vec(),
            buyer: addr(&seller).to_vec(),
            price: 1,
            approvals: sig(&[&buyer, &seller, &r0, &r1], &i),
        }),
    );
    assert_eq!(r, Err("owner consent missing".into()));
    // free "swap" and self-swap are refused
    assert_eq!(
        swap(&mut w, &buyer, &seller, b"casa", 0),
        Err("price must be positive".into())
    );
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

proptest! {
    /// Random swaps and transfers keep every asset with exactly one owner, keep the
    /// owner index consistent and conserve the coin supply.
    #[test]
    fn ownership_and_supply_invariants(steps in proptest::collection::vec((0usize..3, 0usize..3, 0usize..3, 0u8..2, 1u64..400), 1..25)) {
        let people: Vec<SigningKey> = (0..3).map(|i| key(500 + i)).collect();
        let funded: Vec<(&SigningKey, u64)> = people.iter().map(|p| (p, 1000)).collect();
        let mut w = world(&funded);
        let ids: [&[u8]; 3] = [b"a1", b"a2", b"a3"];
        for (n, id) in ids.iter().enumerate() {
            register(&mut w, &people[n % 3], id, &[0, 1]).unwrap();
        }
        for (from, to, which, kind, price) in steps {
            let id = ids[which];
            let owner_now = asset(&w.l, CLASS, id).unwrap().owner;
            let seller = people.iter().find(|p| addr(p).to_vec() == owner_now).unwrap().clone();
            let buyer = &people[to];
            let _ = from;
            if addr(&seller) == addr(buyer) { continue; }
            let (r0, r1) = (w.regs[0].clone(), w.regs[1].clone());
            let _ = if kind == 0 {
                transfer(&mut w, &seller, buyer, id, &[&seller, buyer, &r0, &r1])
            } else {
                swap(&mut w, &seller, buyer, id, price)
            };
            teko_coin::audit(&w.l).unwrap();
            let mut count = 0;
            for p in &people {
                for (c, a) in assets_of(&w.l, &addr(p)) {
                    count += 1;
                    prop_assert_eq!(asset(&w.l, &c, &a).unwrap().owner, addr(p).to_vec());
                }
            }
            prop_assert_eq!(count, 3);
        }
    }
}
