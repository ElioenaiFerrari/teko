use prost::Message;
use teko_crypto::{SigningKey, public_key_bytes, sign_tagged, tags};
use teko_ledger::Address;

#[derive(Clone, PartialEq, Eq, Message)]
pub struct Lien {
    #[prost(uint64, tag = "1")]
    pub id: u64,
    #[prost(string, tag = "2")]
    pub kind: String,
}

#[derive(Clone, PartialEq, Eq, Message)]
pub struct AssetRec {
    #[prost(bytes = "vec", tag = "1")]
    pub owner: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    pub metadata_hash: Vec<u8>,
    /// Bumped by every change; consents are bound to it.
    #[prost(uint64, tag = "3")]
    pub version: u64,
    #[prost(message, repeated, tag = "4")]
    pub liens: Vec<Lien>,
    #[prost(uint64, tag = "5")]
    pub next_lien: u64,
}

#[derive(Clone, PartialEq, Eq, Message)]
pub struct ClassRec {
    /// SEC1 public keys of the registrar members.
    #[prost(bytes = "vec", repeated, tag = "1")]
    pub registrars: Vec<Vec<u8>>,
    #[prost(uint32, tag = "2")]
    pub threshold: u32,
    #[prost(uint64, tag = "3")]
    pub version: u64,
    /// Seconds a pending trade of this class waits before anyone can finalize it; inside this
    /// window the registrar quorum may veto (ADR-0036).
    #[prost(uint64, tag = "4")]
    pub window_secs: u64,
}

/// A trade in progress for one asset (ADR-0036): the owner OFFERED it to a named buyer; once the
/// buyer ACCEPTED, the price waits in the escrow until `release_at`.
#[derive(Clone, PartialEq, Eq, Message)]
pub struct OfferRec {
    #[prost(bytes = "vec", tag = "1")]
    pub buyer: Vec<u8>,
    #[prost(uint64, tag = "2")]
    pub price: u64,
    /// The asset version the offer is bound to.
    #[prost(uint64, tag = "3")]
    pub version: u64,
    #[prost(bytes = "vec", tag = "4")]
    pub metadata_hash: Vec<u8>,
    #[prost(bool, tag = "5")]
    pub accepted: bool,
    /// Block time (unix seconds) from which anyone may finalize; 0 until accepted.
    #[prost(uint64, tag = "6")]
    pub release_at: u64,
}

/// A signature of one party over an intent.
#[derive(Clone, PartialEq, Eq, Message)]
pub struct Approval {
    #[prost(bytes = "vec", tag = "1")]
    pub key: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    pub signature: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum IntentKind {
    Register = 1,
    AddLien = 4,
    ReleaseLien = 5,
    UpdateRegistrars = 6,
    /// The registrar quorum vetoes a pending trade (ADR-0036).
    Veto = 7,
}

/// What a consent signs: everything that matters about one action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Intent {
    pub kind: IntentKind,
    pub class: Vec<u8>,
    pub asset: Vec<u8>,
    /// Owner / seller.
    pub from: Address,
    /// Buyer (zero when not applicable).
    pub to: Address,
    /// Price in coin units (Swap only).
    pub amount: u64,
    /// Asset version (class version for `UpdateRegistrars`).
    pub version: u64,
    /// Kind-specific data (metadata hash, lien kind or id, new registrar set).
    pub extra: Vec<u8>,
}

impl Intent {
    fn base(kind: IntentKind, class: &[u8], asset: &[u8]) -> Intent {
        Intent {
            kind,
            class: class.to_vec(),
            asset: asset.to_vec(),
            from: [0; 32],
            to: [0; 32],
            amount: 0,
            version: 0,
            extra: Vec::new(),
        }
    }

    pub fn register(class: &[u8], asset: &[u8], owner: Address, metadata_hash: &[u8]) -> Intent {
        Intent {
            from: owner,
            extra: metadata_hash.to_vec(),
            ..Self::base(IntentKind::Register, class, asset)
        }
    }

