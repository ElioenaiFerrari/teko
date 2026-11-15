#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use proptest::prelude::*;
use rand::SeedableRng;
use rand::rngs::StdRng;
use teko_crypto::{SigningKey, generate_signing_key};
use teko_ledger::{
    Ctx, Ledger, LedgerError, Module, ModuleError, State, Tx, TxError, address_of,
    verify_state_proof,
};

const CHAIN: &str = "teko-test";
const P: [u8; 32] = [9; 32];

/// Test module: payload = key || 0x00 || value (sets key); payload "fail" fails;
/// payload "bump" increments a counter (non-commutative with set on the same key).
struct Kv(u32);

impl Module for Kv {
    fn id(&self) -> u32 {
        self.0
    }
    fn execute(&self, ctx: &mut Ctx<'_, '_>, payload: &[u8]) -> Result<(), ModuleError> {
        if payload == b"fail" {
            ctx.set(b"partial", vec![1]);
            return Err(ModuleError::new("asked to fail"));
        }
        if payload == b"bump" {
            let n = ctx
                .get(b"counter")
                .and_then(|v| v.first().copied())
                .unwrap_or(0);
            ctx.set(b"counter", vec![n.wrapping_add(1)]);
            ctx.emit("bumped", vec![n.wrapping_add(1)]);
            return Ok(());
        }
        let split = payload
            .iter()
            .position(|b| *b == 0)
            .ok_or(ModuleError::new("bad payload"))?;
        ctx.set(&payload[..split], payload[split + 1..].to_vec());
        Ok(())
    }
}

fn key(seed: u64) -> SigningKey {
    generate_signing_key(&mut StdRng::seed_from_u64(seed))
}

fn ledger() -> Ledger {
    Ledger::new(CHAIN, vec![Box::new(Kv(1)), Box::new(Kv(2))]).unwrap()
}

fn tx(sk: &SigningKey, nonce: u64, module: u32, payload: &[u8]) -> Tx {
    Tx::new_signed(sk, CHAIN, nonce, module, payload.to_vec())
}

#[test]
fn applies_block_and_advances_height_and_nonce() {
    let mut l = ledger();
    let a = key(1);
    let out = l
        .apply_block(1, &P, &[tx(&a, 0, 1, b"x\0y"), tx(&a, 1, 1, b"bump")])
        .unwrap();
    assert_eq!(out.height, 1);
    assert_eq!(l.height(), 1);
    assert_eq!(l.read(1, b"x"), Some(&b"y"[..]));
    assert_eq!(l.read(1, b"counter"), Some(&[1u8][..]));
    assert_eq!(
        l.nonce_of(&address_of(&teko_crypto::public_key_bytes(&a))),
        2
    );
    assert_eq!(out.receipts[1].events[0].topic, "bumped");
    assert_eq!(out.state_root, l.state_root());
}

#[test]
fn tampering_any_field_breaks_the_signature() {
    let a = key(2);
    let t = tx(&a, 0, 1, b"x\0y");
    assert!(t.verify().is_ok());
    let mut m = t.clone();
    m.payload.push(1);
    assert_eq!(m.verify(), Err(TxError::BadSignature));
    let mut m = t.clone();
    m.nonce += 1;
    assert_eq!(m.verify(), Err(TxError::BadSignature));
    let mut m = t.clone();
    m.module = 2;
    assert_eq!(m.verify(), Err(TxError::BadSignature));
    let mut m = t.clone();
    m.chain_id = "other".into();
    assert_eq!(m.verify(), Err(TxError::BadSignature));
    let mut m = t;
    m.sender = teko_crypto::public_key_bytes(&key(3));
    assert_eq!(m.verify(), Err(TxError::BadSignature));
}

