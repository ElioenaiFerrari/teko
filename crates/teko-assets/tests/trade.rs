//! Direct trade with a veto window (ADR-0036): each party sends ITS OWN transaction, the price
//! waits in an escrow, the organ may veto inside the window, and anyone finalizes after it.
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
const META: [u8; 32] = [7; 32];
const CLASS: &[u8] = b"imovel";
const WINDOW: u64 = 1_200;
const ID: &[u8] = b"casa-1";

struct W {
    l: Ledger,
    regs: Vec<SigningKey>,
    now: u64,
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
        cap: 10_000_000,
        initial_reward: 0,
        halving_interval: 10,
        ..Default::default()
    };
    let alloc: Vec<(Address, u64)> = funded.iter().map(|(k, v)| (addr(k), *v)).collect();
    teko_coin::genesis(&mut l, cfg, &alloc, true).unwrap();
    let keys: Vec<Vec<u8>> = regs.iter().map(public_key_bytes).collect();
    genesis_class_with_window(&mut l, CLASS, &keys, 2, WINDOW).unwrap();
    W {
        l,
        regs,
        now: 1_000_000,
    }
}

/// Sends `op` as `sender` in a block at `at` (the block time modules see).
fn send(w: &mut W, sender: &SigningKey, op: Op, at: u64) -> Result<(), String> {
    let nonce = w.l.nonce_of(&addr(sender));
    let t = Tx::new_signed(sender, CHAIN, nonce, ASSETS_MODULE, op.encode());
    let h = w.l.height() + 1;
    match w.l.apply_block_at(h, at, &P, &[t]) {
        Ok(_) => {
            w.now = w.now.max(at);
            Ok(())
        }
        Err(LedgerError::Tx {
            error: TxError::Module(m),
            ..
        }) => Err(m),
        Err(e) => Err(format!("{e:?}")),
    }
}

fn sig(w: &W, who: &[usize], i: &Intent) -> Vec<Approval> {
    who.iter()
        .map(|n| Approval::sign(&w.regs[*n], CHAIN, i))
        .collect()
}

/// The organ registers `ID` to `owner` (a relayer sends the transaction).
fn register(w: &mut W, owner: &SigningKey) {
    let i = Intent::register(CLASS, ID, addr(owner), &META);
    let approvals = sig(w, &[0, 1], &i);
    let relayer = key(777);
    let at = w.now;
    send(
        w,
        &relayer,
        Op::Register(RegisterOp {
            class: CLASS.to_vec(),
            asset: ID.to_vec(),
            owner: addr(owner).to_vec(),
            metadata_hash: META.to_vec(),
            approvals,
        }),
        at,
    )
    .unwrap();
}

fn version(w: &W) -> u64 {
    asset(&w.l, CLASS, ID).unwrap().version
}

fn offer(
    w: &mut W,
    seller: &SigningKey,
    buyer: &SigningKey,
    price: u64,
    at: u64,
) -> Result<(), String> {
    let v = version(w);
    send(
        w,
        seller,
        Op::Offer(OfferOp {
            class: CLASS.to_vec(),
            asset: ID.to_vec(),
            buyer: addr(buyer).to_vec(),
            price,
            version: v,
            metadata_hash: META.to_vec(),
        }),
        at,
    )
}

/// The buyer accepts the terms of the offer as it is NOW (what a careful client shows the buyer).
fn accept(w: &mut W, buyer: &SigningKey, at: u64) -> Result<(), String> {
    let o = offer_of(&w.l, CLASS, ID).unwrap_or_default();
    send(
        w,
        buyer,
        Op::Accept(AcceptOp {
            class: CLASS.to_vec(),
            asset: ID.to_vec(),
            price: o.price,
            version: o.version,
            metadata_hash: o.metadata_hash,
        }),
        at,
    )
}

fn finalize(w: &mut W, who: &SigningKey, at: u64) -> Result<(), String> {
    send(
        w,
        who,
        Op::Finalize(FinalizeOp {
            class: CLASS.to_vec(),
            asset: ID.to_vec(),
        }),
        at,
    )
}

