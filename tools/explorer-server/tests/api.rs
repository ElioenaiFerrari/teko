#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! The HTTP-independent request handler: routes, validation and the read-only rule.

use explorer_server::{Shared, handle};
use teko_crypto::{generate_signing_key, public_key_bytes};
use teko_economy::EconomyGenesis;
use teko_explorer::Index;
use teko_ledger::address_of;

fn shared() -> Shared {
    let mut rng = <rand::rngs::StdRng as rand::SeedableRng>::seed_from_u64(1);
    let k = generate_signing_key(&mut rng);
    let g = EconomyGenesis::test_defaults(
        "teko-api-test",
        public_key_bytes(&k),
        address_of(&public_key_bytes(&k)),
    );
    Shared::new(Index::new(g).unwrap(), "devnet")
}

fn body(r: &explorer_server::Response) -> serde_json::Value {
    serde_json::from_slice(&r.body).unwrap()
}

#[test]
fn the_api_is_read_only() {
    let s = shared();
    for m in ["POST", "PUT", "DELETE", "PATCH"] {
        let r = handle(&s, m, "/api/v1/status");
        assert_eq!(r.status, 405, "{m}");
    }
    assert_eq!(handle(&s, "GET", "/api/v1/status").status, 200);
}

#[test]
fn static_pages_and_unknown_routes() {
    let s = shared();
    let home = handle(&s, "GET", "/");
    assert_eq!(home.status, 200);
    assert!(home.content_type.starts_with("text/html"));
    assert!(String::from_utf8_lossy(&home.body).contains("Teko"));
    assert_eq!(handle(&s, "GET", "/app.js").status, 200);
    assert_eq!(handle(&s, "GET", "/styles.css").status, 200);
    assert_eq!(handle(&s, "GET", "/nada").status, 404);
    assert_eq!(handle(&s, "GET", "/api/v1/nada").status, 404);
    assert_eq!(handle(&s, "GET", "/../etc/passwd").status, 404);
}

#[test]
fn bad_input_is_refused_with_a_clear_error() {
    let s = shared();
    for url in [
        "/api/v1/blocks?limit=0",
        "/api/v1/blocks?limit=101",
        "/api/v1/blocks?limit=abc",
        "/api/v1/blocks?before=x",
        "/api/v1/accounts/tekow1errado",
        "/api/v1/search",
    ] {
        let r = handle(&s, "GET", url);
        assert_eq!(r.status, 400, "{url}");
        assert!(body(&r)["error"].is_string(), "{url}");
    }
    let long = format!("/api/v1/search?q={}", "a".repeat(300));
    assert_eq!(handle(&s, "GET", &long).status, 400);
    assert_eq!(handle(&s, "GET", "/api/v1/blocks/1").status, 404);
    assert_eq!(handle(&s, "GET", "/api/v1/txs/abc").status, 404);
}

#[test]
fn an_empty_chain_reports_its_genesis_state() {
    let s = shared();
    let st = body(&handle(&s, "GET", "/api/v1/status"));
    assert_eq!(st["chain"]["height"], 0);
    assert_eq!(st["chain"]["minted"]["ypy"], 0);
    assert_eq!(st["chain"]["founder_phase_active"], true);
    assert_eq!(st["chain"]["cap"]["text"], "21.000.000 TEKO");
    assert_eq!(
        body(&handle(&s, "GET", "/api/v1/blocks")),
        serde_json::json!([])
    );
    let v = body(&handle(&s, "GET", "/api/v1/validators"));
    assert_eq!(v.as_array().unwrap().len(), 1);
    let found = body(&handle(&s, "GET", "/api/v1/search?q=nada%20disso"));
    assert_eq!(found["type"], "not_found");
}
