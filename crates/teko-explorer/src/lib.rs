//! Chain explorer index (ADR-0031). It rebuilds the economy chain from the final
//! blocks, **verifying every commit certificate itself** (it trusts no node), and
//! answers explorer queries: blocks, transactions with their receipts and events,
//! accounts, validators, assets and supply. No I/O: a server or a CLI feeds it blocks
//! (see `tools/explorer-server`) and serializes the views to JSON.
//!
//! Amounts are integers in **ypy**; every [`Amount`] also carries its TEKO text
//! (ADR-0029), formatted exactly.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod index;
mod model;

pub use index::{ExplorerError, Index};
pub use model::*;