fn cancel(w: &mut W, who: &SigningKey, at: u64) -> Result<(), String> {
    send(
        w,
        who,
        Op::Cancel(CancelOp {
            class: CLASS.to_vec(),
            asset: ID.to_vec(),
        }),
        at,
    )
}

fn veto(w: &mut W, who: &[usize], reason: &str, at: u64) -> Result<(), String> {
    let a = asset(&w.l, CLASS, ID).unwrap();
    let o = offer_of(&w.l, CLASS, ID).unwrap();
    let i = Intent::veto(
        CLASS,
        ID,
        a.owner.as_slice().try_into().unwrap(),
        o.buyer.as_slice().try_into().unwrap(),
        o.price,
        a.version,
        reason,
    );
    let approvals = sig(w, who, &i);
    let relayer = key(777);
    send(
        w,
        &relayer,
        Op::Veto(VetoOp {
            class: CLASS.to_vec(),
            asset: ID.to_vec(),
            reason: reason.to_string(),
            approvals,
        }),
        at,
    )
}

fn bal(w: &W, a: &Address) -> u64 {
    teko_coin::balance(&w.l, a)
}

#[test]
fn offer_accept_and_finalize_move_the_asset_and_the_money_only_after_the_window() {
    let (seller, buyer, stranger) = (key(1), key(2), key(3));
    let mut w = world(&[(&buyer, 1_000), (&stranger, 10)]);
    register(&mut w, &seller);
    let escrow = escrow_address(CLASS, ID);

    offer(&mut w, &seller, &buyer, 400, 2_000).unwrap();
    // the offer alone moves nothing
    assert_eq!(
        asset(&w.l, CLASS, ID).unwrap().owner,
        addr(&seller).to_vec()
    );
    assert_eq!(bal(&w, &addr(&buyer)), 1_000);

    accept(&mut w, &buyer, 2_010).unwrap();
    // the price left the buyer and waits in the escrow; the window is the class's, from the accept
    assert_eq!(bal(&w, &addr(&buyer)), 600);
    assert_eq!(bal(&w, &escrow), 400);
    let o = offer_of(&w.l, CLASS, ID).unwrap();
    assert!(o.accepted);
    assert_eq!(o.release_at, 2_010 + WINDOW);
    assert_eq!(
        asset(&w.l, CLASS, ID).unwrap().owner,
        addr(&seller).to_vec(),
        "still the seller's"
    );

    // one second early: refused, and nothing changed
    assert_eq!(
        finalize(&mut w, &stranger, 2_010 + WINDOW - 1),
        Err("veto window still open".into())
    );
    assert_eq!(bal(&w, &escrow), 400);
    // at the end of the window ANYONE may finalize (here, a stranger)
    finalize(&mut w, &stranger, 2_010 + WINDOW).unwrap();
    let a = asset(&w.l, CLASS, ID).unwrap();
    assert_eq!(a.owner, addr(&buyer).to_vec());
    assert_eq!(a.version, 1, "the version moved, so old consents are dead");
    assert_eq!(bal(&w, &addr(&seller)), 400);
    assert_eq!(bal(&w, &escrow), 0, "the escrow is empty again");
    assert!(offer_of(&w.l, CLASS, ID).is_none());
    teko_coin::audit(&w.l).unwrap();
    // and it cannot be finalized twice
    assert_eq!(
        finalize(&mut w, &stranger, 9_999_999),
        Err("no offer for this asset".into())
    );
}

