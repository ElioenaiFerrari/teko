use prost::Message;
use teko_bft::Evidence;
use teko_crypto::{tagged_hash, tags};
use teko_ledger::{Address, Ctx, Ledger, Module, ModuleError, address_of};

/// Module id of the coin in `Tx::module`.
pub const COIN_MODULE: u32 = 1;

pub(crate) const K_CFG: &[u8] = b"cfg";
pub(crate) const K_MINTED: &[u8] = b"minted";
pub(crate) const K_BURNED: &[u8] = b"burned";

// Key prefixes end with ':' so a prefix scan never matches another family
// (for example "bal:" versus the "burned" counter).
pub(crate) const P_BAL: &[u8] = b"bal:";
pub(crate) const P_STAKE: &[u8] = b"stk:";
pub(crate) const P_UNB: &[u8] = b"unb:";
const P_PK: &[u8] = b"pk:";
const P_FAUCET: &[u8] = b"fct:";
const P_SLASHED: &[u8] = b"slx:";

pub(crate) fn k_bal(a: &Address) -> Vec<u8> {
    [P_BAL, a].concat()
}
pub(crate) fn k_stake(a: &Address) -> Vec<u8> {
    [P_STAKE, a].concat()
}
pub(crate) fn k_pk(a: &Address) -> Vec<u8> {
    [P_PK, a].concat()
}
pub(crate) fn k_unb(a: &Address) -> Vec<u8> {
    [P_UNB, a].concat()
}
fn k_faucet(a: &Address) -> Vec<u8> {
    [P_FAUCET, a].concat()
}

/// Economic parameters, fixed at genesis. Amounts are in the smallest unit.
#[derive(Clone, PartialEq, Eq, Message)]
pub struct CoinConfig {
    /// Hard cap on everything ever created.
    #[prost(uint64, tag = "1")]
    pub cap: u64,
    /// Block reward of the first epoch.
    #[prost(uint64, tag = "2")]
    pub initial_reward: u64,
    /// The reward halves every `halving_interval` blocks.
    #[prost(uint64, tag = "3")]
    pub halving_interval: u64,
    #[prost(uint64, tag = "4")]
    pub fee_per_byte: u64,
    /// Blocks coins stay slashable after `Unstake`.
    #[prost(uint64, tag = "5")]
    pub unbonding_period: u64,
    /// Share (basis points, 0..=10000) burned on equivocation.
    #[prost(uint32, tag = "6")]
    pub slash_bps: u32,
    /// Devnet faucet: amount per request (0 = disabled).
    #[prost(uint64, tag = "7")]
    pub faucet_amount: u64,
    /// Blocks between two faucet requests by the same address.
    #[prost(uint64, tag = "8")]
    pub faucet_cooldown: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoinError {
    InvalidConfig(&'static str),
    /// Allocations and the faucet exist only on the devnet.
    OnlyDevnet,
    AllocationExceedsCap,
    Ledger(String),
}

/// Operations a coin transaction can carry (`payload` = op byte + arguments).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    Transfer { to: Address, amount: u64 },
    Stake { amount: u64 },
    Unstake { amount: u64 },
    Claim,
    Slash { evidence: Vec<u8> },
    Faucet,
}

impl Op {
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Op::Transfer { to, amount } => [&[0u8][..], to, &amount.to_be_bytes()].concat(),
            Op::Stake { amount } => [&[1u8][..], &amount.to_be_bytes()].concat(),
            Op::Unstake { amount } => [&[2u8][..], &amount.to_be_bytes()].concat(),
            Op::Claim => vec![3],
            Op::Slash { evidence } => [&[4u8][..], evidence].concat(),
            Op::Faucet => vec![5],
        }
    }

    pub fn decode(bytes: &[u8]) -> Result<Op, ModuleError> {
        let bad = || ModuleError::new("malformed coin payload");
        let (op, rest) = bytes.split_first().ok_or_else(bad)?;
        let amount = |b: &[u8]| -> Result<u64, ModuleError> {
            <[u8; 8]>::try_from(b)
                .map(u64::from_be_bytes)
                .map_err(|_| bad())
        };
        match op {
            0 => {
                let to = rest.get(..32).ok_or_else(bad)?;
                let amt = rest.get(32..).ok_or_else(bad)?;
                Ok(Op::Transfer {
                    to: <[u8; 32]>::try_from(to).map_err(|_| bad())?,
                    amount: amount(amt)?,
                })
            }
            1 => Ok(Op::Stake {
                amount: amount(rest)?,
            }),
            2 => Ok(Op::Unstake {
                amount: amount(rest)?,
            }),
            3 if rest.is_empty() => Ok(Op::Claim),
            4 => Ok(Op::Slash {
                evidence: rest.to_vec(),
            }),
            5 if rest.is_empty() => Ok(Op::Faucet),
            _ => Err(bad()),
        }
    }
}

