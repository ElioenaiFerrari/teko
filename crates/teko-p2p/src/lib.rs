//! Authenticated, encrypted links between validators (ADR-0048). Transport only: no consensus rule
//! lives here. A link is a Noise XX session (`snow`) whose static key is BOUND to the node's P-256
//! consensus key by a signature inside the handshake, so the peer on the other end is proven to be the
//! institution whose key the validator set lists, on THIS chain and THIS genesis.

#![forbid(unsafe_code)]

mod error;
mod handshake;
mod identity;
mod limits;
mod records;
mod session;
#[cfg(test)]
mod tests;

pub use error::NetError;
pub use handshake::{accept, connect};
pub use identity::{IdentityProof, NetIdentity, PeerPolicy};
pub use limits::{ConnGuard, ConnLimiter, Reputation, TokenBucket};
pub use records::{MAX_LIFETIME_SECS, MergeOutcome, PeerBook, PeerRecord, valid_addr};
pub use session::{HANDSHAKE_TIMEOUT, Session};
