//! Serializable views returned by the [`crate::Index`].

use serde::Serialize;
use teko_crypto::amount::format_amount;

/// An amount in ypy with its exact TEKO text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Amount {
    pub ypy: u64,
    pub text: String,
}

impl Amount {
    pub fn new(ypy: u64) -> Self {
        Self {
            ypy,
            text: format_amount(ypy),
        }
    }
}

/// What a transaction does, decoded from its module and payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TxKind {
    Transfer {
        to: String,
        amount: Amount,
    },
    Stake {
        amount: Amount,
    },
    Unstake {
        amount: Amount,
    },
    Claim,
    /// Penalty for equivocation; `validator` is the consensus key that was caught.
    Slash {
        validator: Option<String>,
    },
    Faucet,
    SetValidatorKey {
        key: String,
    },
    /// An assets-module operation (`register`, `transfer`, `swap`, `add_lien`, ...).
    Assets {
        op: String,
        class: String,
        asset: String,
    },
    Unknown {
        module: u32,
        payload_len: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EventView {
    pub height: u64,
    pub tx_id: String,
    pub module: String,
    pub topic: String,
    pub data_hex: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TxView {
    pub id: String,
    pub height: u64,
    pub index: usize,
    pub timestamp: u64,
    /// Wallet address (`tekow1…`) of the sender.
    pub sender: String,
    pub module: String,
    pub nonce: u64,
    pub fee: Amount,
    pub kind: TxKind,
    pub events: Vec<EventView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BlockSummary {
    pub height: u64,
    pub hash: String,
    pub timestamp: u64,
    pub proposer_owner: String,
    pub tx_count: usize,
    pub reward: Amount,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BlockView {
    pub height: u64,
    pub hash: String,
    pub prev_hash: String,
    pub state_root: String,
    pub timestamp: u64,
    /// Consensus key of the proposer and the wallet that received the reward.
    pub proposer_key: String,
    pub proposer_owner: String,
    /// Coins created by this block (the block reward, clamped by the cap).
    pub reward: Amount,
    pub fees: Amount,
    /// Round in which the block was decided and how many precommits certify it.
    pub round: u32,
    pub precommits: usize,
    pub txs: Vec<TxView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Unbonding {
    pub release_height: u64,
    pub amount: Amount,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AssetView {
    pub class: String,
    pub id: String,
    pub owner: String,
    pub version: u64,
    pub liens: Vec<String>,
    /// Hex of the 32-byte hash of the asset's off-chain metadata record (ADR-0032).
    pub metadata_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AccountView {
    pub address: String,
    /// False when this address never appeared in the chain (all zeros then).
    pub known: bool,
    pub balance: Amount,
    pub stake: Amount,
    pub unbonding: Vec<Unbonding>,
    pub nonce: u64,
    pub assets: Vec<AssetView>,
    pub tx_count: usize,
    /// Newest first.
    pub recent_txs: Vec<TxView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ValidatorView {
    pub key: String,
    pub owner: String,
    pub stake: Amount,
    /// Voting power in the BFT (stake quantized to 10.000 units).
    pub power: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClassView {
    pub name: String,
    pub threshold: u32,
    pub registrars: usize,
    pub version: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StatusView {
    pub chain_id: String,
    pub height: u64,
    pub tip_hash: String,
    pub state_root: String,
    pub minted: Amount,
    pub burned: Amount,
    pub cap: Amount,
    pub total_stake: Amount,
    pub next_reward: Amount,
    pub founder_phase_active: bool,
    pub founder_target: Amount,
    pub validators: usize,
    pub tx_total: usize,
    /// Every certificate was verified by this index (always true: a bad block is refused).
    pub verified: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SearchResult {
    Block { height: u64 },
    Tx { id: String },
    Account { address: String },
    Validator { key: String, owner: String },
    NotFound,
}
