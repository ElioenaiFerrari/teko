#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use proptest::prelude::*;
use prost::Message;
use rand::SeedableRng;
use rand::rngs::StdRng;
use teko_bft::{Evidence, Vote, VoteKind};
use teko_coin::*;
use teko_crypto::{SigningKey, generate_signing_key, public_key_bytes};
use teko_ledger::{Address, Ledger, LedgerError, Tx, TxError, address_of};

const CHAIN: &str = "teko-devnet";
const P: Address = [9; 32];

fn cfg() -> CoinConfig {
    CoinConfig {
        cap: 1_000_000,
        initial_reward: 1000,
        halving_interval: 10,
        fee_per_byte: 0,
        unbonding_period: 5,
        slash_bps: 1000,
        faucet_amount: 500,
        faucet_cooldown: 3,
    }
}

fn key(seed: u64) -> SigningKey {
    generate_signing_key(&mut StdRng::seed_from_u64(seed))
}
fn addr(sk: &SigningKey) -> Address {
    address_of(&public_key_bytes(sk))
}

fn new_ledger(cfg: CoinConfig, alloc: &[(Address, u64)], devnet: bool) -> Ledger {
    let mut l = Ledger::new(CHAIN, vec![module()])
        .unwrap()
        .with_fee_module(COIN_MODULE)
        .unwrap();
    genesis(&mut l, cfg, alloc, devnet).unwrap();
    l
}

fn tx(sk: &SigningKey, l: &Ledger, fee: u64, op: Op) -> Tx {
    Tx::new_signed_with_fee(
        sk,
        CHAIN,
        l.nonce_of(&addr(sk)),
        fee,
        COIN_MODULE,
        op.encode(),
    )
}

fn blk(l: &mut Ledger, txs: &[Tx]) -> Result<(), LedgerError> {
    let h = l.height() + 1;
    l.apply_block(h, &P, txs).map(|_| ())
}

fn run(l: &mut Ledger, sk: &SigningKey, fee: u64, op: Op) -> Result<(), LedgerError> {
    let t = tx(sk, l, fee, op);
    blk(l, &[t])
}

fn empty(l: &mut Ledger, n: u64) {
    for _ in 0..n {
        blk(l, &[]).unwrap();
    }
}

fn err_msg(r: Result<(), LedgerError>) -> String {
    match r {
        Err(LedgerError::Tx {
            error: TxError::Module(m),
            ..
        }) => m,
        other => format!("{other:?}"),
    }
}

#[test]
fn genesis_rules() {
    let mut l = Ledger::new(CHAIN, vec![module()]).unwrap();
    assert_eq!(
        genesis(&mut l, cfg(), &[], false),
        Err(CoinError::OnlyDevnet)
    ); // faucet
    let no_faucet = CoinConfig {
        faucet_amount: 0,
        ..cfg()
    };
    assert_eq!(
        genesis(&mut l, no_faucet.clone(), &[([1; 32], 5)], false),
        Err(CoinError::OnlyDevnet)
    );
    assert!(matches!(
        genesis(&mut l, CoinConfig { cap: 0, ..cfg() }, &[], true),
        Err(CoinError::InvalidConfig(_))
    ));
    assert!(matches!(
        genesis(
            &mut l,
            CoinConfig {
                halving_interval: 0,
                ..cfg()
            },
            &[],
            true
        ),
        Err(CoinError::InvalidConfig(_))
    ));
    assert!(matches!(
        genesis(
            &mut l,
            CoinConfig {
                slash_bps: 10_001,
                ..cfg()
            },
            &[],
            true
        ),
        Err(CoinError::InvalidConfig(_))
    ));
    assert_eq!(
        genesis(
            &mut l,
            CoinConfig { cap: 10, ..cfg() },
            &[([1; 32], 11)],
            true
        ),
        Err(CoinError::AllocationExceedsCap)
    );
    // a mainnet-style genesis: no allocations, no faucet, nothing minted
    let mut l = Ledger::new(CHAIN, vec![module()]).unwrap();
    genesis(&mut l, no_faucet, &[], false).unwrap();
    assert_eq!(minted(&l), 0);
    audit(&l).unwrap();
}