#[test]
fn rejects_wrong_chain_replay_and_nonce_gap() {
    let mut l = ledger();
    let a = key(4);
    let foreign = Tx::new_signed(&a, "other-chain", 0, 1, b"x\0y".to_vec());
    assert_eq!(l.check_tx(&foreign), Err(TxError::WrongChain));
    l.apply_block(1, &P, &[tx(&a, 0, 1, b"x\0y")]).unwrap();
    // replay of the same transaction
    assert!(matches!(
        l.check_tx(&tx(&a, 0, 1, b"x\0y")),
        Err(TxError::BadNonce {
            expected: 1,
            got: 0
        })
    ));
    assert!(matches!(
        l.apply_block(2, &P, &[tx(&a, 5, 1, b"x\0y")]),
        Err(LedgerError::Tx {
            index: 0,
            error: TxError::BadNonce {
                expected: 1,
                got: 5
            }
        })
    ));
    assert_eq!(l.height(), 1);
}

#[test]
fn unknown_module_is_rejected() {
    let l = ledger();
    assert_eq!(
        l.check_tx(&tx(&key(5), 0, 9, b"x\0y")),
        Err(TxError::UnknownModule(9))
    );
}

#[test]
fn a_failing_transaction_rejects_the_whole_block_atomically() {
    let mut l = ledger();
    let (a, b) = (key(6), key(7));
    let root = l.state_root();
    let r = l.apply_block(1, &P, &[tx(&a, 0, 1, b"x\0y"), tx(&b, 0, 1, b"fail")]);
    assert!(matches!(r, Err(LedgerError::Tx { index: 1, .. })));
    assert_eq!(l.height(), 0);
    assert_eq!(l.state_root(), root);
    assert_eq!(l.read(1, b"x"), None);
    assert_eq!(l.read(1, b"partial"), None);
}

#[test]
fn wrong_height_is_rejected() {
    let mut l = ledger();
    assert_eq!(
        l.apply_block(2, &P, &[]),
        Err(LedgerError::WrongHeight {
            expected: 1,
            got: 2
        })
    );
    l.apply_block(1, &P, &[]).unwrap();
    assert_eq!(
        l.apply_block(1, &P, &[]),
        Err(LedgerError::WrongHeight {
            expected: 2,
            got: 1
        })
    );
}

#[test]
fn modules_have_separate_namespaces() {
    let mut l = ledger();
    let a = key(8);
    l.apply_block(1, &P, &[tx(&a, 0, 1, b"k\0one"), tx(&a, 1, 2, b"k\0two")])
        .unwrap();
    assert_eq!(l.read(1, b"k"), Some(&b"one"[..]));
    assert_eq!(l.read(2, b"k"), Some(&b"two"[..]));
}

#[test]
fn select_valid_drops_bad_transactions_and_the_result_applies() {
    let mut l = ledger();
    let (a, b) = (key(9), key(10));
    let batch = vec![
        tx(&a, 0, 1, b"x\0y"),
        tx(&b, 0, 1, b"fail"),
        tx(&a, 1, 1, b"bump"),
        tx(&b, 0, 1, b"z\0w"),
    ];
    let kept = l.select_valid(&P, batch);
    assert_eq!(kept.len(), 3);
    assert!(l.apply_block(1, &P, &kept).is_ok());
    assert_eq!(l.read(1, b"partial"), None);
}

#[test]
fn genesis_only_before_the_first_block_and_modules_are_validated() {
    let mut l = ledger();
    l.genesis_put(1, b"seed", vec![7]).unwrap();
    assert_eq!(l.read(1, b"seed"), Some(&[7u8][..]));
    l.apply_block(1, &P, &[]).unwrap();
    assert_eq!(
        l.genesis_put(1, b"late", vec![1]),
        Err(LedgerError::GenesisClosed)
    );
    assert!(matches!(
        Ledger::new(CHAIN, vec![Box::new(Kv(0))]),
        Err(LedgerError::ReservedModule)
    ));
    assert!(matches!(
        Ledger::new(CHAIN, vec![Box::new(Kv(1)), Box::new(Kv(1))]),
        Err(LedgerError::DuplicateModule(1))
    ));
}

#[test]
fn state_proofs_verify_and_reject_a_wrong_value() {
    let mut l = ledger();
    let a = key(11);
    let txs: Vec<Tx> = (0..9u8)
        .map(|i| tx(&a, u64::from(i), 1, &[b'k', i, 0, i]))
        .collect();
    let out = l.apply_block(1, &P, &txs).unwrap();
    let state: &State = l.state();
    let mut k = vec![1u8, 0, 0, 0, 1];
    k.extend_from_slice(&[b'k', 3]);
    let p = state.prove(&k).unwrap();
    assert!(verify_state_proof(&out.state_root, &k, &[3], &p));
    assert!(!verify_state_proof(&out.state_root, &k, &[4], &p));
    assert!(state.prove(b"missing").is_none());
}

