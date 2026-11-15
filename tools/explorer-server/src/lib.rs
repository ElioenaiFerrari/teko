//! Teko chain explorer server (ADR-0031): a local web page plus a **read-only** JSON
//! API over a chain this process verifies itself (`teko-explorer`), fed by the nodes
//! of a network in `dev/net`. The API holds no secret and writes nothing.

pub mod api;
pub mod state;
pub mod votes;

pub use api::{Response, handle};
pub use state::{Meta, NodeHealth, Shared, spawn_sync, sync_once};
