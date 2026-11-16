use prost::Message;
use teko_bft::Commit;
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

/// What the validators agree on (its encoding is the BFT payload). `log_size`/`log_root` are
/// the bulletin log AFTER the block's entries are appended, so a block pins the exact Merkle
/// state. Heights start at 1.
#[derive(Clone, PartialEq, Message)]
pub struct Block {
    #[prost(string, tag = "1")]
    pub chain_id: String,
    #[prost(uint64, tag = "2")]
    pub height: u64,
    /// `block_id` of the previous block (`genesis_hash` for the first).
    #[prost(bytes = "vec", tag = "3")]
    pub prev_hash: Vec<u8>,
    /// Unix seconds claimed by the PROPOSER. Each validator only votes for a time near its own
    /// clock and the time only moves forward (ADR-0034); it is not a trusted clock beyond that.
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
}

impl Block {
    /// Strict decoding: trailing bytes or a non-canonical encoding are refused.
    pub fn decode_strict(bytes: &[u8]) -> Result<Block, ChainError> {
        let b = Block::decode(bytes).map_err(|_| ChainError::Block("malformed block"))?;
        if b.encode_to_vec() != bytes {
            return Err(ChainError::Block("non-canonical block encoding"));
        }
        Ok(b)
    }

    /// The identity of this block (the BFT `block_id` of its payload), the next block's `prev_hash`.
    pub fn id(&self) -> Vec<u8> {
        teko_bft::block_id(&self.encode_to_vec())
    }
}

/// A final block: the payload and the proof that more than 2/3 of the institutions signed it.
#[derive(Clone, PartialEq, Message)]
pub struct DecidedBlock {
    #[prost(bytes = "vec", tag = "1")]
    pub payload: Vec<u8>,
    #[prost(message, optional, tag = "2")]
    pub commit: Option<Commit>,
}

impl DecidedBlock {
    pub fn block(&self) -> Result<Block, ChainError> {
        Block::decode_strict(&self.payload)
    }
}

/// The identity of a block, used as the next block's `prev_hash`.
pub fn block_hash(b: &Block) -> Vec<u8> {
    b.id()
}

/// `prev_hash` of the first block: ties the chain to its `chain_id`.
pub fn genesis_hash(chain_id: &str) -> [u8; 32] {
    tagged_hash(tags::CHAIN_GENESIS, &[chain_id.as_bytes()])
}