    /// The registrar quorum vetoes the pending trade of `asset` (owner `from`, named buyer `to`, `price`),
    /// for a public `reason`. The signature covers all of that, so it cannot be moved to another trade.
    pub fn veto(
        class: &[u8],
        asset: &[u8],
        owner: Address,
        buyer: Address,
        price: u64,
        version: u64,
        reason: &str,
    ) -> Intent {
        Intent {
            from: owner,
            to: buyer,
            amount: price,
            version,
            extra: reason.as_bytes().to_vec(),
            ..Self::base(IntentKind::Veto, class, asset)
        }
    }

    pub fn add_lien(
        class: &[u8],
        asset: &[u8],
        owner: Address,
        version: u64,
        kind: &str,
    ) -> Intent {
        Intent {
            from: owner,
            version,
            extra: kind.as_bytes().to_vec(),
            ..Self::base(IntentKind::AddLien, class, asset)
        }
    }

    pub fn release_lien(
        class: &[u8],
        asset: &[u8],
        owner: Address,
        version: u64,
        lien_id: u64,
    ) -> Intent {
        Intent {
            from: owner,
            version,
            extra: lien_id.to_be_bytes().to_vec(),
            ..Self::base(IntentKind::ReleaseLien, class, asset)
        }
    }

    pub fn update_registrars(
        class: &[u8],
        class_version: u64,
        registrars: &[Vec<u8>],
        threshold: u32,
    ) -> Intent {
        let mut extra = threshold.to_be_bytes().to_vec();
        for r in registrars {
            extra.extend_from_slice(&(r.len() as u32).to_be_bytes());
            extra.extend_from_slice(r);
        }
        Intent {
            version: class_version,
            extra,
            ..Self::base(IntentKind::UpdateRegistrars, class, &[])
        }
    }
}

pub fn intent_hash(chain_id: &str, i: &Intent) -> [u8; 32] {
    teko_crypto::ledger::asset_intent_hash(
        chain_id,
        i.kind as u8,
        &i.class,
        &i.asset,
        &i.from,
        &i.to,
        i.amount,
        i.version,
        &i.extra,
    )
}

