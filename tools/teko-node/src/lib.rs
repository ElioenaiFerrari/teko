//! Teko validator node and verified client (ADR-0014, ADR-0034).
//!
//! A node runs two BFT lanes side by side (`lane`): the VOTE chain (bulletin entries ordered by
//! the institutions, equal power) and, when configured, the ECONOMY chain. Nodes gossip, take
//! turns proposing and decide blocks with commit certificates; no block is ever made on a timer.
//! Clients NEVER trust a node: they download the blocks and re-verify every certificate
//! (`NodeClient::sync_chain`).
//!
//! Loopback only by default: the devnet runs on your machine. DEV/PROTOTYPE: the
//! wire is plain TCP without encryption or peer authentication beyond the block
//! signatures themselves (which are what matter for integrity).
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod client;
mod dkg;
mod economy;
mod error;
mod lane;
mod node;
mod setup;
mod votes;
pub mod wire;
mod world;

pub use client::{EconStatus, NodeClient, RemoteStatus};
pub use dkg::{load_share as load_tally_share, run_ceremony};
pub use economy::EconomyConfig;
pub use error::NodeError;
pub use node::{Node, NodeConfig, NodeHandle};
pub use setup::{
    DEVNET_CRED_TYPES, ECONOMY_FILE, NetFiles, init_economy, init_economy_with_issuer,
    init_network, load_class_registrar_keys, load_economy, load_network, load_orgmember_keys,
    load_registrar_keys, load_validator_key, node_dir, orgmember_file,
};
pub use world::NodeWorld;