#[test]
fn only_the_owner_offers_only_the_named_buyer_accepts_and_stale_offers_are_refused() {
    let (seller, buyer, other, thief) = (key(1), key(2), key(3), key(4));
    let mut w = world(&[(&buyer, 1_000), (&other, 1_000)]);
    register(&mut w, &seller);
    // not the owner
    assert_eq!(
        offer(&mut w, &thief, &buyer, 10, 2_000),
        Err("only the owner can offer".into())
    );
    // not to oneself
    assert_eq!(
        offer(&mut w, &seller, &seller, 10, 2_000),
        Err("seller and buyer are the same".into())
    );
    // the offer is bound to the asset's version and metadata hash
    let bad_version = Op::Offer(OfferOp {
        class: CLASS.to_vec(),
        asset: ID.to_vec(),
        buyer: addr(&buyer).to_vec(),
        price: 10,
        version: 5,
        metadata_hash: META.to_vec(),
    });
    assert_eq!(
        send(&mut w, &seller, bad_version, 2_000),
        Err("offer is for another version of the asset".into())
    );
    let bad_meta = Op::Offer(OfferOp {
        class: CLASS.to_vec(),
        asset: ID.to_vec(),
        buyer: addr(&buyer).to_vec(),
        price: 10,
        version: 0,
        metadata_hash: vec![9; 32],
    });
    assert_eq!(
        send(&mut w, &seller, bad_meta, 2_000),
        Err("offer is for another asset record".into())
    );

    offer(&mut w, &seller, &buyer, 100, 2_000).unwrap();
    // one offer per asset
    assert_eq!(
        offer(&mut w, &seller, &other, 100, 2_001),
        Err("the asset already has an offer".into())
    );
    // only the named buyer accepts
    assert_eq!(
        accept(&mut w, &other, 2_002),
        Err("only the named buyer can accept".into())
    );
    assert_eq!(
        accept(&mut w, &seller, 2_002),
        Err("only the named buyer can accept".into())
    );
    accept(&mut w, &buyer, 2_003).unwrap();
    assert_eq!(
        accept(&mut w, &buyer, 2_004),
        Err("the offer was already accepted".into())
    );
}

#[test]
fn an_acceptance_binds_the_terms_the_buyer_saw_so_a_cancel_and_re_offer_cannot_hijack_it() {
    let (seller, buyer) = (key(1), key(2));
    let mut w = world(&[(&buyer, 5_000)]);
    register(&mut w, &seller);
    offer(&mut w, &seller, &buyer, 400, 2_000).unwrap();
    // the buyer's client showed price 400 and the buyer signed an acceptance of exactly that...
    let seen = Op::Accept(AcceptOp {
        class: CLASS.to_vec(),
        asset: ID.to_vec(),
        price: 400,
        version: 0,
        metadata_hash: META.to_vec(),
    });
    // ...but before it lands the owner cancels and re-offers the asset at 4000
    cancel(&mut w, &seller, 2_001).unwrap();
    offer(&mut w, &seller, &buyer, 4_000, 2_002).unwrap();
    assert_eq!(
        send(&mut w, &buyer, seen, 2_003),
        Err("the offer terms changed: review and accept again".into())
    );
    assert_eq!(bal(&w, &addr(&buyer)), 5_000, "nothing was charged");
    assert!(!offer_of(&w.l, CLASS, ID).unwrap().accepted);
}

#[test]
fn accepting_without_the_money_fails_and_leaves_the_offer_open() {
    let (seller, buyer) = (key(1), key(2));
    let mut w = world(&[(&buyer, 50)]);
    register(&mut w, &seller);
    offer(&mut w, &seller, &buyer, 400, 2_000).unwrap();
    assert!(
        accept(&mut w, &buyer, 2_001)
            .unwrap_err()
            .contains("escrow payment failed")
    );
    let o = offer_of(&w.l, CLASS, ID).unwrap();
    assert!(!o.accepted, "nothing was half done");
    assert_eq!(bal(&w, &addr(&buyer)), 50);
    assert_eq!(bal(&w, &escrow_address(CLASS, ID)), 0);
}

#[test]
fn the_seller_can_cancel_only_before_the_buyer_accepts() {
    let (seller, buyer) = (key(1), key(2));
    let mut w = world(&[(&buyer, 1_000)]);
    register(&mut w, &seller);
    offer(&mut w, &seller, &buyer, 400, 2_000).unwrap();
    assert_eq!(
        cancel(&mut w, &buyer, 2_001),
        Err("only the owner can cancel".into())
    );
    cancel(&mut w, &seller, 2_002).unwrap();
    assert!(offer_of(&w.l, CLASS, ID).is_none());
    // a new offer is possible, and once accepted it can no longer be cancelled
    offer(&mut w, &seller, &buyer, 400, 2_003).unwrap();
    accept(&mut w, &buyer, 2_004).unwrap();
    assert_eq!(
        cancel(&mut w, &seller, 2_005),
        Err("the offer was already accepted: only the organ can veto".into())
    );
}

