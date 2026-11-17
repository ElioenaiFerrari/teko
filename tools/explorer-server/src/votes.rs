//! The vote ledger (BFT among equal-power institutions, ADR-0034): block headers, entry counts
//! by kind and the commit certificate.
//! Entry CONTENTS are not shown (ballots are encrypted anyway); the election board
//! with Merkle proofs stays in the Suite.

use std::collections::BTreeMap;

use serde::Serialize;
use teko_chain::Chain;
use teko_crypto::hex;
use teko_node::{PeerLink, who_is};

/// Who signed (or proposed): the institution and the computer it runs on (from `peers.txt`).
#[derive(Debug, Clone, Serialize)]
pub struct SignerView {
    pub key: String,
    /// Position in `peers.txt`; `None` for a key that is not listed.
    pub index: Option<usize>,
    /// The host of the institution's peer address.
    pub host: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct VoteBlock {
    pub height: u64,
    pub hash: String,
    pub prev_hash: String,
    pub timestamp: u64,
    pub round: u32,
    pub proposer_key: String,
    pub proposer: SignerView,
    pub entries: usize,
    /// `manifest`, `transition`, `registration`, `ballot`, `result`, `dkg`, ...
    pub by_kind: BTreeMap<String, usize>,
    /// Precommits in the block's commit certificate (each from a distinct institution) and the
    /// quorum that makes a block final (more than 2/3 of the institutions).
    pub precommits: usize,
    /// The institutions whose precommits certify the block, by position.
    pub signers: Vec<SignerView>,
    pub quorum: usize,
    pub log_size: u64,
    pub log_root: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct VotesSnapshot {
    pub available: bool,
    pub chain_id: String,
    /// Precommits needed for a block to be final.
    pub quorum: usize,
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
pub fn snapshot(chain: &Chain, links: &[PeerLink]) -> VotesSnapshot {
    let view = |key: &[u8]| {
        let w = who_is(links, key);
        SignerView {
            key: hex::encode(key),
            index: w.index,
            host: w.host,
        }
    };
    let p = chain.profile();
    // equal power: more than 2/3 of the institutions IN FORCE (they change by rotation, ADR-0045)
    let in_force = chain.epoch_at(chain.height() + 1);
    let quorum = in_force.set.len() * 2 / 3 + 1;
    let blocks = chain
        .blocks()
        .iter()
        .filter_map(|d| {
            let b = d.block().ok()?;
            let mut by_kind = BTreeMap::new();
            for e in &b.entries {
                *by_kind.entry(kind_name(e.kind)).or_insert(0) += 1;
            }
            let (precommits, round) = d
                .commit
                .as_ref()
                .map_or((0, 0), |c| (c.precommits.len(), c.round));
            let mut signers: Vec<SignerView> = d
                .commit
                .as_ref()
                .map(|c| c.precommits.iter().map(|v| view(&v.validator)).collect())
                .unwrap_or_default();
            signers.sort_by_key(|s| s.index.unwrap_or(usize::MAX));
            Some(VoteBlock {
                height: b.height,
                hash: hex::encode(&b.id()),
                prev_hash: hex::encode(&b.prev_hash),
                timestamp: b.timestamp,
                round,
                proposer_key: hex::encode(&b.proposer),
                proposer: view(&b.proposer),
                entries: b.entries.len(),
                by_kind,
                precommits,
                signers,
                quorum,
                log_size: b.log_size,
                log_root: hex::encode(&b.log_root),
            })
        })
        .collect();
    VotesSnapshot {
        available: true,
        chain_id: p.chain_id.clone(),
        quorum,
        validators: in_force.set.len(),
        height: chain.height(),
        blocks,
    }
}