#[test]
fn canonical_bytes_roundtrip_and_reject_garbage() {
    let t = tx(&key(12), 0, 1, b"x\0y");
    assert_eq!(Tx::from_bytes(&t.to_bytes()).unwrap(), t);
    let mut extra = t.to_bytes();
    extra.push(0);
    assert!(Tx::from_bytes(&extra).is_err());
    assert!(Tx::from_bytes(&[0xff, 0xff]).is_err());
}

#[test]
fn oversized_payload_is_rejected() {
    let t = tx(&key(13), 0, 1, &vec![0u8; teko_ledger::MAX_PAYLOAD + 1]);
    assert_eq!(t.verify(), Err(TxError::PayloadTooLarge));
}

proptest! {
    /// Two replicas applying the same blocks reach the same root, and the root
    /// does not depend on the order entries were inserted.
    #[test]
    fn replicas_agree(ops in proptest::collection::vec((0u64..3, 0u8..4, 0u8..255), 1..30)) {
        let keys: Vec<SigningKey> = (0..3).map(|i| key(100 + i)).collect();
        let mut nonces = [0u64; 3];
        let mut txs = Vec::new();
        for (who, k, v) in ops {
            let w = who as usize;
            txs.push(tx(&keys[w], nonces[w], 1, &[b'k', k, 0, v]));
            nonces[w] += 1;
        }
        let mut l1 = ledger();
        let mut l2 = ledger();
        let half = txs.len() / 2;
        let o1 = l1.apply_block(1, &P, &txs[..half]).unwrap();
        let o1b = l1.apply_block(2, &P, &txs[half..]).unwrap();
        let raw: Vec<Vec<u8>> = txs[..half].iter().map(Tx::to_bytes).collect();
        let o2 = l2.apply_block_bytes(1, &P, &raw).unwrap();
        let raw: Vec<Vec<u8>> = txs[half..].iter().map(Tx::to_bytes).collect();
        let o2b = l2.apply_block_bytes(2, &P, &raw).unwrap();
        prop_assert_eq!(o1.state_root, o2.state_root);
        prop_assert_eq!(o1b.state_root, o2b.state_root);
        prop_assert_eq!(l1.state_root(), l2.state_root());
    }

    #[test]
    fn order_of_transactions_matters_for_roots(_x in 0u8..1) {
        // set then bump vs bump then set on the same key give different states
        let (a, b) = (key(200), key(201));
        let mut l1 = ledger();
        let mut l2 = ledger();
        l1.apply_block(1, &P, &[tx(&a, 0, 1, b"counter\0\x09"), tx(&b, 0, 1, b"bump")]).unwrap();
        l2.apply_block(1, &P, &[tx(&b, 0, 1, b"bump"), tx(&a, 0, 1, b"counter\0\x09")]).unwrap();
        prop_assert_ne!(l1.state_root(), l2.state_root());
    }
}

#[test]
fn check_block_is_a_dry_run_that_predicts_the_real_root() {
    let mut l = ledger();
    let a = key(31);
    let txs = [tx(&a, 0, 1, b"x\0y"), tx(&a, 1, 1, b"bump")];
    let before = l.state_root();
    let (root, receipts) = l.check_block(1, &P, &txs).unwrap();
    assert_eq!(receipts.len(), 2);
    assert_eq!(l.state_root(), before); // nothing changed
    assert_eq!(l.height(), 0);
    assert_eq!(l.apply_block(1, &P, &txs).unwrap().state_root, root);
    // wrong height and invalid transactions are refused by the dry run too
    assert!(matches!(
        l.check_block(1, &P, &[]),
        Err(LedgerError::WrongHeight {
            expected: 2,
            got: 1
        })
    ));
    assert!(l.check_block(2, &P, &[tx(&a, 0, 1, b"x\0y")]).is_err());
}