#[test]
fn the_organ_quorum_vetoes_inside_the_window_the_buyer_is_refunded_and_the_asset_stays() {
    let (seller, buyer) = (key(1), key(2));
    let mut w = world(&[(&buyer, 1_000)]);
    register(&mut w, &seller);
    offer(&mut w, &seller, &buyer, 400, 2_000).unwrap();
    // nothing to veto before the buyer accepted
    assert_eq!(
        veto(&mut w, &[0, 1], "fraude", 2_001),
        Err("nothing pending to veto".into())
    );
    accept(&mut w, &buyer, 2_010).unwrap();
    // one member is not a quorum
    assert_eq!(
        veto(&mut w, &[0], "fraude", 2_020),
        Err("registrar quorum missing".into())
    );
    // the quorum vetoes with a public reason
    veto(&mut w, &[0, 2], "penhora judicial nao registrada", 2_030).unwrap();
    assert_eq!(
        asset(&w.l, CLASS, ID).unwrap().owner,
        addr(&seller).to_vec()
    );
    assert_eq!(
        bal(&w, &addr(&buyer)),
        1_000,
        "the buyer got the whole price back"
    );
    assert_eq!(bal(&w, &escrow_address(CLASS, ID)), 0);
    assert!(offer_of(&w.l, CLASS, ID).is_none());
    teko_coin::audit(&w.l).unwrap();
}

#[test]
fn a_veto_after_the_window_or_for_another_offer_is_refused() {
    let (seller, buyer) = (key(1), key(2));
    let mut w = world(&[(&buyer, 1_000)]);
    register(&mut w, &seller);
    offer(&mut w, &seller, &buyer, 400, 2_000).unwrap();
    accept(&mut w, &buyer, 2_010).unwrap();
    // the window is over: the organ lost its chance (anyone may finalize now)
    assert_eq!(
        veto(&mut w, &[0, 1], "tarde", 2_010 + WINDOW),
        Err("the veto window is over".into())
    );
    // a quorum signature for a DIFFERENT reason does not authorize this reason
    let a = asset(&w.l, CLASS, ID).unwrap();
    let i = Intent::veto(
        CLASS,
        ID,
        a.owner.as_slice().try_into().unwrap(),
        addr(&buyer),
        400,
        a.version,
        "outro motivo",
    );
    let approvals = sig(&w, &[0, 1], &i);
    let relayer = key(777);
    let r = send(
        &mut w,
        &relayer,
        Op::Veto(VetoOp {
            class: CLASS.to_vec(),
            asset: ID.to_vec(),
            reason: "motivo diferente".into(),
            approvals,
        }),
        2_100,
    );
    assert_eq!(r, Err("registrar quorum missing".into()));
    // still pending, still finalizable
    finalize(&mut w, &buyer, 2_010 + WINDOW).unwrap();
}

#[test]
fn any_change_to_the_asset_cancels_a_pending_offer_and_refunds_the_buyer() {
    let (seller, buyer) = (key(1), key(2));
    let mut w = world(&[(&buyer, 1_000)]);
    register(&mut w, &seller);
    offer(&mut w, &seller, &buyer, 400, 2_000).unwrap();
    accept(&mut w, &buyer, 2_010).unwrap();
    // the organ places a lien during the window (it bumps the version)
    let a = asset(&w.l, CLASS, ID).unwrap();
    let i = Intent::add_lien(
        CLASS,
        ID,
        a.owner.as_slice().try_into().unwrap(),
        a.version,
        "penhora",
    );
    let approvals = sig(&w, &[0, 1], &i);
    let relayer = key(777);
    send(
        &mut w,
        &relayer,
        Op::AddLien(LienOp {
            class: CLASS.to_vec(),
            asset: ID.to_vec(),
            kind: "penhora".into(),
            approvals,
        }),
        2_020,
    )
    .unwrap();
    assert!(
        offer_of(&w.l, CLASS, ID).is_none(),
        "the stale offer is gone"
    );
    assert_eq!(
        bal(&w, &addr(&buyer)),
        1_000,
        "and the buyer's money is back"
    );
    assert_eq!(bal(&w, &escrow_address(CLASS, ID)), 0);
    // while the lien exists nobody can offer the asset
    assert_eq!(
        offer(&mut w, &seller, &buyer, 400, 2_030),
        Err("asset has active liens".into())
    );
    teko_coin::audit(&w.l).unwrap();
}

