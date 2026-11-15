#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;

use rand::SeedableRng;
use rand::rngs::StdRng;
use teko_assets::devnet::{self, ConsentRequest, OwnedAsset, sample_record};
use teko_assets::{Approval, Intent, asset};
use teko_coin::devnet::{CHAIN_ID, ECONOMY_FILE, append_allocation};
use teko_crypto::asset_meta;
use teko_crypto::{SigningKey, generate_signing_key, public_key_bytes, wallet_address};
use teko_ledger::{Address, address_of};

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

/// A software "device": signs the consent of `key` for the swap it is asked about,
/// over the hash of the metadata record it is handed (as the real device does).
fn soft<'a>(
    key: &'a SigningKey,
) -> impl FnMut(&ConsentRequest<'_>) -> Result<Approval, String> + 'a {
    move |r| {
        assert_eq!(r.kind, 3);
        let i = Intent::swap(
            r.class.as_bytes(),
            r.asset.as_bytes(),
            *r.seller,
            *r.buyer,
            r.price,
            r.version,
            &asset_meta::hash(r.class, r.metadata),
        );
        Ok(Approval::sign(key, CHAIN_ID, &i))
    }
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
fn the_simulated_registrar_and_counterparty_trade_with_me() {
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
    let mut c = soft(&me_key);
    let r = devnet::trade_with_counterparty(
        &mut econ, &cast, &me, "imovel", "casa-1", 4000, true, &mut c,
    );
    assert!(r.unwrap_err().contains("gravame"));
    devnet::set_lien(&mut econ, &cast, "imovel", "casa-1", false).unwrap();
    assert!(devnet::set_lien(&mut econ, &cast, "imovel", "casa-1", false).is_err()); // no lien left

    // I sell to the counterparty: my consent signed by "my device"
    let before = econ.balance(&me);
    let after = devnet::trade_with_counterparty(
        &mut econ, &cast, &me, "imovel", "casa-1", 4000, true, &mut c,
    )
    .unwrap();
    assert_eq!(after, before + 4000);
    assert!(devnet::owned(&econ, &me).is_empty());
    assert_eq!(
        asset(econ.ledger(), b"imovel", b"casa-1").unwrap().owner,
        cp.to_vec()
    );
    // selling something that is not mine, or a price of zero, is refused
    assert!(
        devnet::trade_with_counterparty(&mut econ, &cast, &me, "imovel", "casa-1", 1, true, &mut c)
            .unwrap_err()
            .contains("não é seu")
    );
    assert!(
        devnet::trade_with_counterparty(
            &mut econ, &cast, &me, "imovel", "casa-1", 0, false, &mut c
        )
        .is_err()
    );

    // I buy it back
    let spent = devnet::trade_with_counterparty(
        &mut econ, &cast, &me, "imovel", "casa-1", 3000, false, &mut c,
    )
    .unwrap();
    assert_eq!(spent, after - 3000);
    assert_eq!(devnet::owned(&econ, &me).len(), 1);
    // the buyer without enough coins cannot buy
    devnet::register(&mut econ, &cast, &cp, "imovel", &house("casa-2")).unwrap();
    let r = devnet::trade_with_counterparty(
        &mut econ, &cast, &me, "imovel", "casa-2", 999_999, false, &mut c,
    );
    assert!(r.unwrap_err().contains("saldo"));
    teko_coin::audit(econ.ledger()).unwrap();

    // everything survives a restart
    let (again, _) = devnet::open(&d).unwrap();
    assert_eq!(devnet::owned(&again, &me).len(), 1);
    assert_eq!(again.balance(&me), spent);
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn a_consent_from_the_wrong_person_does_not_move_the_asset() {
    let d = dir("b");
    let (_me_key, me) = person(2);
    let (other_key, _) = person(3);
    let (mut econ, cast) = devnet::open(&d).unwrap();
    append_allocation(
        &d.join(ECONOMY_FILE),
        &wallet_address(&cast.counterparty_address()).unwrap(),
        9000,
    )
    .unwrap();
    let (mut econ2, cast) = devnet::open(&d).unwrap();
    drop(econ);
    econ = {
        devnet::register(&mut econ2, &cast, &me, "imovel", &house("casa-9")).unwrap();
        econ2
    };
    // somebody else's key signs "my" consent: the ledger refuses and nothing moves
    let mut c = soft(&other_key);
    let r = devnet::trade_with_counterparty(
        &mut econ, &cast, &me, "imovel", "casa-9", 500, true, &mut c,
    );
    assert!(r.is_err());
    assert_eq!(devnet::owned(&econ, &me).len(), 1);
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
    // an invalid record never reaches the chain
    let mut bad = asset_meta::parse("veiculo", &rec).unwrap();
    bad[2].1 = "12345678901".into();
    let p: Vec<(&str, &str)> = bad.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    assert!(asset_meta::canonical("veiculo", &p).is_err());

    // the órgão of one class cannot sign for the other: the vehicle órgão's keys differ
    let imovel_keys = cast.registrars_for("imovel").unwrap();
    let veiculo_keys = cast.registrars_for("veiculo").unwrap();
    assert_ne!(
        public_key_bytes(&imovel_keys[0]),
        public_key_bytes(&veiculo_keys[0])
    );

    // sell it to the counterparty: the "device" is handed the record and signs its hash
    let mut c = soft(&me_key);
    devnet::trade_with_counterparty(
        &mut econ, &cast, &me, "veiculo", &id, 5_000_000, true, &mut c,
    )
    .unwrap();
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
    let mut c = soft(&me_key);

    // another computer without the record: blocked
    let empty = cast_without_records(&d);
    let r =
        devnet::trade_with_counterparty(&mut econ, &empty, &me, "veiculo", &id, 100, true, &mut c);
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
    let r = devnet::trade_with_counterparty(
        &mut econ, &swapped, &me, "veiculo", &id, 100, true, &mut c,
    );
    assert!(r.unwrap_err().contains("metadados"));
    assert_eq!(devnet::owned(&econ, &me).len(), 1, "nothing moved");
    let _ = fs::remove_dir_all(&d);
}

fn cast_without_records(d: &std::path::Path) -> devnet::Cast {
    let empty = d.join("empty-store");
    let _ = fs::remove_dir_all(&empty);
    devnet::Cast::load(d).unwrap().with_store(empty)
}
