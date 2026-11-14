//! Teko validator node and verified client (federated PoA, ADR-0014).
//!
//! Nodes keep a `teko_chain::Chain`, gossip entries, take turns proposing blocks
//! and collect M-of-N validator signatures. Clients NEVER trust a node: they
//! download the blocks and re-verify every signature (`NodeClient::sync_chain`).
//!
//! Loopback only by default: the devnet runs on your machine. DEV/PROTOTYPE: the
//! wire is plain TCP without encryption or peer authentication beyond the block
//! signatures themselves (which are what matter for integrity).
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod client;
mod error;
mod node;
mod setup;
pub mod wire;

pub use client::{NodeClient, RemoteStatus};
pub use error::NodeError;
pub use node::{Node, NodeConfig, NodeHandle};
pub use setup::{NetFiles, init_network, load_network, load_validator_key, node_dir};