#[test]
fn after_the_organ_releases_the_lien_the_asset_can_be_offered_again() {
    let (seller, buyer) = (key(1), key(2));
    let mut w = world(&[(&buyer, 1_000)]);
    register(&mut w, &seller);
    let a = asset(&w.l, CLASS, ID).unwrap();
    let owner: Address = a.owner.as_slice().try_into().unwrap();
    let relayer = key(777);
    let add = Intent::add_lien(CLASS, ID, owner, a.version, "penhora");
    let approvals = sig(&w, &[0, 1], &add);
    send(
        &mut w,
        &relayer,
        Op::AddLien(LienOp {
            class: CLASS.to_vec(),
            asset: ID.to_vec(),
            kind: "penhora".into(),
            approvals,
        }),
        2_000,
    )
    .unwrap();
    let a = asset(&w.l, CLASS, ID).unwrap();
    let rel = Intent::release_lien(CLASS, ID, owner, a.version, a.liens[0].id);
    let approvals = sig(&w, &[1, 2], &rel);
    send(
        &mut w,
        &relayer,
        Op::ReleaseLien(ReleaseOp {
            class: CLASS.to_vec(),
            asset: ID.to_vec(),
            lien_id: a.liens[0].id,
            approvals,
        }),
        2_010,
    )
    .unwrap();
    offer(&mut w, &seller, &buyer, 100, 2_020).unwrap();
}

#[test]
fn each_class_has_its_own_window_and_a_zero_window_is_refused() {
    let regs: Vec<Vec<u8>> = (0..3).map(|i| public_key_bytes(&key(1000 + i))).collect();
    let mut l = Ledger::new(CHAIN, vec![teko_coin::module(), module()]).unwrap();
    genesis_class_with_window(&mut l, b"imovel", &regs, 2, 3_600).unwrap();
    genesis_class_with_window(&mut l, b"veiculo", &regs, 2, 60).unwrap();
    assert_eq!(class_info(&l, b"imovel").unwrap().window_secs, 3_600);
    assert_eq!(class_info(&l, b"veiculo").unwrap().window_secs, 60);
    // a class without a window would let anybody finalize at once, defeating the veto
    assert!(genesis_class_with_window(&mut l, b"x", &regs, 2, 0).is_err());
    // the old constructor keeps a sane default (20 minutes)
    genesis_class(&mut l, b"y", &regs, 2).unwrap();
    assert_eq!(class_info(&l, b"y").unwrap().window_secs, 1_200);
}

#[test]
fn the_payloads_the_device_builds_are_byte_for_byte_what_the_module_decodes() {
    use teko_crypto::ledger::{asset_accept_payload, asset_offer_payload};
    // zero values (price 0, version 0) are omitted by protobuf: the device must omit them too
    for (price, version) in [
        (0u64, 0u64),
        (1, 0),
        (400, 7),
        (u64::MAX, u64::MAX),
        (128, 300),
    ] {
        let device = asset_offer_payload(CLASS, ID, &[9; 32], price, version, &META);
        let op = Op::Offer(OfferOp {
            class: CLASS.to_vec(),
            asset: ID.to_vec(),
            buyer: vec![9; 32],
            price,
            version,
            metadata_hash: META.to_vec(),
        });
        assert_eq!(device, op.encode(), "offer {price}/{version}");
        assert_eq!(Op::decode(&device), Some(op));
        let device = asset_accept_payload(CLASS, ID, price, version, &META);
        let op = Op::Accept(AcceptOp {
            class: CLASS.to_vec(),
            asset: ID.to_vec(),
            price,
            version,
            metadata_hash: META.to_vec(),
        });
        assert_eq!(device, op.encode(), "accept {price}/{version}");
        assert_eq!(Op::decode(&device), Some(op));
    }
}

