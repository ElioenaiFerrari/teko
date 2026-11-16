//! Property/asset module for `teko-ledger` (M4 of the ledger program, ADR-0022).
//!
//! * CLASSES (for example "imovel", "veiculo") are created at genesis and governed by
//!   a REGISTRAR: an M-of-N set of public keys (the "órgão").
//! * An asset is first REGISTERED by a registrar quorum. Afterwards it changes hands by a DIRECT
//!   TRADE with a veto window (ADR-0036): the owner OFFERS it to one named buyer, the buyer
//!   ACCEPTS (the price waits in an escrow), the registrar quorum may VETO inside the class's
//!   window, and ANYONE finalizes after it (asset to the buyer, escrow to the seller, all or
//!   nothing). Each party sends its OWN transaction, so the transaction signature is the consent.
//! * LIENS (gravames) are added/released by the registrar quorum, block trades and cancel a
//!   pending one (the buyer is refunded). Registration and registrar changes stay by quorum.
//! * Quorum approvals are signatures over an INTENT bound to chain, class, asset, parties,
//!   price and the asset's VERSION, so they cannot be replayed after any change.
//! * The window is measured in BLOCK TIME (`Ctx::time`): with no empty blocks the height does
//!   not advance by itself.
//!
//! LIMIT (the oracle problem): the ledger proves who consented, not that the real
//! world agrees. A registrar can lie; it is a trusted institution, only made
//! accountable by M-of-N and a public history. NOT AUDITED; no legal validity.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod devnet;
mod metadata;
mod module;
mod query;
mod types;

pub use metadata::MetadataStore;
pub use module::{
    ASSETS_MODULE, AssetsModule, DEFAULT_WINDOW_SECS, META_HASH_LEN, escrow_address, genesis_class,
    genesis_class_with_window, module,
};
pub use query::{asset, assets_of, class_info, offer_of, offers};
pub use types::{
    AcceptOp, Approval, AssetRec, CancelOp, ClassRec, FinalizeOp, Intent, IntentKind, Lien, LienOp,
    OfferOp, OfferRec, Op, RegisterOp, RegistrarsOp, ReleaseOp, VetoOp, intent_hash,
};