/// Reward of block `height` (heights start at 1), before the cap clamp.
pub fn reward_at(cfg: &CoinConfig, height: u64) -> u64 {
    if cfg.halving_interval == 0 || height == 0 {
        return 0;
    }
    let epoch = (height - 1) / cfg.halving_interval;
    if epoch >= 64 {
        0
    } else {
        cfg.initial_reward >> epoch
    }
}

/// Writes the genesis state. `devnet` must be true to use the faucet or allocations.
pub fn genesis(
    ledger: &mut Ledger,
    cfg: CoinConfig,
    allocations: &[(Address, u64)],
    devnet: bool,
) -> Result<(), CoinError> {
    if cfg.cap == 0 {
        return Err(CoinError::InvalidConfig("cap must be positive"));
    }
    if cfg.halving_interval == 0 {
        return Err(CoinError::InvalidConfig(
            "halving_interval must be positive",
        ));
    }
    if cfg.slash_bps > 10_000 {
        return Err(CoinError::InvalidConfig("slash_bps above 10000"));
    }
    if (cfg.faucet_amount > 0 || !allocations.is_empty()) && !devnet {
        return Err(CoinError::OnlyDevnet);
    }
    let mut minted = 0u64;
    for (addr, amount) in allocations {
        minted = minted
            .checked_add(*amount)
            .ok_or(CoinError::AllocationExceedsCap)?;
        if minted > cfg.cap {
            return Err(CoinError::AllocationExceedsCap);
        }
        let prev = ledger.read(COIN_MODULE, &k_bal(addr)).map(<[u8]>::to_vec);
        let cur = prev.as_deref().map(read_u64).unwrap_or(0);
        let new = cur
            .checked_add(*amount)
            .ok_or(CoinError::AllocationExceedsCap)?;
        put(ledger, &k_bal(addr), new)?;
    }
    put(ledger, K_MINTED, minted)?;
    put(ledger, K_BURNED, 0)?;
    ledger
        .genesis_put(COIN_MODULE, K_CFG, cfg.encode_to_vec())
        .map_err(|e| CoinError::Ledger(format!("{e:?}")))
}

fn put(ledger: &mut Ledger, key: &[u8], v: u64) -> Result<(), CoinError> {
    ledger
        .genesis_put(COIN_MODULE, key, v.to_be_bytes().to_vec())
        .map_err(|e| CoinError::Ledger(format!("{e:?}")))
}

pub(crate) fn read_u64(b: &[u8]) -> u64 {
    <[u8; 8]>::try_from(b).map(u64::from_be_bytes).unwrap_or(0)
}

/// The module to register in a `Ledger` (also designate it as the fee module).
pub fn module() -> Box<dyn Module> {
    Box::new(CoinModule)
}

pub struct CoinModule;

fn get_u64(ctx: &Ctx<'_, '_>, key: &[u8]) -> u64 {
    ctx.get(key).map(|v| read_u64(&v)).unwrap_or(0)
}

fn set_u64(ctx: &mut Ctx<'_, '_>, key: &[u8], v: u64) {
    if v == 0 {
        ctx.delete(key);
    } else {
        ctx.set(key, v.to_be_bytes().to_vec());
    }
}

fn load_cfg(ctx: &Ctx<'_, '_>) -> Result<CoinConfig, ModuleError> {
    let raw = ctx
        .get(K_CFG)
        .ok_or_else(|| ModuleError::new("coin not initialised"))?;
    CoinConfig::decode(raw.as_slice()).map_err(|_| ModuleError::new("corrupt coin config"))
}

fn add(ctx: &mut Ctx<'_, '_>, key: &[u8], amount: u64) -> Result<(), ModuleError> {
    let v = get_u64(ctx, key)
        .checked_add(amount)
        .ok_or_else(|| ModuleError::new("overflow"))?;
    set_u64(ctx, key, v);
    Ok(())
}

