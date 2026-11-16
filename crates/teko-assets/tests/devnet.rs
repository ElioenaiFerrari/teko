#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;

use rand::SeedableRng;
use rand::rngs::StdRng;
use teko_assets::devnet::{self, OwnedAsset, TradeAction, TradeRequest, sample_record};
use teko_assets::{ASSETS_MODULE, AcceptOp, OfferOp, Op, asset, offer_of};
use teko_coin::devnet::{ECONOMY_FILE, Economy, append_allocation};
use teko_crypto::asset_meta;
use teko_crypto::{SigningKey, generate_signing_key, public_key_bytes, wallet_address};
use teko_ledger::{Address, Tx, address_of};

fn dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("teko-assets-dev-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn person(seed: u64) -> (SigningKey, Address) {
    let k = generate_signing_key(&mut StdRng::seed_from_u64(seed));
    let a = address_of(&public_key_bytes(&k));
    (k, a)
}

/// A software "device": builds and signs the citizen's OWN transaction from the request fields (the
/// metadata hash comes from the record it is handed), as the real device does.
fn soft<'a>(key: &'a SigningKey) -> impl FnMut(&TradeRequest<'_>) -> Result<Tx, String> + 'a {
    move |r| {
        let op = match r.action {
            TradeAction::Offer => Op::Offer(OfferOp {
                class: r.class.into(),
                asset: r.asset.as_bytes().to_vec(),
                buyer: r.buyer.to_vec(),
                price: r.price,
                version: r.version,
                metadata_hash: asset_meta::hash(r.class, r.metadata).to_vec(),
            }),
            TradeAction::Accept => Op::Accept(AcceptOp {
                class: r.class.into(),
                asset: r.asset.as_bytes().to_vec(),
                price: r.price,
                version: r.version,
                metadata_hash: asset_meta::hash(r.class, r.metadata).to_vec(),
            }),
        };
        Ok(Tx::new_signed_with_fee(
            key,
            r.chain_id,
            r.nonce,
            r.fee,
            ASSETS_MODULE,
            op.encode(),
        ))
    }
}

/// Moves this machine's clock past the veto window of the pending trade, so it can be finalized.
fn after_the_window(econ: &mut Economy, class: &[u8], id: &[u8]) {
    let release = offer_of(econ.ledger(), class, id).unwrap().release_at;
    econ.set_clock(Box::new(move || release + 5));
}

fn house(id: &str) -> Vec<u8> {
    sample_record("imovel", id).unwrap()
}

fn owned(id: &str, version: u64, liens: usize) -> OwnedAsset {
    OwnedAsset {
        class: "imovel".into(),
        id: id.into(),
        version,
        liens,
    }
}

#[test]
fn a_trade_is_pending_in_the_window_and_completes_only_after_it() {
    let d = dir("a");
    let (me_key, me) = person(1);
    append_allocation(&d.join(ECONOMY_FILE), &wallet_address(&me).unwrap(), 10_000).unwrap();
    let (econ, cast) = devnet::open(&d).unwrap();
    // the counterparty needs coins to buy: fund it in the allocations
    drop(econ);
    let cp = cast.counterparty_address();
    append_allocation(&d.join(ECONOMY_FILE), &wallet_address(&cp).unwrap(), 6_000).unwrap();
    let (mut econ, cast) = devnet::open(&d).unwrap();

    // the registrar registers an asset to me
    devnet::register(&mut econ, &cast, &me, "imovel", &house("casa-1")).unwrap();
    assert_eq!(devnet::owned(&econ, &me), vec![owned("casa-1", 0, 0)]);
    assert!(devnet::register(&mut econ, &cast, &me, "imovel", &house("casa-1")).is_err()); // already registered

    // a lien blocks the sale; releasing it allows it
    devnet::set_lien(&mut econ, &cast, "imovel", "casa-1", true).unwrap();
    let mut dev = soft(&me_key);
    let r = devnet::sell_to_counterparty(&mut econ, &cast, &me, "imovel", "casa-1", 4000, &mut dev);
    assert!(r.unwrap_err().contains("gravame"));
    devnet::set_lien(&mut econ, &cast, "imovel", "casa-1", false).unwrap();
    assert!(devnet::set_lien(&mut econ, &cast, "imovel", "casa-1", false).is_err()); // no lien left

    // I sell: MY device signs the offer from MY wallet (so I pay its fee); the counterparty accepts
    let cp_before = econ.balance(&cp);
    let me_before = econ.balance(&me);
    devnet::sell_to_counterparty(&mut econ, &cast, &me, "imovel", "casa-1", 4000, &mut dev)
        .unwrap();
    // the trade is PENDING: nothing moved yet, the buyer's money waits in the escrow
    let trades = devnet::pending_trades(&econ, &me);
    assert_eq!(trades.len(), 1);
    assert!(trades[0].accepted && trades[0].release_at > econ.last_time());
    let before = econ.balance(&me);
    assert!(
        before < me_before,
        "the offer left from MY wallet and I paid its fee"
    );
    assert_eq!(
        devnet::owned(&econ, &me).len(),
        1,
        "still mine during the window"
    );
    assert!(
        econ.balance(&cp) <= cp_before - 4_000,
        "the price waits in the escrow (and the buyer paid a fee)"
    );
    // finalizing inside the window is refused
    let r = devnet::finalize_pending(&mut econ, &cast, "imovel", "casa-1");
    assert!(r.unwrap_err().contains("veto window still open"));
    // after the window ANYONE may finalize
    after_the_window(&mut econ, b"imovel", b"casa-1");
    devnet::finalize_pending(&mut econ, &cast, "imovel", "casa-1").unwrap();
    assert_eq!(econ.balance(&me), before + 4000);
    assert!(devnet::owned(&econ, &me).is_empty());
    assert_eq!(
        asset(econ.ledger(), b"imovel", b"casa-1").unwrap().owner,
        cp.to_vec()
    );
    assert!(devnet::pending_trades(&econ, &me).is_empty());
    // selling something that is not mine, or a price of zero, is refused
    assert!(
        devnet::sell_to_counterparty(&mut econ, &cast, &me, "imovel", "casa-1", 1, &mut dev)
            .unwrap_err()
            .contains("dono")
    );
    assert!(
        devnet::sell_to_counterparty(&mut econ, &cast, &me, "imovel", "casa-1", 0, &mut dev)
            .is_err()
    );

    // I buy it back: the counterparty offers, MY device signs the acceptance
    let me_before_buy = econ.balance(&me);
    devnet::buy_from_counterparty(&mut econ, &cast, &me, "imovel", "casa-1", 3000, &mut dev)
        .unwrap();
    assert!(
        econ.balance(&me) <= me_before_buy - 3000,
        "my price is in the escrow"
    );
    after_the_window(&mut econ, b"imovel", b"casa-1");
    devnet::finalize_pending(&mut econ, &cast, "imovel", "casa-1").unwrap();
    assert_eq!(devnet::owned(&econ, &me).len(), 1);
    // the buyer without enough coins cannot buy
    devnet::register(&mut econ, &cast, &cp, "imovel", &house("casa-2")).unwrap();
    let r =
        devnet::buy_from_counterparty(&mut econ, &cast, &me, "imovel", "casa-2", 999_999, &mut dev);
    assert!(r.unwrap_err().contains("saldo"));
    teko_coin::audit(econ.ledger()).unwrap();

    // everything survives a restart, block times included
    let spent = econ.balance(&me);
    let (again, _) = devnet::open(&d).unwrap();
    assert_eq!(devnet::owned(&again, &me).len(), 1);
    assert_eq!(again.balance(&me), spent);
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn the_organ_can_veto_a_pending_trade_and_everyone_gets_back_what_is_theirs() {
    let d = dir("veto");
    let (me_key, me) = person(7);
    let (_, cast0) = devnet::open(&d).unwrap();
    let cp = cast0.counterparty_address();
    append_allocation(&d.join(ECONOMY_FILE), &wallet_address(&cp).unwrap(), 5_000).unwrap();
    append_allocation(&d.join(ECONOMY_FILE), &wallet_address(&me).unwrap(), 5_000).unwrap(); // pays its own fees
    let (mut econ, cast) = devnet::open(&d).unwrap();
    devnet::register(&mut econ, &cast, &me, "imovel", &house("casa-v")).unwrap();
    let mut dev = soft(&me_key);
    devnet::sell_to_counterparty(&mut econ, &cast, &me, "imovel", "casa-v", 2_000, &mut dev)
        .unwrap();
    let cp_in_escrow = econ.balance(&cp);
    assert!(cp_in_escrow <= 3_000);
    // nothing pending is refused to veto; the pending one is vetoed with a public reason
    devnet::veto_pending(
        &mut econ,
        &cast,
        "imovel",
        "casa-v",
        "penhora judicial nao registrada",
    )
    .unwrap();
    assert_eq!(
        econ.balance(&cp),
        cp_in_escrow + 2_000,
        "the buyer got the whole price back"
    );
    assert_eq!(
        devnet::owned(&econ, &me).len(),
        1,
        "the asset never left the owner"
    );
    assert!(devnet::pending_trades(&econ, &me).is_empty());
    assert!(devnet::veto_pending(&mut econ, &cast, "imovel", "casa-v", "x").is_err());
    teko_coin::audit(econ.ledger()).unwrap();
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn if_i_reject_the_acceptance_the_counterpartys_offer_is_withdrawn_and_nothing_stays_blocked() {
    let d = dir("rej");
    let (me_key, me) = person(9);
    let (_, cast0) = devnet::open(&d).unwrap();
    let cp = cast0.counterparty_address();
    append_allocation(&d.join(ECONOMY_FILE), &wallet_address(&me).unwrap(), 5_000).unwrap();
    append_allocation(&d.join(ECONOMY_FILE), &wallet_address(&cp).unwrap(), 5_000).unwrap(); // pays its own fees
    let (mut econ, cast) = devnet::open(&d).unwrap();
    devnet::register(&mut econ, &cast, &cp, "imovel", &house("casa-r")).unwrap();
    // my device refuses to sign the acceptance
    let mut refusing =
        |_: &TradeRequest<'_>| -> Result<Tx, String> { Err("rejeitado no dispositivo".into()) };
    let r = devnet::buy_from_counterparty(
        &mut econ,
        &cast,
        &me,
        "imovel",
        "casa-r",
        1_000,
        &mut refusing,
    );
    assert!(r.unwrap_err().contains("rejeitado"));
    assert!(
        offer_of(econ.ledger(), b"imovel", b"casa-r").is_none(),
        "no dead offer is left"
    );
    assert_eq!(econ.balance(&me), 5_000, "I paid nothing");
    // and a later, real purchase works normally
    let mut dev = soft(&me_key);
    devnet::buy_from_counterparty(&mut econ, &cast, &me, "imovel", "casa-r", 1_000, &mut dev)
        .unwrap();
    assert_eq!(devnet::pending_trades(&econ, &me).len(), 1);
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn an_offer_signed_by_someone_else_does_not_move_anything() {
    let d = dir("b");
    let (_me_key, me) = person(2);
    let (other_key, _) = person(3);
    let (econ, cast) = devnet::open(&d).unwrap();
    drop(econ);
    append_allocation(
        &d.join(ECONOMY_FILE),
        &wallet_address(&cast.counterparty_address()).unwrap(),
        9000,
    )
    .unwrap();
    let (mut econ, cast) = devnet::open(&d).unwrap();
    devnet::register(&mut econ, &cast, &me, "imovel", &house("casa-9")).unwrap();
    // somebody else's key signs "my" offer: the ledger refuses (only the owner can offer)
    let mut dev = soft(&other_key);
    let r = devnet::sell_to_counterparty(&mut econ, &cast, &me, "imovel", "casa-9", 500, &mut dev);
    assert!(r.is_err());
    assert_eq!(devnet::owned(&econ, &me).len(), 1);
    assert!(devnet::pending_trades(&econ, &me).is_empty());
    let _ = fs::remove_dir_all(&d);
}

fn vehicle(n: u32) -> Vec<u8> {
    let f = asset_meta::sample("veiculo", n).unwrap();
    let p: Vec<(&str, &str)> = f.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    asset_meta::canonical("veiculo", &p).unwrap()
}

#[test]
fn a_vehicle_is_registered_by_its_own_organ_with_the_chassis_as_id() {
    let d = dir("v");
    let (me_key, me) = person(5);
    let (_, cast0) = devnet::open(&d).unwrap();
    let cp = cast0.counterparty_address();
    append_allocation(
        &d.join(ECONOMY_FILE),
        &wallet_address(&cp).unwrap(),
        9_000_000,
    )
    .unwrap();
    append_allocation(&d.join(ECONOMY_FILE), &wallet_address(&me).unwrap(), 5_000).unwrap(); // pays its own fees
    let (mut econ, cast) = devnet::open(&d).unwrap();

    let rec = vehicle(4);
    let id = devnet::register(&mut econ, &cast, &me, "veiculo", &rec).unwrap();
    assert_eq!(id, "9BWZZZ377VT000004"); // the chassis
    assert_eq!(
        devnet::owned(&econ, &me),
        vec![OwnedAsset {
            class: "veiculo".into(),
            id: id.clone(),
            version: 0,
            liens: 0
        }]
    );
    // the chain stores only the hash; the record is in the off-chain store
    let on_chain = asset(econ.ledger(), b"veiculo", id.as_bytes()).unwrap();
    assert_eq!(
        on_chain.metadata_hash,
        asset_meta::hash("veiculo", &rec).to_vec()
    );
    assert_eq!(
        cast.store
            .get_verified("veiculo", &id, &on_chain.metadata_hash),
        Some(rec.clone())
    );
    // the same chassis cannot be registered twice
    assert!(
        devnet::register(&mut econ, &cast, &me, "veiculo", &rec)
            .unwrap_err()
            .contains("já está registrado")
    );
    // the órgão of one class cannot sign for the other: the vehicle órgão's keys differ
    let imovel_keys = cast.registrars_for("imovel").unwrap();
    let veiculo_keys = cast.registrars_for("veiculo").unwrap();
    assert_ne!(
        public_key_bytes(&imovel_keys[0]),
        public_key_bytes(&veiculo_keys[0])
    );

    // sell it to the counterparty: the "device" is handed the record and signs its hash
    let mut dev = soft(&me_key);
    devnet::sell_to_counterparty(&mut econ, &cast, &me, "veiculo", &id, 5_000_000, &mut dev)
        .unwrap();
    after_the_window(&mut econ, b"veiculo", id.as_bytes());
    devnet::finalize_pending(&mut econ, &cast, "veiculo", &id).unwrap();
    assert!(devnet::owned(&econ, &me).is_empty());
    assert_eq!(
        asset(econ.ledger(), b"veiculo", id.as_bytes())
            .unwrap()
            .owner,
        cp.to_vec()
    );
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn a_sale_is_blocked_when_the_metadata_record_is_missing_or_does_not_match() {
    let d = dir("m");
    let (me_key, me) = person(6);
    let (_, cast0) = devnet::open(&d).unwrap();
    append_allocation(
        &d.join(ECONOMY_FILE),
        &wallet_address(&cast0.counterparty_address()).unwrap(),
        9_000_000,
    )
    .unwrap();
    let (mut econ, cast) = devnet::open(&d).unwrap();
    let id = devnet::register(&mut econ, &cast, &me, "veiculo", &vehicle(8)).unwrap();
    let mut dev = soft(&me_key);

    // another computer without the record: blocked
    let empty = cast_without_records(&d);
    let r = devnet::sell_to_counterparty(&mut econ, &empty, &me, "veiculo", &id, 100, &mut dev);
    assert!(r.unwrap_err().contains("metadados"));

    // a record for the same chassis with other content does not match the chain hash
    let other = {
        let mut f = asset_meta::parse("veiculo", &vehicle(8)).unwrap();
        f[7].1 = "Preto".into(); // another colour
        let p: Vec<(&str, &str)> = f.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        asset_meta::canonical("veiculo", &p).unwrap()
    };
    let swapped = cast_without_records(&d);
    swapped.store.put("veiculo", &other).unwrap();
    let r = devnet::sell_to_counterparty(&mut econ, &swapped, &me, "veiculo", &id, 100, &mut dev);
    assert!(r.unwrap_err().contains("metadados"));
    assert_eq!(devnet::owned(&econ, &me).len(), 1, "nothing moved");
    assert!(devnet::pending_trades(&econ, &me).is_empty());
    let _ = fs::remove_dir_all(&d);
}

fn cast_without_records(d: &std::path::Path) -> devnet::Cast {
    let empty = d.join("empty-store");
    let _ = fs::remove_dir_all(&empty);
    devnet::Cast::load(d).unwrap().with_store(empty)
}