impl Approval {
    /// Signs `intent` for `chain_id` with `key`.
    pub fn sign(key: &SigningKey, chain_id: &str, intent: &Intent) -> Approval {
        let h = intent_hash(chain_id, intent);
        Approval {
            key: public_key_bytes(key),
            signature: sign_tagged(key, tags::ASSET_APPROVAL, &h),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Message)]
pub struct RegisterOp {
    #[prost(bytes = "vec", tag = "1")]
    pub class: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    pub asset: Vec<u8>,
    #[prost(bytes = "vec", tag = "3")]
    pub owner: Vec<u8>,
    #[prost(bytes = "vec", tag = "4")]
    pub metadata_hash: Vec<u8>,
    #[prost(message, repeated, tag = "5")]
    pub approvals: Vec<Approval>,
}

/// The owner offers the asset to ONE named buyer at a price (ADR-0036). Signed by the owner as
/// the sender of the transaction: the transaction signature IS the consent.
#[derive(Clone, PartialEq, Eq, Message)]
pub struct OfferOp {
    #[prost(bytes = "vec", tag = "1")]
    pub class: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    pub asset: Vec<u8>,
    #[prost(bytes = "vec", tag = "3")]
    pub buyer: Vec<u8>,
    #[prost(uint64, tag = "4")]
    pub price: u64,
    /// The asset version and metadata hash the owner saw and consented to.
    #[prost(uint64, tag = "5")]
    pub version: u64,
    #[prost(bytes = "vec", tag = "6")]
    pub metadata_hash: Vec<u8>,
}

/// The named buyer accepts: the price moves to the escrow and the veto window starts. The buyer
/// signs the TERMS it saw (price, version, metadata hash), so an owner who cancels and re-offers
/// on other terms cannot make this acceptance apply to them.
#[derive(Clone, PartialEq, Eq, Message)]
pub struct AcceptOp {
    #[prost(bytes = "vec", tag = "1")]
    pub class: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    pub asset: Vec<u8>,
    #[prost(uint64, tag = "3")]
    pub price: u64,
    #[prost(uint64, tag = "4")]
    pub version: u64,
    #[prost(bytes = "vec", tag = "5")]
    pub metadata_hash: Vec<u8>,
}

/// The owner withdraws an offer the buyer has not accepted yet.
#[derive(Clone, PartialEq, Eq, Message)]
pub struct CancelOp {
    #[prost(bytes = "vec", tag = "1")]
    pub class: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    pub asset: Vec<u8>,
}

/// Anyone completes an accepted trade once the veto window is over.
#[derive(Clone, PartialEq, Eq, Message)]
pub struct FinalizeOp {
    #[prost(bytes = "vec", tag = "1")]
    pub class: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    pub asset: Vec<u8>,
}

/// The registrar quorum vetoes a pending trade inside the window, with a PUBLIC reason.
#[derive(Clone, PartialEq, Eq, Message)]
pub struct VetoOp {
    #[prost(bytes = "vec", tag = "1")]
    pub class: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    pub asset: Vec<u8>,
    #[prost(string, tag = "3")]
    pub reason: String,
    #[prost(message, repeated, tag = "4")]
    pub approvals: Vec<Approval>,
}

#[derive(Clone, PartialEq, Eq, Message)]
pub struct LienOp {
    #[prost(bytes = "vec", tag = "1")]
    pub class: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    pub asset: Vec<u8>,
    #[prost(string, tag = "3")]
    pub kind: String,
    #[prost(message, repeated, tag = "4")]
    pub approvals: Vec<Approval>,
}

#[derive(Clone, PartialEq, Eq, Message)]
pub struct ReleaseOp {
    #[prost(bytes = "vec", tag = "1")]
    pub class: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    pub asset: Vec<u8>,
    #[prost(uint64, tag = "3")]
    pub lien_id: u64,
    #[prost(message, repeated, tag = "4")]
    pub approvals: Vec<Approval>,
}

#[derive(Clone, PartialEq, Eq, Message)]
pub struct RegistrarsOp {
    #[prost(bytes = "vec", tag = "1")]
    pub class: Vec<u8>,
    #[prost(bytes = "vec", repeated, tag = "2")]
    pub registrars: Vec<Vec<u8>>,
    #[prost(uint32, tag = "3")]
    pub threshold: u32,
    #[prost(message, repeated, tag = "4")]
    pub approvals: Vec<Approval>,
}

/// A transaction payload for the assets module (op byte + message).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    Register(RegisterOp),
    AddLien(LienOp),
    ReleaseLien(ReleaseOp),
    UpdateRegistrars(RegistrarsOp),
    Offer(OfferOp),
    Accept(AcceptOp),
    Cancel(CancelOp),
    Finalize(FinalizeOp),
    Veto(VetoOp),
}

impl Op {
    pub fn encode(&self) -> Vec<u8> {
        let (tag, body) = match self {
            Op::Register(m) => (1u8, m.encode_to_vec()),
            Op::AddLien(m) => (4, m.encode_to_vec()),
            Op::ReleaseLien(m) => (5, m.encode_to_vec()),
            Op::UpdateRegistrars(m) => (6, m.encode_to_vec()),
            Op::Offer(m) => (7, m.encode_to_vec()),
            Op::Accept(m) => (8, m.encode_to_vec()),
            Op::Cancel(m) => (9, m.encode_to_vec()),
            Op::Finalize(m) => (10, m.encode_to_vec()),
            Op::Veto(m) => (11, m.encode_to_vec()),
        };
        let mut out = vec![tag];
        out.extend_from_slice(&body);
        out
    }

    /// Strict decoding: unknown op or non-canonical bytes are refused.
    pub fn decode(bytes: &[u8]) -> Option<Op> {
        let (tag, body) = bytes.split_first()?;
        let op = match tag {
            1 => Op::Register(RegisterOp::decode(body).ok()?),
            4 => Op::AddLien(LienOp::decode(body).ok()?),
            5 => Op::ReleaseLien(ReleaseOp::decode(body).ok()?),
            6 => Op::UpdateRegistrars(RegistrarsOp::decode(body).ok()?),
            7 => Op::Offer(OfferOp::decode(body).ok()?),
            8 => Op::Accept(AcceptOp::decode(body).ok()?),
            9 => Op::Cancel(CancelOp::decode(body).ok()?),
            10 => Op::Finalize(FinalizeOp::decode(body).ok()?),
            11 => Op::Veto(VetoOp::decode(body).ok()?),
            _ => return None,
        };
        (op.encode() == bytes).then_some(op)
    }
}
