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
    Transfer = 2,
    Swap = 3,
    AddLien = 4,
    ReleaseLien = 5,
    UpdateRegistrars = 6,
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

    /// `metadata_hash` is the asset's on-chain metadata hash: the consent covers WHICH
    /// asset it is (ADR-0032), not only its id.
    pub fn transfer(
        class: &[u8],
        asset: &[u8],
        from: Address,
        to: Address,
        version: u64,
        metadata_hash: &[u8],
    ) -> Intent {
        Intent {
            from,
            to,
            version,
            extra: metadata_hash.to_vec(),
            ..Self::base(IntentKind::Transfer, class, asset)
        }
    }

    pub fn swap(
        class: &[u8],
        asset: &[u8],
        from: Address,
        to: Address,
        price: u64,
        version: u64,
        metadata_hash: &[u8],
    ) -> Intent {
        Intent {
            from,
            to,
            amount: price,
            version,
            extra: metadata_hash.to_vec(),
            ..Self::base(IntentKind::Swap, class, asset)
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

#[derive(Clone, PartialEq, Eq, Message)]
pub struct TransferOp {
    #[prost(bytes = "vec", tag = "1")]
    pub class: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    pub asset: Vec<u8>,
    #[prost(bytes = "vec", tag = "3")]
    pub to: Vec<u8>,
    #[prost(message, repeated, tag = "4")]
    pub approvals: Vec<Approval>,
}

#[derive(Clone, PartialEq, Eq, Message)]
pub struct SwapOp {
    #[prost(bytes = "vec", tag = "1")]
    pub class: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    pub asset: Vec<u8>,
    #[prost(bytes = "vec", tag = "3")]
    pub buyer: Vec<u8>,
    #[prost(uint64, tag = "4")]
    pub price: u64,
    #[prost(message, repeated, tag = "5")]
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
    Transfer(TransferOp),
    Swap(SwapOp),
    AddLien(LienOp),
    ReleaseLien(ReleaseOp),
    UpdateRegistrars(RegistrarsOp),
}

impl Op {
    pub fn encode(&self) -> Vec<u8> {
        let (tag, body) = match self {
            Op::Register(m) => (1u8, m.encode_to_vec()),
            Op::Transfer(m) => (2, m.encode_to_vec()),
            Op::Swap(m) => (3, m.encode_to_vec()),
            Op::AddLien(m) => (4, m.encode_to_vec()),
            Op::ReleaseLien(m) => (5, m.encode_to_vec()),
            Op::UpdateRegistrars(m) => (6, m.encode_to_vec()),
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
            2 => Op::Transfer(TransferOp::decode(body).ok()?),
            3 => Op::Swap(SwapOp::decode(body).ok()?),
            4 => Op::AddLien(LienOp::decode(body).ok()?),
            5 => Op::ReleaseLien(ReleaseOp::decode(body).ok()?),
            6 => Op::UpdateRegistrars(RegistrarsOp::decode(body).ok()?),
            _ => return None,
        };
        (op.encode() == bytes).then_some(op)
    }
}
