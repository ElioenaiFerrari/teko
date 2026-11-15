//! The vote ledger (PoA, M-of-N): block headers, entry counts by kind and signatures.
//! Entry CONTENTS are not shown (ballots are encrypted anyway); the election board
//! with Merkle proofs stays in the Suite.

use std::collections::BTreeMap;

use serde::Serialize;
use teko_chain::{Chain, block_hash};
use teko_crypto::hex;

#[derive(Debug, Clone, Serialize)]
pub struct VoteBlock {
    pub height: u64,
    pub hash: String,
    pub prev_hash: String,
    pub timestamp: u64,
    pub round: u32,
    pub proposer_key: String,
    pub entries: usize,
    /// `manifest`, `transition`, `registration`, `ballot`, `result`, `dkg`, ...
    pub by_kind: BTreeMap<String, usize>,
    /// Valid validator signatures on the block and how many are required (M).
    pub signatures: usize,
    pub threshold: usize,
    pub log_size: u64,
    pub log_root: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct VotesSnapshot {
    pub available: bool,
    pub chain_id: String,
    pub threshold: usize,
    pub validators: usize,
    pub height: u64,
    /// Oldest first.
    pub blocks: Vec<VoteBlock>,
}

fn kind_name(k: u32) -> String {
    match k {
        1 => "manifest".into(),
        2 => "transition".into(),
        3 => "registration".into(),
        4 => "ballot".into(),
        5 => "result".into(),
        6 => "dkg".into(),
        other => format!("kind-{other}"),
    }
}

/// Summarizes a chain the client already verified (`NodeClient::sync_chain`).
pub fn snapshot(chain: &Chain) -> VotesSnapshot {
    let p = chain.profile();
    let blocks = chain
        .blocks()
        .iter()
        .map(|b| {
            let mut by_kind = BTreeMap::new();
            for e in &b.entries {
                *by_kind.entry(kind_name(e.kind)).or_insert(0) += 1;
            }
            VoteBlock {
                height: b.height,
                hash: hex::encode(&block_hash(b)),
                prev_hash: hex::encode(&b.prev_hash),
                timestamp: b.timestamp,
                round: b.round,
                proposer_key: hex::encode(&b.proposer),
                entries: b.entries.len(),
                by_kind,
                signatures: chain.count_signatures(b),
                threshold: p.threshold,
                log_size: b.log_size,
                log_root: hex::encode(&b.log_root),
            }
        })
        .collect();
    VotesSnapshot {
        available: true,
        chain_id: p.chain_id.clone(),
        threshold: p.threshold,
        validators: p.validators.len(),
        height: chain.height(),
        blocks,
    }
}
