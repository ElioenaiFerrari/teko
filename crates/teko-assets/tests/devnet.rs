#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;

use rand::SeedableRng;
use rand::rngs::StdRng;
use teko_assets::devnet::{self, CLASS};
use teko_assets::{Approval, Intent, asset};
use teko_coin::devnet::{CHAIN_ID, ECONOMY_FILE, append_allocation};
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

/// A software "device": signs the consent of `key` for the swap it is asked about.
fn soft<'a>(
    key: &'a SigningKey,
) -> impl FnMut(u8, &str, &Address, &Address, u64, u64) -> Result<Approval, String> + 'a {
    move |kind, asset_id, from, to, price, version| {
        assert_eq!(kind, 3);
        let i = Intent::swap(
            CLASS.as_bytes(),
            asset_id.as_bytes(),
            *from,
            *to,
            price,
            version,
        );
        Ok(Approval::sign(key, CHAIN_ID, &i))
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
    devnet::register(&mut econ, &cast, &me, "casa-1").unwrap();
    assert_eq!(
        devnet::owned(&econ, &me),
        vec![("casa-1".to_string(), 0, 0)]
    );
    assert!(devnet::register(&mut econ, &cast, &me, "casa-1").is_err()); // already registered

    // a lien blocks the sale; releasing it allows it
    devnet::set_lien(&mut econ, &cast, "casa-1", true).unwrap();
    let mut c = soft(&me_key);
    let r = devnet::trade_with_counterparty(&mut econ, &cast, &me, "casa-1", 4000, true, &mut c);
    assert!(r.unwrap_err().contains("gravame"));
    devnet::set_lien(&mut econ, &cast, "casa-1", false).unwrap();
    assert!(devnet::set_lien(&mut econ, &cast, "casa-1", false).is_err()); // no lien left

    // I sell to the counterparty: my consent signed by "my device"
    let before = econ.balance(&me);
    let after =
        devnet::trade_with_counterparty(&mut econ, &cast, &me, "casa-1", 4000, true, &mut c)
            .unwrap();
    assert_eq!(after, before + 4000);
    assert!(devnet::owned(&econ, &me).is_empty());
    assert_eq!(
        asset(econ.ledger(), CLASS.as_bytes(), b"casa-1")
            .unwrap()
            .owner,
        cp.to_vec()
    );
    // selling something that is not mine, or a price of zero, is refused
    assert!(
        devnet::trade_with_counterparty(&mut econ, &cast, &me, "casa-1", 1, true, &mut c)
            .unwrap_err()
            .contains("não é seu")
    );
    assert!(
        devnet::trade_with_counterparty(&mut econ, &cast, &me, "casa-1", 0, false, &mut c).is_err()
    );

    // I buy it back
    let spent =
        devnet::trade_with_counterparty(&mut econ, &cast, &me, "casa-1", 3000, false, &mut c)
            .unwrap();
    assert_eq!(spent, after - 3000);
    assert_eq!(devnet::owned(&econ, &me).len(), 1);
    // the buyer without enough coins cannot buy
    devnet::register(&mut econ, &cast, &cp, "casa-2").unwrap();
    let r =
        devnet::trade_with_counterparty(&mut econ, &cast, &me, "casa-2", 999_999, false, &mut c);
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
        devnet::register(&mut econ2, &cast, &me, "casa-9").unwrap();
        econ2
    };
    // somebody else's key signs "my" consent: the ledger refuses and nothing moves
    let mut c = soft(&other_key);
    let r = devnet::trade_with_counterparty(&mut econ, &cast, &me, "casa-9", 500, true, &mut c);
    assert!(r.is_err());
    assert_eq!(devnet::owned(&econ, &me).len(), 1);
    let _ = fs::remove_dir_all(&d);
}
