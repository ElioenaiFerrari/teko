//! Property/asset module for `teko-ledger` (M4 of the ledger program, ADR-0022).
//!
//! * CLASSES (for example "imovel", "veiculo") are created at genesis and governed by
//!   a REGISTRAR: an M-of-N set of public keys (the "órgão").
//! * An asset is first REGISTERED by a registrar quorum; afterwards it changes hands
//!   only with THREE consents: the current owner, the buyer and the registrar quorum.
//! * LIENS (gravames) are added/released by the registrar quorum and block transfers.
//! * SWAP is atomic delivery-versus-payment: the asset moves to the buyer and the
//!   buyer pays the owner in the coin module, all or nothing.
//! * Consents are signatures over an INTENT bound to chain, class, asset, parties,
//!   price and the asset's VERSION, so they cannot be replayed after any change.
//!
//! LIMIT (the oracle problem): the ledger proves who consented, not that the real
//! world agrees. A registrar can lie; it is a trusted institution, only made
//! accountable by M-of-N and a public history. NOT AUDITED; no legal validity.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod devnet;
mod module;
mod query;
mod types;

pub use module::{ASSETS_MODULE, AssetsModule, genesis_class, module};
pub use query::{asset, assets_of, class_info};
pub use types::{
    Approval, AssetRec, ClassRec, Intent, IntentKind, Lien, LienOp, Op, RegisterOp, RegistrarsOp,
    ReleaseOp, SwapOp, TransferOp, intent_hash,
};
