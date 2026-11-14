//! Teko device protocol: typed, versioned messages and framing.
//!
//! Invariant: no message defined here carries a PIN, seed, or private key.
#![cfg_attr(not(test), no_std)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

extern crate alloc;

mod crc;
mod error;
mod frame;
mod messages;

pub use error::ProtocolError;
pub use frame::{
    CRC_LEN, Frame, HEADER_LEN, MAGIC, MAX_CHUNK_PAYLOAD, MAX_PAYLOAD, PROTO_VERSION, parse_header,
};
pub use messages::*;