fn sub(ctx: &mut Ctx<'_, '_>, key: &[u8], amount: u64) -> Result<(), ModuleError> {
    let v = get_u64(ctx, key)
        .checked_sub(amount)
        .ok_or_else(|| ModuleError::new("insufficient funds"))?;
    set_u64(ctx, key, v);
    Ok(())
}

/// Creates `amount` (clamped to the cap) for `to`. Returns what was really minted.
fn mint(
    ctx: &mut Ctx<'_, '_>,
    cfg: &CoinConfig,
    to: &Address,
    amount: u64,
) -> Result<u64, ModuleError> {
    let minted = get_u64(ctx, K_MINTED);
    let room = cfg.cap.saturating_sub(minted);
    let amt = amount.min(room);
    if amt == 0 {
        return Ok(0);
    }
    set_u64(ctx, K_MINTED, minted + amt);
    add(ctx, &k_bal(to), amt)?;
    Ok(amt)
}

fn read_unbonding(raw: Option<Vec<u8>>) -> Vec<(u64, u64)> {
    raw.map(|v| {
        let (pairs, _) = v.as_chunks::<16>();
        pairs
            .iter()
            .map(|c| (read_u64(&c[..8]), read_u64(&c[8..])))
            .collect()
    })
    .unwrap_or_default()
}

pub(crate) fn encode_unbonding(list: &[(u64, u64)]) -> Vec<u8> {
    list.iter()
        .flat_map(|(h, a)| [h.to_be_bytes(), a.to_be_bytes()].concat())
        .collect()
}

pub(crate) fn decode_unbonding(raw: Option<Vec<u8>>) -> Vec<(u64, u64)> {
    read_unbonding(raw)
}

fn save_unbonding(ctx: &mut Ctx<'_, '_>, addr: &Address, list: &[(u64, u64)]) {
    if list.is_empty() {
        ctx.delete(&k_unb(addr));
    } else {
        ctx.set(&k_unb(addr), encode_unbonding(list));
    }
}

impl Module for CoinModule {
    fn id(&self) -> u32 {
        COIN_MODULE
    }

    fn begin_block(&self, ctx: &mut Ctx<'_, '_>) -> Result<(), ModuleError> {
        let cfg = load_cfg(ctx)?;
        let reward = reward_at(&cfg, ctx.height);
        let proposer = ctx.proposer;
        let minted = mint(ctx, &cfg, &proposer, reward)?;
        if minted > 0 {
            ctx.emit("reward", minted.to_be_bytes().to_vec());
        }
        Ok(())
    }

    fn min_fee(&self, ctx: &Ctx<'_, '_>, tx_len: usize) -> u64 {
        load_cfg(ctx)
            .map(|c| c.fee_per_byte.saturating_mul(tx_len as u64))
            .unwrap_or(u64::MAX)
    }

    fn charge_fee(&self, ctx: &mut Ctx<'_, '_>, fee: u64) -> Result<(), ModuleError> {
        if fee == 0 {
            return Ok(());
        }
        let (from, to) = (ctx.sender, ctx.proposer);
        sub(ctx, &k_bal(&from), fee).map_err(|_| ModuleError::new("cannot pay the fee"))?;
        add(ctx, &k_bal(&to), fee)
    }