#[test]
fn reward_halves_and_is_credited_to_the_proposer() {
    let c = CoinConfig {
        initial_reward: 1000,
        halving_interval: 10,
        ..cfg()
    };
    assert_eq!(reward_at(&c, 0), 0);
    assert_eq!(reward_at(&c, 1), 1000);
    assert_eq!(reward_at(&c, 10), 1000);
    assert_eq!(reward_at(&c, 11), 500);
    assert_eq!(reward_at(&c, 21), 250);
    assert_eq!(reward_at(&c, 10 * 64 + 1), 0);
    let mut l = new_ledger(c, &[], true);
    empty(&mut l, 11);
    assert_eq!(balance(&l, &P), 10 * 1000 + 500);
    assert_eq!(minted(&l), 10_500);
    audit(&l).unwrap();
}

#[test]
fn the_cap_is_never_exceeded() {
    let c = CoinConfig {
        cap: 25,
        initial_reward: 10,
        halving_interval: 100,
        faucet_amount: 0,
        ..cfg()
    };
    let mut l = new_ledger(c, &[], true);
    empty(&mut l, 10);
    assert_eq!(minted(&l), 25);
    assert_eq!(balance(&l, &P), 25);
    audit(&l).unwrap();
}

#[test]
fn transfers_move_funds_and_refuse_overdraft_and_zero() {
    let (a, b) = (key(1), key(2));
    let mut l = new_ledger(cfg(), &[(addr(&a), 100)], true);
    run(
        &mut l,
        &a,
        0,
        Op::Transfer {
            to: addr(&b),
            amount: 40,
        },
    )
    .unwrap();
    assert_eq!((balance(&l, &addr(&a)), balance(&l, &addr(&b))), (60, 40));
    let t = tx(
        &a,
        &l,
        0,
        Op::Transfer {
            to: addr(&b),
            amount: 61,
        },
    );
    assert_eq!(err_msg(blk(&mut l, &[t])), "insufficient funds");
    let t = tx(
        &a,
        &l,
        0,
        Op::Transfer {
            to: addr(&b),
            amount: 0,
        },
    );
    assert_eq!(err_msg(blk(&mut l, &[t])), "amount must be positive");
    audit(&l).unwrap();
}

#[test]
fn fees_are_priced_per_byte_and_paid_to_the_proposer() {
    let a = key(3);
    let c = CoinConfig {
        fee_per_byte: 2,
        initial_reward: 0,
        ..cfg()
    };
    let mut l = new_ledger(c, &[(addr(&a), 10_000)], true);
    let op = Op::Transfer {
        to: [7; 32],
        amount: 1,
    };
    // too low
    let low = tx(&a, &l, 1, op.clone());
    assert!(matches!(l.check_tx(&low), Err(TxError::FeeTooLow { .. })));
    assert!(matches!(
        blk(&mut l, &[low]),
        Err(LedgerError::Tx {
            error: TxError::FeeTooLow { .. },
            ..
        })
    ));
    // enough: pay generously, the proposer gets exactly what was paid
    let ok = tx(&a, &l, 1000, op.clone());
    l.check_tx(&ok).unwrap();
    blk(&mut l, &[ok]).unwrap();
    assert_eq!(balance(&l, &P), 1000);
    assert_eq!(balance(&l, &addr(&a)), 10_000 - 1000 - 1);
    audit(&l).unwrap();
    // cannot pay the fee: the whole transaction is refused
    let poor = key(4);
    let t = tx(&poor, &l, 1000, op);
    assert_eq!(err_msg(blk(&mut l, &[t])), "cannot pay the fee");
}

