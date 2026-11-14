//! Wire format: `[u32 BE length][prost Msg]`, one request and one reply per
//! connection. Small on purpose; block integrity comes from signatures, not
//! from the transport.

use std::io::{Read, Write};

use prost::Message;
use teko_attestation::SignerSignature;
use teko_chain::BlockEntry;

use crate::error::NodeError;

pub const MAX_MSG: usize = 16 * 1024 * 1024;

pub const SUBMIT: u32 = 1; // client -> node: SubmitEntries (node forwards to peers)
pub const FORWARD: u32 = 2; // node -> node: SubmitEntries (NOT re-forwarded)
pub const STATUS: u32 = 3;
pub const GET_BLOCKS: u32 = 4;
pub const PROPOSAL: u32 = 5;
pub const COMMIT: u32 = 6;
pub const DKG_START: u32 = 7; // coordinator -> node: begin the key ceremony (round 1)
pub const DKG_PACKAGES: u32 = 8; // coordinator -> node: everyone's round-1 packages; node finishes
pub const DKG_SHARE: u32 = 9; // node -> node: the secret share for the requester (loopback only)
pub const DKG_REPLY: u32 = 104;
pub const ACK: u32 = 100;
pub const STATUS_REPLY: u32 = 101;
pub const BLOCKS_REPLY: u32 = 102;
pub const SIG_REPLY: u32 = 103;

#[derive(Clone, PartialEq, Message)]
pub struct Msg {
    #[prost(uint32, tag = "1")]
    pub kind: u32,
    #[prost(bytes = "vec", tag = "2")]
    pub body: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
pub struct SubmitEntries {
    #[prost(message, repeated, tag = "1")]
    pub entries: Vec<BlockEntry>,
}

#[derive(Clone, PartialEq, Message)]
pub struct Ack {
    #[prost(bool, tag = "1")]
    pub ok: bool,
    #[prost(string, tag = "2")]
    pub info: String,
    /// For SUBMIT: how many entries were queued (the rest were duplicates).
    #[prost(uint32, tag = "3")]
    pub accepted: u32,
}

#[derive(Clone, PartialEq, Message)]
pub struct StatusReply {
    #[prost(string, tag = "1")]
    pub chain_id: String,
    #[prost(uint64, tag = "2")]
    pub height: u64,
    #[prost(bytes = "vec", tag = "3")]
    pub tip_hash: Vec<u8>,
    #[prost(uint64, tag = "4")]
    pub log_size: u64,
    #[prost(bytes = "vec", tag = "5")]
    pub log_root: Vec<u8>,
    #[prost(uint32, tag = "6")]
    pub mempool: u32,
    #[prost(uint32, tag = "7")]
    pub peers_online: u32,
    #[prost(uint32, tag = "8")]
    pub peers_total: u32,
    #[prost(uint32, tag = "9")]
    pub validator_index: u32,
    /// Conflicting blocks for one height, both with a valid quorum (V19).
    #[prost(uint32, tag = "10")]
    pub equivocations: u32,
}

#[derive(Clone, PartialEq, Message)]
pub struct GetBlocks {
    #[prost(uint64, tag = "1")]
    pub from: u64,
    #[prost(uint32, tag = "2")]
    pub max: u32,
}

#[derive(Clone, PartialEq, Message)]
pub struct BlocksReply {
    #[prost(bytes = "vec", repeated, tag = "1")]
    pub blocks: Vec<Vec<u8>>,
    #[prost(uint64, tag = "2")]
    pub height: u64,
}

#[derive(Clone, PartialEq, Message)]
pub struct BlockMsg {
    #[prost(bytes = "vec", tag = "1")]
    pub block: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
pub struct SigReply {
    #[prost(message, optional, tag = "1")]
    pub signature: Option<SignerSignature>,
    #[prost(string, tag = "2")]
    pub reason: String,
}

pub fn msg<M: Message>(kind: u32, body: &M) -> Msg {
    Msg {
        kind,
        body: body.encode_to_vec(),
    }
}

pub fn write_msg(w: &mut impl Write, m: &Msg) -> Result<(), NodeError> {
    let bytes = m.encode_to_vec();
    if bytes.len() > MAX_MSG {
        return Err(NodeError::Protocol("message too large".into()));
    }
    w.write_all(&(bytes.len() as u32).to_be_bytes())?;
    w.write_all(&bytes)?;
    w.flush()?;
    Ok(())
}

pub fn read_msg(r: &mut impl Read) -> Result<Msg, NodeError> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len)?;
    let n = u32::from_be_bytes(len) as usize;
    if n > MAX_MSG {
        return Err(NodeError::Protocol("message too large".into()));
    }
    let mut buf = vec![0u8; n];
    r.read_exact(&mut buf)?;
    Msg::decode(buf.as_slice()).map_err(|_| NodeError::Protocol("malformed message".into()))
}

pub fn decode<M: Message + Default>(m: &Msg) -> Result<M, NodeError> {
    M::decode(m.body.as_slice()).map_err(|_| NodeError::Protocol("malformed body".into()))
}

use teko_ballot_crypto::dkg_wire::Round1Msg;

#[derive(Clone, PartialEq, Message)]
pub struct DkgStart {
    #[prost(bytes = "vec", tag = "1")]
    pub session: Vec<u8>,
    #[prost(uint32, tag = "2")]
    pub n: u32,
    #[prost(uint32, tag = "3")]
    pub threshold: u32,
}

#[derive(Clone, PartialEq, Message)]
pub struct DkgPackages {
    #[prost(bytes = "vec", tag = "1")]
    pub session: Vec<u8>,
    #[prost(uint32, tag = "2")]
    pub threshold: u32,
    #[prost(message, repeated, tag = "3")]
    pub packages: Vec<Round1Msg>,
}

#[derive(Clone, PartialEq, Message)]
pub struct DkgShareReq {
    /// 1-based index of the requesting validator.
    #[prost(uint32, tag = "1")]
    pub requester: u32,
}

#[derive(Clone, PartialEq, Message)]
pub struct DkgReply {
    #[prost(bool, tag = "1")]
    pub ok: bool,
    #[prost(string, tag = "2")]
    pub info: String,
    #[prost(message, optional, tag = "3")]
    pub round1: Option<Round1Msg>,
    /// SECRET share (only in a DKG_SHARE reply, node to node).
    #[prost(bytes = "vec", tag = "4")]
    pub share: Vec<u8>,
    /// The election public key a node derived (DKG_PACKAGES reply).
    #[prost(bytes = "vec", tag = "5")]
    pub group_key: Vec<u8>,
}