#[test]
fn the_new_operations_round_trip_and_reject_garbage() {
    let ops = vec![
        Op::Offer(OfferOp {
            class: CLASS.to_vec(),
            asset: ID.to_vec(),
            buyer: vec![1; 32],
            price: 5,
            version: 2,
            metadata_hash: META.to_vec(),
        }),
        Op::Accept(AcceptOp {
            class: CLASS.to_vec(),
            asset: ID.to_vec(),
            price: 5,
            version: 2,
            metadata_hash: META.to_vec(),
        }),
        Op::Cancel(CancelOp {
            class: CLASS.to_vec(),
            asset: ID.to_vec(),
        }),
        Op::Finalize(FinalizeOp {
            class: CLASS.to_vec(),
            asset: ID.to_vec(),
        }),
        Op::Veto(VetoOp {
            class: CLASS.to_vec(),
            asset: ID.to_vec(),
            reason: "x".into(),
            approvals: Vec::new(),
        }),
    ];
    for op in ops {
        assert_eq!(Op::decode(&op.encode()), Some(op.clone()));
        let mut extra = op.encode();
        extra.push(0);
        assert_eq!(Op::decode(&extra), None, "trailing bytes are refused");
    }
    // the old consent-based transfer and swap no longer exist
    assert_eq!(Op::decode(&[2, 0]), None);
    assert_eq!(Op::decode(&[3, 0]), None);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(40))]

    /// Whatever the order of offers, accepts, cancels, vetoes and finalizations: coins and assets are conserved
    /// (the escrow included) and an asset always has exactly one owner.
    #[test]
    fn money_and_ownership_are_conserved_under_any_sequence(
        steps in proptest::collection::vec((0u8..6, 0u64..3000), 1..30)
    ) {
        let (seller, buyer, stranger) = (key(1), key(2), key(3));
        let mut w = world(&[(&buyer, 1_000), (&seller, 500), (&stranger, 100)]);
        register(&mut w, &seller);
        let total_before: u64 = [&seller, &buyer, &stranger].iter().map(|k| bal(&w, &addr(k))).sum();
        let mut t = 2_000u64;
        for (op, dt) in steps {
            t += dt;
            let owner_is_seller = asset(&w.l, CLASS, ID).unwrap().owner == addr(&seller).to_vec();
            let (a, b) = if owner_is_seller { (&seller, &buyer) } else { (&buyer, &seller) };
            let _ = match op {
                0 => offer(&mut w, a, b, 100 + dt % 200, t),
                1 => accept(&mut w, b, t),
                2 => cancel(&mut w, a, t),
                3 => finalize(&mut w, &stranger, t),
                4 if offer_of(&w.l, CLASS, ID).is_some() => veto(&mut w, &[0, 1], "teste", t),
                _ => Ok(()),
            };
            let escrow = bal(&w, &escrow_address(CLASS, ID));
            let total: u64 = [&seller, &buyer, &stranger].iter().map(|k| bal(&w, &addr(k))).sum::<u64>() + escrow;
            prop_assert_eq!(total, total_before, "coins are conserved, escrow included");
            prop_assert!(teko_coin::audit(&w.l).is_ok());
            let owners = assets_of(&w.l, &addr(&seller)).len() + assets_of(&w.l, &addr(&buyer)).len();
            prop_assert_eq!(owners, 1, "exactly one owner at every moment");
            if offer_of(&w.l, CLASS, ID).is_none() {
                prop_assert_eq!(escrow, 0, "no offer, no money in the escrow");
            }
        }
    }
}
