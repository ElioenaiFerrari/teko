use prost::Message;
use teko_ledger::{Address, Ledger};

use crate::module::{
    COIN_MODULE, CoinConfig, K_BURNED, K_CFG, K_MINTED, P_BAL, P_STAKE, P_UNB, decode_unbonding,
    k_bal, k_pk, k_stake, k_unb, read_u64,
};

fn num(l: &Ledger, key: &[u8]) -> u64 {
    l.read(COIN_MODULE, key).map(read_u64).unwrap_or(0)
}

pub fn balance(l: &Ledger, a: &Address) -> u64 {
    num(l, &k_bal(a))
}

pub fn stake_of(l: &Ledger, a: &Address) -> u64 {
    num(l, &k_stake(a))
}

/// Coins waiting to be claimed after `Unstake`, as (release height, amount).
pub fn unbonding_of(l: &Ledger, a: &Address) -> Vec<(u64, u64)> {
    decode_unbonding(l.read(COIN_MODULE, &k_unb(a)).map(<[u8]>::to_vec))
}

/// Total ever created (rewards, faucet and devnet allocations).
pub fn minted(l: &Ledger) -> u64 {
    num(l, K_MINTED)
}

pub fn burned(l: &Ledger) -> u64 {
    num(l, K_BURNED)
}

pub fn config(l: &Ledger) -> Option<CoinConfig> {
    CoinConfig::decode(l.read(COIN_MODULE, K_CFG)?).ok()
}

/// Staked validators as (SEC1 public key, stake), in address order: the input for the
/// consensus validator set.
pub fn validators(l: &Ledger) -> Vec<(Vec<u8>, u64)> {
    l.scan(COIN_MODULE, P_STAKE)
        .into_iter()
        .filter_map(|(k, v)| {
            let addr = <Address>::try_from(k.get(P_STAKE.len()..)?).ok()?;
            let stake = read_u64(&v);
            let pk = l.read(COIN_MODULE, &k_pk(&addr))?.to_vec();
            (stake > 0).then_some((pk, stake))
        })
        .collect()
}

/// Checks the supply invariant: created - burned = balances + stake + unbonding, and
/// created never exceeds the cap.
pub fn audit(l: &Ledger) -> Result<(), String> {
    let cfg = config(l).ok_or("coin not initialised")?;
    let sum = |prefix: &[u8]| -> u128 {
        l.scan(COIN_MODULE, prefix)
            .iter()
            .map(|(_, v)| u128::from(read_u64(v)))
            .sum()
    };
    let unb: u128 = l
        .scan(COIN_MODULE, P_UNB)
        .iter()
        .flat_map(|(_, v)| decode_unbonding(Some(v.clone())))
        .map(|(_, a)| u128::from(a))
        .sum();
    let held = sum(P_BAL) + sum(P_STAKE) + unb;
    let (m, b) = (minted(l), burned(l));
    if m > cfg.cap {
        return Err(format!("minted {m} above cap {}", cfg.cap));
    }
    if u128::from(m) != u128::from(b) + held {
        return Err(format!(
            "supply mismatch: minted {m}, burned {b}, held {held}"
        ));
    }
    Ok(())
}
