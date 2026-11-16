//! Credentials on the chain (ADR-0037): issuance by per-member transactions, contest window,
//! validity, veto and revocation, all measured in block time.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use proptest::prelude::*;
use rand::SeedableRng;
use rand::rngs::StdRng;
use teko_credentials::*;
use teko_crypto::{SigningKey, generate_signing_key, public_key_bytes};
use teko_ledger::{Address, Ledger, LedgerError, Tx, TxError};

const CHAIN: &str = "teko-devnet";
const P: Address = [9; 32];
const CONTEST: u64 = 600;
const VALIDITY: u64 = 10_000;
const CNH: &[u8] = b"cnh-b";
const CAND: &[u8] = b"candidato";

struct W {
    l: Ledger,
    members: Vec<SigningKey>,
    now: u64,
}

fn key(seed: u64) -> SigningKey {
    generate_signing_key(&mut StdRng::seed_from_u64(seed))
}

fn addr(k: &SigningKey) -> Address {
    teko_ledger::address_of(&public_key_bytes(k))
}

fn world() -> W {
    let members: Vec<SigningKey> = (0..3).map(|i| key(2000 + i)).collect();
    let mut l = Ledger::new(CHAIN, vec![module()]).unwrap();
    let keys: Vec<Vec<u8>> = members.iter().map(|k| addr(k).to_vec()).collect();
    genesis_type(&mut l, CNH, &keys, 2, CONTEST, VALIDITY, false).unwrap();
    genesis_type(&mut l, CAND, &keys, 1, CONTEST, VALIDITY, true).unwrap();
    W {
        l,
        members,
        now: 1_000_000,
    }
}