#[test]
fn stake_unbonding_and_claim() {
    let a = key(5);
    let mut l = new_ledger(cfg(), &[(addr(&a), 1000)], true);
    run(&mut l, &a, 0, Op::Stake { amount: 600 }).unwrap();
    assert_eq!(stake_of(&l, &addr(&a)), 600);
    assert_eq!(validators(&l), vec![(public_key_bytes(&a), 600)]);
    run(&mut l, &a, 0, Op::Unstake { amount: 100 }).unwrap();
    assert_eq!(stake_of(&l, &addr(&a)), 500);
    let release = unbonding_of(&l, &addr(&a))[0].0;
    // too early
    let t = tx(&a, &l, 0, Op::Claim);
    assert_eq!(err_msg(blk(&mut l, &[t])), "nothing to claim yet");
    let wait = release - l.height() - 1;
    empty(&mut l, wait);
    run(&mut l, &a, 0, Op::Claim).unwrap();
    assert_eq!(balance(&l, &addr(&a)), 400 + 100);
    assert!(unbonding_of(&l, &addr(&a)).is_empty());
    let t = tx(&a, &l, 0, Op::Unstake { amount: 501 });
    assert_eq!(err_msg(blk(&mut l, &[t])), "insufficient funds");
    audit(&l).unwrap();
}

fn evidence(sk: &SigningKey, height: u64) -> Vec<u8> {
    let a = Vote::new(CHAIN, sk, VoteKind::Prevote, height, 0, Some(&[1; 32]));
    let b = Vote::new(CHAIN, sk, VoteKind::Prevote, height, 0, Some(&[2; 32]));
    Evidence {
        a: Some(a),
        b: Some(b),
    }
    .encode_to_vec()
}

#[test]
fn equivocation_burns_stake_and_unbonding_once() {
    let (v, reporter) = (key(6), key(7));
    let mut l = new_ledger(cfg(), &[(addr(&v), 1000), (addr(&reporter), 10)], true);
    run(&mut l, &v, 0, Op::Stake { amount: 1000 }).unwrap();
    run(&mut l, &v, 0, Op::Unstake { amount: 200 }).unwrap();
    let ev = evidence(&v, 2);
    run(
        &mut l,
        &reporter,
        0,
        Op::Slash {
            evidence: ev.clone(),
        },
    )
    .unwrap();
    // 10% of (800 staked + 200 unbonding) = 100, taken from the stake first
    assert_eq!(stake_of(&l, &addr(&v)), 700);
    assert_eq!(unbonding_of(&l, &addr(&v))[0].1, 200);
    assert_eq!(burned(&l), 100);
    audit(&l).unwrap();
    // the same offence cannot be punished twice, even with a different pair of votes
    let again = evidence(&v, 2);
    let t = tx(&reporter, &l, 0, Op::Slash { evidence: again });
    assert_eq!(
        err_msg(blk(&mut l, &[t])),
        "already slashed for this offence"
    );
}

#[test]
fn bad_or_stale_evidence_is_refused() {
    let (v, reporter) = (key(8), key(9));
    let mut l = new_ledger(cfg(), &[(addr(&v), 1000), (addr(&reporter), 10)], true);
    run(&mut l, &v, 0, Op::Stake { amount: 1000 }).unwrap();
    // two votes for the SAME block are not equivocation
    let a = Vote::new(CHAIN, &v, VoteKind::Prevote, 1, 0, Some(&[1; 32]));
    let same = Evidence {
        a: Some(a.clone()),
        b: Some(a),
    }
    .encode_to_vec();
    let t = tx(&reporter, &l, 0, Op::Slash { evidence: same });
    assert_eq!(err_msg(blk(&mut l, &[t])), "invalid evidence");
    // a vote signed for another chain does not count here
    let x = Vote::new("other", &v, VoteKind::Prevote, 1, 0, Some(&[1; 32]));
    let y = Vote::new("other", &v, VoteKind::Prevote, 1, 0, Some(&[2; 32]));
    let foreign = Evidence {
        a: Some(x),
        b: Some(y),
    }
    .encode_to_vec();
    let t = tx(&reporter, &l, 0, Op::Slash { evidence: foreign });
    assert_eq!(err_msg(blk(&mut l, &[t])), "invalid evidence");
    // too old: the unbonding window (5 blocks) has passed
    empty(&mut l, 10);
    let t = tx(
        &reporter,
        &l,
        0,
        Op::Slash {
            evidence: evidence(&v, 1),
        },
    );
    assert_eq!(
        err_msg(blk(&mut l, &[t])),
        "evidence outside the slashing window"
    );
    // garbage
    let t = tx(
        &reporter,
        &l,
        0,
        Op::Slash {
            evidence: vec![1, 2, 3],
        },
    );
    assert!(err_msg(blk(&mut l, &[t])).contains("evidence"));
    assert_eq!(burned(&l), 0);
}

