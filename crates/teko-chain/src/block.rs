use prost::Message;
use teko_attestation::SignerSignature;
use teko_bulletin::{Entry, EntryKind};
use teko_crypto::{tagged_hash, tags};

use crate::error::ChainError;

/// Entries per block and bytes per block (a block is gossiped in one message).
pub const MAX_BLOCK_ENTRIES: usize = 5_000;
pub const MAX_BLOCK_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, PartialEq, Message)]
pub struct BlockEntry {
    #[prost(uint32, tag = "1")]
    pub kind: u32,
    #[prost(bytes = "vec", tag = "2")]
    pub payload: Vec<u8>,
}

impl BlockEntry {
    pub fn to_entry(&self) -> Result<Entry, ChainError> {
        let kind = u8::try_from(self.kind).map_err(|_| ChainError::Block("entry kind > 255"))?;
        Ok(Entry::new(EntryKind(kind), self.payload.clone()))
    }
}

impl From<&Entry> for BlockEntry {
    fn from(e: &Entry) -> Self {
        Self {
            kind: u32::from(e.kind.0),
            payload: e.payload.clone(),
        }
    }
}

/// One block. The signed content is `header_hash()`: everything except the
/// signatures themselves. `log_size`/`log_root` are the bulletin log AFTER the
/// block's entries are appended, so a block pins the exact Merkle state.
#[derive(Clone, PartialEq, Message)]
pub struct Block {
    #[prost(string, tag = "1")]
    pub chain_id: String,
    #[prost(uint64, tag = "2")]
    pub height: u64,
    #[prost(bytes = "vec", tag = "3")]
    pub prev_hash: Vec<u8>,
    /// Unix seconds claimed by the proposer (NOT a trusted clock).
    #[prost(uint64, tag = "4")]
    pub timestamp: u64,
    #[prost(bytes = "vec", tag = "5")]
    pub proposer: Vec<u8>,
    #[prost(message, repeated, tag = "6")]
    pub entries: Vec<BlockEntry>,
    #[prost(uint64, tag = "7")]
    pub log_size: u64,
    #[prost(bytes = "vec", tag = "8")]
    pub log_root: Vec<u8>,
    #[prost(message, repeated, tag = "9")]
    pub signatures: Vec<SignerSignature>,
    /// View change: 0 normally; raised when the proposer of the turn is silent, so
    /// the next validator in the rotation may propose for the same height.
    #[prost(uint32, tag = "10")]
    pub round: u32,
}

fn entries_hash(entries: &[BlockEntry]) -> [u8; 32] {
    let kinds: Vec<[u8; 4]> = entries.iter().map(|e| e.kind.to_be_bytes()).collect();
    let mut parts: Vec<&[u8]> = Vec::with_capacity(entries.len() * 2);
    for (e, k) in entries.iter().zip(&kinds) {
        parts.push(k);
        parts.push(&e.payload);
    }
    tagged_hash(tags::BLOCK_ENTRIES, &parts)
}

impl Block {
    /// What validators sign. Covers every field but the signatures.
    pub fn header_hash(&self) -> [u8; 32] {
        tagged_hash(
            tags::BLOCK,
            &[
                self.chain_id.as_bytes(),
                &self.height.to_be_bytes(),
                &self.prev_hash,
                &self.timestamp.to_be_bytes(),
                &self.proposer,
                &self.round.to_be_bytes(),
                &entries_hash(&self.entries),
                &self.log_size.to_be_bytes(),
                &self.log_root,
            ],
        )
    }
}

/// The identity of a block, used as the next block's `prev_hash`.
pub fn block_hash(b: &Block) -> [u8; 32] {
    b.header_hash()
}

/// `prev_hash` of the first block: ties the chain to its `chain_id`.
pub fn genesis_hash(chain_id: &str) -> [u8; 32] {
    tagged_hash(tags::CHAIN_GENESIS, &[chain_id.as_bytes()])
}