/// Sends `op` as `sender` in a block at time `at`.
fn send(w: &mut W, sender: &SigningKey, op: Op, at: u64) -> Result<(), String> {
    let nonce =
        w.l.nonce_of(&teko_ledger::address_of(&public_key_bytes(sender)));
    let t = Tx::new_signed(sender, CHAIN, nonce, CREDENTIALS_MODULE, op.encode());
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

fn subject() -> Vec<u8> {
    public_key_bytes(&key(5000))
}

fn private_propose(valid_until: u64) -> ProposeOp {
    ProposeOp {
        ctype: CNH.to_vec(),
        subject: subject(),
        commitment: vec![7; 32],
        claims: vec![],
        valid_until,
    }
}

fn id_of(p: &ProposeOp) -> Vec<u8> {
    credential_id(
        &p.ctype,
        &p.subject,
        &p.commitment,
        &p.claims,
        p.valid_until,
    )
    .to_vec()
}

fn vote(ctype: &[u8], id: &[u8], reason: &str) -> VoteOp {
    VoteOp {
        ctype: ctype.to_vec(),
        id: id.to_vec(),
        reason: reason.into(),
    }
}

fn st(w: &W, ctype: &[u8], id: &[u8], at: u64) -> Status {
    status(&w.l, ctype, id, at).unwrap()
}

#[test]
fn it_goes_from_proposed_to_pending_to_active_to_expired_by_block_time_only() {
    let mut w = world();
    let t0 = w.now;
    let until = t0 + 5_000;
    let p = private_propose(until);
    let id = id_of(&p);
    let m0 = w.members[0].clone();
    let m1 = w.members[1].clone();
    send(&mut w, &m0, Op::Propose(p), t0).unwrap();
    assert_eq!(st(&w, CNH, &id, t0), Status::Proposed);
    send(&mut w, &m1, Op::Approve(vote(CNH, &id, "")), t0 + 10).unwrap();
    // pending for the whole contest window
    assert_eq!(st(&w, CNH, &id, t0 + 10), Status::Pending);
    assert_eq!(st(&w, CNH, &id, t0 + 10 + CONTEST - 1), Status::Pending);
    // active from the end of the window, with no transaction at all
    assert_eq!(st(&w, CNH, &id, t0 + 10 + CONTEST), Status::Active);
    assert_eq!(st(&w, CNH, &id, until - 1), Status::Active);
    assert_eq!(st(&w, CNH, &id, until), Status::Expired);
    assert_eq!(credentials_of(&w.l, &subject()), vec![(CNH.to_vec(), id)]);
}

#[test]
fn only_members_propose_and_approve_and_each_member_counts_once() {
    let mut w = world();
    let t0 = w.now;
    let outsider = key(31337);
    let p = private_propose(t0 + 5_000);
    let id = id_of(&p);
    assert!(
        send(&mut w, &outsider, Op::Propose(p.clone()), t0)
            .unwrap_err()
            .contains("member")
    );
    let m0 = w.members[0].clone();
    send(&mut w, &m0, Op::Propose(p.clone()), t0).unwrap();
    // the same proposal again is refused; the proposer cannot approve twice
    assert!(send(&mut w, &m0, Op::Propose(p), t0 + 1).is_err());
    assert!(
        send(&mut w, &m0, Op::Approve(vote(CNH, &id, "")), t0 + 1)
            .unwrap_err()
            .contains("already")
    );
    assert!(send(&mut w, &outsider, Op::Approve(vote(CNH, &id, "")), t0 + 2).is_err());
    assert_eq!(st(&w, CNH, &id, t0 + 4_000), Status::Proposed);
}

#[test]
fn the_quorum_vetoes_inside_the_window_and_never_after() {
    let mut w = world();
    let t0 = w.now;
    let p = private_propose(t0 + 5_000);
    let id = id_of(&p);
    let (m0, m1, m2) = (
        w.members[0].clone(),
        w.members[1].clone(),
        w.members[2].clone(),
    );
    send(&mut w, &m0, Op::Propose(p), t0).unwrap();
    send(&mut w, &m1, Op::Approve(vote(CNH, &id, "")), t0).unwrap();
    // one veto is not a quorum (2 of 3)
    send(&mut w, &m0, Op::Veto(vote(CNH, &id, "fraude")), t0 + 5).unwrap();
    assert_eq!(st(&w, CNH, &id, t0 + 5), Status::Pending);
    // a public reason is required
    assert!(send(&mut w, &m1, Op::Veto(vote(CNH, &id, "")), t0 + 6).is_err());
    send(&mut w, &m1, Op::Veto(vote(CNH, &id, "fraude")), t0 + 6).unwrap();
    assert_eq!(st(&w, CNH, &id, t0 + 6), Status::Vetoed);
    assert_eq!(credential(&w.l, CNH, &id).unwrap().reason, "fraude");
    // a vetoed credential is never active, even much later
    assert_eq!(st(&w, CNH, &id, t0 + 4_000), Status::Vetoed);
    assert!(send(&mut w, &m2, Op::Approve(vote(CNH, &id, "")), t0 + 7).is_err());

    // after the window a veto is refused (it must be a revocation)
    let p2 = ProposeOp {
        commitment: vec![8; 32],
        ..private_propose(t0 + 5_000)
    };
    let id2 = id_of(&p2);
    send(&mut w, &m0, Op::Propose(p2), t0 + 10).unwrap();
    send(&mut w, &m1, Op::Approve(vote(CNH, &id2, "")), t0 + 10).unwrap();
    let e = send(
        &mut w,
        &m0,
        Op::Veto(vote(CNH, &id2, "tarde")),
        t0 + 10 + CONTEST,
    )
    .unwrap_err();
    assert!(e.contains("contest window"), "{e}");
}

#[test]
fn an_active_credential_is_revoked_by_the_quorum_with_a_public_reason() {
    let mut w = world();
    let t0 = w.now;
    let p = private_propose(t0 + 5_000);
    let id = id_of(&p);
    let (m0, m1) = (w.members[0].clone(), w.members[1].clone());
    send(&mut w, &m0, Op::Propose(p), t0).unwrap();
    send(&mut w, &m1, Op::Approve(vote(CNH, &id, "")), t0).unwrap();
    // while pending, a revocation is refused (veto instead)
    assert!(send(&mut w, &m0, Op::Revoke(vote(CNH, &id, "x")), t0 + 5).is_err());
    let late = t0 + CONTEST + 1;
    send(&mut w, &m0, Op::Revoke(vote(CNH, &id, "cassada")), late).unwrap();
    assert_eq!(st(&w, CNH, &id, late), Status::Active); // 1 of 2
    send(&mut w, &m1, Op::Revoke(vote(CNH, &id, "cassada")), late + 1).unwrap();
    assert_eq!(st(&w, CNH, &id, late + 1), Status::Revoked);
    // an expired credential cannot be revoked (it is already worthless)
    let p2 = ProposeOp {
        commitment: vec![9; 32],
        ..private_propose(late + 700)
    };
    let id2 = id_of(&p2);
    send(&mut w, &m0, Op::Propose(p2), late + 2).unwrap();
    send(&mut w, &m1, Op::Approve(vote(CNH, &id2, "")), late + 2).unwrap();
    assert!(send(&mut w, &m0, Op::Revoke(vote(CNH, &id2, "x")), late + 701).is_err());
}

#[test]
fn validity_rules_follow_the_type() {
    let mut w = world();
    let t0 = w.now;
    let m0 = w.members[0].clone();
    // a type with validity needs a valid_until in (now, now + validity]
    for bad in [0, t0, t0 + VALIDITY + 1] {
        assert!(
            send(&mut w, &m0, Op::Propose(private_propose(bad)), t0).is_err(),
            "{bad}"
        );
    }
    // it must outlive its own contest window
    let e = send(&mut w, &m0, Op::Propose(private_propose(t0 + 5)), t0);
    assert!(e.is_err());
    // a lifetime type refuses an expiry
    let keys: Vec<Vec<u8>> = w.members.iter().map(|k| addr(k).to_vec()).collect();
    let mut l = Ledger::new(CHAIN, vec![module()]).unwrap();
    genesis_type(&mut l, b"vitalicia", &keys, 1, CONTEST, 0, false).unwrap();
    w.l = l;
    let ok = ProposeOp {
        ctype: b"vitalicia".to_vec(),
        ..private_propose(0)
    };
    send(&mut w, &m0, Op::Propose(ok.clone()), t0).unwrap();
    let id = id_of(&ok);
    assert_eq!(
        st(&w, b"vitalicia", &id, t0 + CONTEST + 10_000_000),
        Status::Active
    );
    let with_expiry = ProposeOp {
        commitment: vec![1; 32],
        ..private_propose(t0 + 100)
    };
    let with_expiry = ProposeOp {
        ctype: b"vitalicia".to_vec(),
        ..with_expiry
    };
    assert!(send(&mut w, &m0, Op::Propose(with_expiry), t0 + 1).is_err());
}

#[test]
fn private_types_keep_only_a_commitment_and_public_types_bind_their_claims() {
    let mut w = world();
    let t0 = w.now;
    let m0 = w.members[0].clone();
    // a private type refuses claims in clear
    let leak = ProposeOp {
        claims: b"nome=Ana".to_vec(),
        ..private_propose(t0 + 5_000)
    };
    assert!(
        send(&mut w, &m0, Op::Propose(leak), t0)
            .unwrap_err()
            .contains("off the chain")
    );
    // a public type needs claims whose hash is the commitment
    let claims = b"cargo=governador;uf=SP;numero=13;nome=Ana".to_vec();
    let mut p = ProposeOp {
        ctype: CAND.to_vec(),
        subject: subject(),
        commitment: vec![0; 32],
        claims: claims.clone(),
        valid_until: t0 + 5_000,
    };
    assert!(
        send(&mut w, &m0, Op::Propose(p.clone()), t0)
            .unwrap_err()
            .contains("match")
    );
    p.commitment = public_commitment(&claims).to_vec();
    let id = id_of(&p);
    // the threshold of "candidato" is 1: the proposal is already approved (pending)
    send(&mut w, &m0, Op::Propose(p), t0).unwrap();
    assert_eq!(st(&w, CAND, &id, t0), Status::Pending);
    assert_eq!(st(&w, CAND, &id, t0 + CONTEST), Status::Active);
    assert_eq!(credential(&w.l, CAND, &id).unwrap().claims, claims);
    assert_eq!(credentials_of_type(&w.l, CAND).len(), 1);
    assert_eq!(types(&w.l).len(), 2);
}

#[test]
fn unknown_types_garbage_and_non_canonical_payloads_are_refused() {
    let mut w = world();
    let t0 = w.now;
    let m0 = w.members[0].clone();
    let p = ProposeOp {
        ctype: b"nao-existe".to_vec(),
        ..private_propose(t0 + 5_000)
    };
    assert!(
        send(&mut w, &m0, Op::Propose(p), t0)
            .unwrap_err()
            .contains("unknown")
    );
    let bad_subject = ProposeOp {
        subject: vec![1, 2, 3],
        ..private_propose(t0 + 5_000)
    };
    assert!(send(&mut w, &m0, Op::Propose(bad_subject), t0).is_err());
    let op = Op::Propose(private_propose(t0 + 5_000));
    let bytes = op.encode();
    assert_eq!(Op::decode(&bytes), Some(op));
    let mut longer = bytes.clone();
    longer.push(0);
    assert_eq!(Op::decode(&longer), None);
    assert_eq!(Op::decode(&[99, 1]), None);
    assert_eq!(Op::decode(&[]), None);
}

#[test]
fn genesis_rejects_bad_types() {
    let keys: Vec<Vec<u8>> = (0..2).map(|i| addr(&key(3000 + i)).to_vec()).collect();
    let mut l = Ledger::new(CHAIN, vec![module()]).unwrap();
    assert!(genesis_type(&mut l, b"", &keys, 1, 60, 0, false).is_err());
    assert!(genesis_type(&mut l, b"x", &keys, 0, 60, 0, false).is_err());
    assert!(genesis_type(&mut l, b"x", &keys, 3, 60, 0, false).is_err());
    assert!(genesis_type(&mut l, b"x", &keys, 1, 0, 0, false).is_err());
    assert!(
        genesis_type(
            &mut l,
            b"x",
            &[keys[0].clone(), keys[0].clone()],
            1,
            60,
            0,
            false
        )
        .is_err()
    );
    assert!(genesis_type(&mut l, b"x", &[vec![1; 5]], 1, 60, 0, false).is_err());
    // a member is an ADDRESS, not a key
    assert!(genesis_type(&mut l, b"x", &[public_key_bytes(&key(1))], 1, 60, 0, false).is_err());
    genesis_type(&mut l, b"x", &keys, 2, 60, 0, false).unwrap();
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    /// Whatever the order of approvals and vetoes, a credential is never ACTIVE before the end of
    /// the contest window, and a vetoed or revoked one is never active again.
    #[test]
    fn a_credential_is_never_active_inside_the_window_or_after_veto(
        steps in proptest::collection::vec((0usize..3, 0u8..3, 0u64..1_500), 1..20)
    ) {
        let mut w = world();
        let t0 = w.now;
        let p = private_propose(t0 + 9_000);
        let id = id_of(&p);
        let m0 = w.members[0].clone();
        send(&mut w, &m0, Op::Propose(p), t0).unwrap();
        let mut at = t0;
        let mut dead = false;
        for (who, what, dt) in steps {
            at += dt;
            let m = w.members[who].clone();
            let v = vote(CNH, &id, "motivo");
            let op = match what { 0 => Op::Approve(v), 1 => Op::Veto(v), _ => Op::Revoke(v) };
            let _ = send(&mut w, &m, op, at);
            let rec = credential(&w.l, CNH, &id).unwrap();
            let s = rec.status_at(at);
            if matches!(s, Status::Vetoed | Status::Revoked) { dead = true; }
            if dead { prop_assert!(matches!(s, Status::Vetoed | Status::Revoked)); }
            if rec.approved_at != 0 {
                prop_assert!(rec.active_at == rec.approved_at + CONTEST);
                if at < rec.active_at { prop_assert!(s != Status::Active); }
            }
        }
    }
}