#[test]
fn faucet_is_rate_limited_labeled_and_devnet_only() {
    let a = key(10);
    let mut l = new_ledger(cfg(), &[], true);
    let t = tx(&a, &l, 0, Op::Faucet);
    let out = l.apply_block(1, &P, &[t]).unwrap();
    assert_eq!(balance(&l, &addr(&a)), 500);
    assert!(
        out.receipts[0]
            .events
            .iter()
            .any(|e| e.topic == "faucet-TEST-COINS")
    );
    let t = tx(&a, &l, 0, Op::Faucet);
    assert_eq!(err_msg(blk(&mut l, &[t])), "faucet cooldown");
    empty(&mut l, 3);
    run(&mut l, &a, 0, Op::Faucet).unwrap();
    assert_eq!(balance(&l, &addr(&a)), 1000);
    audit(&l).unwrap();
    // a non-devnet genesis cannot even be created with a faucet, and without one the op fails
    let mut l2 = new_ledger(
        CoinConfig {
            faucet_amount: 0,
            ..cfg()
        },
        &[],
        false,
    );
    let t = tx(&a, &l2, 0, Op::Faucet);
    assert_eq!(err_msg(blk(&mut l2, &[t])), "faucet disabled");
}

#[test]
fn replicas_with_the_coin_agree_on_the_state_root() {
    let (a, b) = (key(11), key(12));
    let build = || new_ledger(cfg(), &[(addr(&a), 1000)], true);
    let (mut l1, mut l2) = (build(), build());
    let txs = vec![
        tx(
            &a,
            &l1,
            0,
            Op::Transfer {
                to: addr(&b),
                amount: 300,
            },
        ),
        Tx::new_signed_with_fee(
            &a,
            CHAIN,
            1,
            0,
            COIN_MODULE,
            Op::Stake { amount: 100 }.encode(),
        ),
    ];
    blk(&mut l1, &txs).unwrap();
    blk(&mut l2, &txs).unwrap();
    assert_eq!(l1.state_root(), l2.state_root());
}

#[test]
fn malformed_payloads_are_refused() {
    let a = key(13);
    let mut l = new_ledger(cfg(), &[(addr(&a), 10)], true);
    for bad in [
        vec![],
        vec![9],
        vec![0, 1, 2],
        vec![1, 1],
        vec![3, 0],
        vec![5, 1],
    ] {
        let t = Tx::new_signed_with_fee(&a, CHAIN, l.nonce_of(&addr(&a)), 0, COIN_MODULE, bad);
        assert!(err_msg(blk(&mut l, &[t])).contains("malformed"));
    }
}

proptest! {
    /// Whatever sequence of operations runs, supply is conserved and capped.
    #[test]
    fn supply_is_conserved_and_capped(ops in proptest::collection::vec((0usize..3, 0u8..6, 1u64..2000), 1..60)) {
        let keys: Vec<SigningKey> = (0..3).map(|i| key(300 + i)).collect();
        let c = CoinConfig { cap: 30_000, initial_reward: 700, halving_interval: 7, ..cfg() };
        let alloc: Vec<(Address, u64)> = keys.iter().map(|k| (addr(k), 1000)).collect();
        let mut l = new_ledger(c.clone(), &alloc, true);
        for (who, kind, amount) in ops {
            let op = match kind {
                0 => Op::Transfer { to: addr(&keys[(who + 1) % 3]), amount },
                1 => Op::Stake { amount },
                2 => Op::Unstake { amount },
                3 => Op::Claim,
                4 => Op::Faucet,
                _ => Op::Slash { evidence: evidence(&keys[who], l.height().max(1)) },
            };
            let t = tx(&keys[who], &l, 0, op);
            let h = l.height() + 1;
            let kept = l.select_valid(&P, vec![t]);
            l.apply_block(h, &P, &kept).unwrap();
            prop_assert!(audit(&l).is_ok(), "{:?}", audit(&l));
            prop_assert!(minted(&l) <= c.cap);
        }
    }
}
