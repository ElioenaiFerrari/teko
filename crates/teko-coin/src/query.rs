use prost::Message;
use teko_ledger::{Address, Ledger};

use crate::module::{
    COIN_MODULE, CoinConfig, K_BURNED, K_CFG, K_FOVER, K_FSET, K_MINTED, K_TSTAKE, P_BAL, P_STAKE,
    P_UNB, decode_unbonding, k_bal, k_pk, k_stake, k_unb, k_vk, k_vo, read_u64,
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

/// One validator of the consensus set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatorEntry {
    /// SEC1 CONSENSUS key (the node's key, not the wallet's, when linked).
    pub key: Vec<u8>,
    /// Address that receives the rewards and bears the penalty.
    pub owner: Address,
    /// Bonded stake (1 for founders: equal power during the founder phase).
    pub stake: u64,
}

/// Total bonded stake.
pub fn total_stake(l: &Ledger) -> u64 {
    num(l, K_TSTAKE)
}

/// Has the founder phase ended? (False while it runs, and also when there is none to end:
/// see `founder_phase_active`.)
pub fn founder_phase_over(l: &Ledger) -> bool {
    l.read(COIN_MODULE, K_FOVER).is_some()
}

/// A founder phase exists (genesis founders were set) and has not ended yet.
pub fn founder_phase_active(l: &Ledger) -> bool {
    l.read(COIN_MODULE, K_FSET).is_some() && !founder_phase_over(l)
}

/// Address that receives rewards for blocks proposed by consensus key `key`.
pub fn owner_of(l: &Ledger, key: &[u8]) -> Address {
    let a = teko_ledger::address_of(key);
    l.read(COIN_MODULE, &k_vo(&a))
        .and_then(|o| <Address>::try_from(o).ok())
        .unwrap_or(a)
}

fn founders(l: &Ledger) -> Vec<ValidatorEntry> {
    let Some(raw) = l.read(COIN_MODULE, K_FSET) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut at = 0usize;
    while let Some(len) = raw
        .get(at..at + 2)
        .map(|b| usize::from(u16::from_be_bytes([b[0], b[1]])))
    {
        at += 2;
        let Some(key) = raw.get(at..at + len) else {
            break;
        };
        at += len;
        out.push(ValidatorEntry {
            key: key.to_vec(),
            owner: owner_of(l, key),
            stake: 1,
        });
    }
    out
}

/// The consensus validator set implied by the CURRENT state, sorted by key (the order is
/// consensus-critical). During the founder phase: the genesis founders. Afterwards: the
/// stakers with at least `min_validator_stake`, at most `max_validators` (largest first).
/// If that would be empty, the founders (if any) remain.
pub fn validator_set(l: &Ledger) -> Vec<ValidatorEntry> {
    if founder_phase_active(l) {
        let mut f = founders(l);
        f.sort_by(|a, b| a.key.cmp(&b.key));
        return f;
    }
    let cfg = config(l);
    let min = cfg.as_ref().map_or(1, |c| c.min_validator_stake.max(1));
    let max = cfg.as_ref().map_or(0, |c| c.max_validators as usize);
    let mut all: Vec<ValidatorEntry> = l
        .scan(COIN_MODULE, P_STAKE)
        .into_iter()
        .filter_map(|(k, v)| {
            let owner = <Address>::try_from(k.get(P_STAKE.len()..)?).ok()?;
            let stake = read_u64(&v);
            if stake < min {
                return None;
            }
            let key = l
                .read(COIN_MODULE, &k_vk(&owner))
                .or_else(|| l.read(COIN_MODULE, &k_pk(&owner)))?
                .to_vec();
            Some(ValidatorEntry { key, owner, stake })
        })
        .collect();
    all.sort_by(|a, b| b.stake.cmp(&a.stake).then_with(|| a.key.cmp(&b.key)));
    if max > 0 {
        all.truncate(max);
    }
    if all.is_empty() {
        all = founders(l);
    }
    all.sort_by(|a, b| a.key.cmp(&b.key));
    all
}

/// Staked validators as (consensus key, stake), in key order: the input for the
/// consensus validator set once the founder phase is over.
pub fn validators(l: &Ledger) -> Vec<(Vec<u8>, u64)> {
    let mut v: Vec<(Vec<u8>, u64)> = l
        .scan(COIN_MODULE, P_STAKE)
        .into_iter()
        .filter_map(|(k, v)| {
            let owner = <Address>::try_from(k.get(P_STAKE.len()..)?).ok()?;
            let stake = read_u64(&v);
            let key = l
                .read(COIN_MODULE, &k_vk(&owner))
                .or_else(|| l.read(COIN_MODULE, &k_pk(&owner)))?
                .to_vec();
            (stake > 0).then_some((key, stake))
        })
        .collect();
    v.sort();
    v
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