    fn execute(&self, ctx: &mut Ctx<'_, '_>, payload: &[u8]) -> Result<(), ModuleError> {
        let cfg = load_cfg(ctx)?;
        let me = ctx.sender;
        match Op::decode(payload)? {
            Op::Transfer { to, amount } => {
                if amount == 0 {
                    return Err(ModuleError::new("amount must be positive"));
                }
                sub(ctx, &k_bal(&me), amount)?;
                add(ctx, &k_bal(&to), amount)
            }
            Op::Stake { amount } => {
                if amount == 0 {
                    return Err(ModuleError::new("amount must be positive"));
                }
                sub(ctx, &k_bal(&me), amount)?;
                add(ctx, &k_stake(&me), amount)?;
                ctx.set(&k_pk(&me), ctx.sender_key.to_vec());
                Ok(())
            }
            Op::Unstake { amount } => {
                if amount == 0 {
                    return Err(ModuleError::new("amount must be positive"));
                }
                sub(ctx, &k_stake(&me), amount)?;
                let mut list = read_unbonding(ctx.get(&k_unb(&me)));
                let release = ctx
                    .height
                    .checked_add(cfg.unbonding_period)
                    .ok_or_else(|| ModuleError::new("overflow"))?;
                list.push((release, amount));
                save_unbonding(ctx, &me, &list);
                Ok(())
            }
            Op::Claim => {
                let list = read_unbonding(ctx.get(&k_unb(&me)));
                let (ready, waiting): (Vec<_>, Vec<_>) =
                    list.into_iter().partition(|(h, _)| *h <= ctx.height);
                let total = ready
                    .iter()
                    .try_fold(0u64, |a, (_, x)| a.checked_add(*x))
                    .ok_or_else(|| ModuleError::new("overflow"))?;
                if total == 0 {
                    return Err(ModuleError::new("nothing to claim yet"));
                }
                save_unbonding(ctx, &me, &waiting);
                add(ctx, &k_bal(&me), total)
            }
            Op::Faucet => {
                if cfg.faucet_amount == 0 {
                    return Err(ModuleError::new("faucet disabled"));
                }
                if let Some(last) = ctx.get(&k_faucet(&me)).map(|v| read_u64(&v))
                    && ctx.height < last.saturating_add(cfg.faucet_cooldown)
                {
                    return Err(ModuleError::new("faucet cooldown"));
                }
                let got = mint(ctx, &cfg, &me, cfg.faucet_amount)?;
                if got == 0 {
                    return Err(ModuleError::new("cap reached"));
                }
                ctx.set(&k_faucet(&me), ctx.height.to_be_bytes().to_vec());
                // Labeled so nobody mistakes it for real emission.
                ctx.emit("faucet-TEST-COINS", got.to_be_bytes().to_vec());
                Ok(())
            }
            Op::Slash { evidence } => slash(ctx, &cfg, &evidence),
        }
    }
}

fn slash(ctx: &mut Ctx<'_, '_>, cfg: &CoinConfig, raw: &[u8]) -> Result<(), ModuleError> {
    let ev = Evidence::decode(raw).map_err(|_| ModuleError::new("malformed evidence"))?;
    if !ev.verify(ctx.chain_id) {
        return Err(ModuleError::new("invalid evidence"));
    }
    let (Some(a), Some(validator)) = (&ev.a, ev.validator()) else {
        return Err(ModuleError::new("invalid evidence"));
    };
    // Evidence must be recent enough that the bonded coins are still slashable.
    if a.height > ctx.height || ctx.height > a.height.saturating_add(cfg.unbonding_period) {
        return Err(ModuleError::new("evidence outside the slashing window"));
    }
    let marker = tagged_hash(
        tags::COIN_SLASH,
        &[
            validator,
            &a.height.to_be_bytes(),
            &a.round.to_be_bytes(),
            &a.kind.to_be_bytes(),
        ],
    );
    let marker_key = [P_SLASHED, &marker].concat();
    if ctx.get(&marker_key).is_some() {
        return Err(ModuleError::new("already slashed for this offence"));
    }
    let addr = address_of(validator);
    let stake = get_u64(ctx, &k_stake(&addr));
    let mut unb = read_unbonding(ctx.get(&k_unb(&addr)));
    let at_risk = unb
        .iter()
        .fold(u128::from(stake), |s, (_, x)| s + u128::from(*x));
    let mut penalty = (at_risk * u128::from(cfg.slash_bps) / 10_000) as u64;
    let total_penalty = penalty;
    let from_stake = penalty.min(stake);
    set_u64(ctx, &k_stake(&addr), stake - from_stake);
    penalty -= from_stake;
    for (_, amt) in unb.iter_mut().rev() {
        if penalty == 0 {
            break;
        }
        let take = penalty.min(*amt);
        *amt -= take;
        penalty -= take;
    }
    unb.retain(|(_, a)| *a > 0);
    save_unbonding(ctx, &addr, &unb);
    let burned = get_u64(ctx, K_BURNED).saturating_add(total_penalty);
    set_u64(ctx, K_BURNED, burned);
    ctx.set(&marker_key, vec![1]);
    ctx.emit("slashed", total_penalty.to_be_bytes().to_vec());
    Ok(())
}
