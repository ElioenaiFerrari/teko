//! Fixed-cap coin module for `teko-ledger` (M3 of the ledger program, ADR-0021).
//!
//! * FIXED CAP: new coins appear only as block rewards (halving), as a devnet
//!   faucet or as devnet genesis allocations, and the total created can never
//!   exceed `cap`. No premine outside the devnet.
//! * FEES: `fee_per_byte * encoded_len`, paid to the block proposer.
//! * STAKE: bond coins to become a validator; unstaking waits `unbonding_period`
//!   blocks, during which the coins can still be slashed.
//! * SLASHING: a verified BFT equivocation `Evidence` burns `slash_bps` of the
//!   validator's stake and unbonding coins, once per (validator, height, round, kind).
//!
//! Supply invariant (checked by `audit`): created - burned = balances + stake + unbonding.
//!
//! NOT AUDITED. Which validators exist at mainnet start is an OPEN DECISION (ADR-0021).
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod module;
mod query;

pub use module::{COIN_MODULE, CoinConfig, CoinError, CoinModule, Op, genesis, module, reward_at};
pub use query::{audit, balance, burned, config, minted, stake_of, unbonding_of, validators};
