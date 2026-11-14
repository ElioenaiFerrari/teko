//! Protocol messages (prost-derived, hand-written: no `protoc` needed).
//!
//! SECURITY: never add fields carrying a PIN, seed, private key or voter secret.
//! `tests::no_secret_field_names` enforces this structurally.

use alloc::string::String;
use alloc::vec::Vec;

use crate::error::ProtocolError;
use crate::frame::Frame;

// ───────────────────────── enums ─────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, prost::Enumeration)]
#[repr(i32)]
pub enum DeviceState {
    Virgin = 0,
    FirmwareInstalled = 1,
    Initialized = 2,
    CredentialsIssued = 3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, prost::Enumeration)]
#[repr(i32)]
pub enum ButtonKind {
    Unspecified = 0,
    PinEntry = 1,
    FirmwareInstall = 2,
    ConfirmCivicId = 3,
    PairwiseId = 4,
    SignDocument = 5,
    ProveAttribute = 6,
    ExportCommitment = 7,
    SignBallot = 8,
    Wipe = 9,
    IssueIdentity = 10,
    LoadCredential = 11,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, prost::Enumeration)]
#[repr(i32)]
pub enum ErrorCode {
    Unspecified = 0,
    MalformedFrame = 1,
    UnknownMessage = 2,
    UnsupportedVersion = 3,
    InvalidState = 4,
    Busy = 5,
    PinLocked = 6,
    DeviceWiped = 7,
    UserRejected = 8,
    Timeout = 9,
    BadSignature = 10,
    InsufficientSignatures = 11,
    Downgrade = 12,
    HashMismatch = 13,
    NotImplemented = 14,
    InvalidArgument = 15,
    Internal = 16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, prost::Enumeration)]
#[repr(i32)]
pub enum PredicateKind {
    Unspecified = 0,
    /// Proves `age >= arg` via issuer-derived claims (`age_over_N`).
    AgeAtLeast = 1,
    /// Proves the attribute `name` exists without revealing its value.
    Has = 2,
    /// Reveals the value of attribute `name` (explicit on-device screen).
    Reveal = 3,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct Predicate {
    #[prost(enumeration = "PredicateKind", tag = "1")]
    pub kind: i32,
    #[prost(uint32, tag = "2")]
    pub arg: u32,
    #[prost(string, tag = "3")]
    pub name: String,
}

// ───────────────────────── requests ─────────────────────────

#[derive(Clone, PartialEq, prost::Message)]
pub struct Initialize {
    #[prost(uint32, tag = "1")]
    pub host_proto_max: u32,
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct GetFeatures {}
#[derive(Clone, PartialEq, prost::Message)]
pub struct AttestationChallenge {
    #[prost(bytes = "vec", tag = "1")]
    pub nonce: Vec<u8>,
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct FirmwareBegin {
    #[prost(bytes = "vec", tag = "1")]
    pub manifest: Vec<u8>,
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct FirmwareChunk {
    #[prost(uint64, tag = "1")]
    pub offset: u64,
    #[prost(bytes = "vec", tag = "2")]
    pub data: Vec<u8>,
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct FirmwareFinish {}
#[derive(Clone, PartialEq, prost::Message)]
pub struct SetupPin {}
#[derive(Clone, PartialEq, prost::Message)]
pub struct Unlock {}
#[derive(Clone, PartialEq, prost::Message)]
pub struct GetCivicId {}
#[derive(Clone, PartialEq, prost::Message)]
pub struct GetPairwiseId {
    #[prost(string, tag = "1")]
    pub service_id: String,
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct SignDocument {
    #[prost(bytes = "vec", tag = "1")]
    pub hash: Vec<u8>,
    /// Human-readable summary supplied by the HOST (untrusted); shown as such.
    #[prost(string, tag = "2")]
    pub summary: String,
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct ProveAttribute {
    #[prost(message, optional, tag = "1")]
    pub predicate: Option<Predicate>,
    #[prost(bytes = "vec", tag = "2")]
    pub verifier_nonce: Vec<u8>,
    #[prost(string, tag = "3")]
    pub verifier_name: String,
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct GetVoterCommitment {}
#[derive(Clone, PartialEq, prost::Message)]
pub struct SignBallot {
    #[prost(string, tag = "1")]
    pub election_id: String,
    #[prost(bytes = "vec", tag = "2")]
    pub ballot_hash: Vec<u8>,
    #[prost(string, tag = "3")]
    pub summary: String,
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct Wipe {}
#[derive(Clone, PartialEq, prost::Message)]
pub struct GetIdentityRequest {
    #[prost(bytes = "vec", tag = "1")]
    pub authority_nonce: Vec<u8>,
    #[prost(string, tag = "2")]
    pub authority_name: String,
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct LoadCredential {
    #[prost(bytes = "vec", tag = "1")]
    pub credential: Vec<u8>,
}
/// Reserved: the v1 emulator does not require it (confirmation is physical).
#[derive(Clone, PartialEq, prost::Message)]
pub struct ButtonAck {}
#[derive(Clone, PartialEq, prost::Message)]
pub struct Cancel {}
#[derive(Clone, PartialEq, prost::Message)]
pub struct Ping {}

// ───────────────────────── responses ─────────────────────────

#[derive(Clone, PartialEq, prost::Message)]
pub struct Features {
    #[prost(uint32, tag = "1")]
    pub proto_ver: u32,
    #[prost(uint32, tag = "2")]
    pub fw_version: u32,
    #[prost(enumeration = "DeviceState", tag = "3")]
    pub state: i32,
    #[prost(bool, tag = "4")]
    pub locked: bool,
    #[prost(uint32, tag = "5")]
    pub retries_remaining: u32,
    #[prost(string, repeated, tag = "6")]
    pub capabilities: Vec<String>,
    #[prost(string, tag = "7")]
    pub label: String,
    #[prost(bool, tag = "8")]
    pub has_credential: bool,
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct AttestationResponse {
    #[prost(bytes = "vec", repeated, tag = "1")]
    pub chain: Vec<Vec<u8>>,
    #[prost(bytes = "vec", tag = "2")]
    pub signature: Vec<u8>,
    #[prost(uint32, tag = "3")]
    pub fw_version: u32,
    #[prost(enumeration = "DeviceState", tag = "4")]
    pub state: i32,
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct FirmwareStatus {
    #[prost(uint64, tag = "1")]
    pub received: u64,
    #[prost(uint32, tag = "2")]
    pub installed_version: u32,
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct ButtonRequest {
    #[prost(enumeration = "ButtonKind", tag = "1")]
    pub kind: i32,
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct PinSet {}
#[derive(Clone, PartialEq, prost::Message)]
pub struct Unlocked {}
#[derive(Clone, PartialEq, prost::Message)]
pub struct PinWrong {
    #[prost(uint32, tag = "1")]
    pub remaining: u32,
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct CivicId {
    #[prost(string, tag = "1")]
    pub id: String,
    #[prost(bytes = "vec", tag = "2")]
    pub pk_identity: Vec<u8>,
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct PairwiseId {
    #[prost(string, tag = "1")]
    pub id: String,
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct DocumentSignature {
    #[prost(bytes = "vec", tag = "1")]
    pub signature: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    pub pk_identity: Vec<u8>,
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct AttributeProof {
    #[prost(bytes = "vec", tag = "1")]
    pub presentation: Vec<u8>,
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct VoterCommitment {
    #[prost(bytes = "vec", tag = "1")]
    pub commitment: Vec<u8>,
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct BallotSignature {
    #[prost(bytes = "vec", tag = "1")]
    pub signature: Vec<u8>,
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct WipeDone {}
#[derive(Clone, PartialEq, prost::Message)]
pub struct IdentityRequest {
    #[prost(bytes = "vec", tag = "1")]
    pub pk_identity: Vec<u8>,
    /// Signature by the identity key over (nonce, pk): proof of possession.
    #[prost(bytes = "vec", tag = "2")]
    pub pop_signature: Vec<u8>,
    #[prost(bytes = "vec", repeated, tag = "3")]
    pub attestation_chain: Vec<Vec<u8>>,
    /// Batch-key signature over (nonce, pk): proves a genuine device.
    #[prost(bytes = "vec", tag = "4")]
    pub attestation_signature: Vec<u8>,
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct CredentialLoaded {}
#[derive(Clone, PartialEq, prost::Message)]
pub struct Success {}
#[derive(Clone, PartialEq, prost::Message)]
pub struct Failure {
    #[prost(enumeration = "ErrorCode", tag = "1")]
    pub code: i32,
    #[prost(uint64, tag = "2")]
    pub retry_after_ms: u64,
    #[prost(uint32, tag = "3")]
    pub remaining: u32,
    #[prost(string, tag = "4")]
    pub message: String,
}

impl Failure {
    pub fn new(code: ErrorCode, message: &str) -> Self {
        Self {
            code: code as i32,
            retry_after_ms: 0,
            remaining: 0,
            message: String::from(message),
        }
    }
}

// ───────────────────────── enums of messages ─────────────────────────

macro_rules! define_messages {
    ($(#[$m:meta])* $name:ident { $($variant:ident($ty:ident) = $id:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Debug, PartialEq)]
        pub enum $name { $($variant($ty)),+ }

        impl $name {
            pub fn msg_type(&self) -> u16 {
                match self { $(Self::$variant(_) => $id),+ }
            }

            pub fn encode_payload(&self) -> Vec<u8> {
                match self { $(Self::$variant(m) => prost::Message::encode_to_vec(m)),+ }
            }

            pub fn decode(msg_type: u16, payload: &[u8]) -> Result<Self, ProtocolError> {
                match msg_type {
                    $($id => <$ty as prost::Message>::decode(payload)
                        .map(Self::$variant)
                        .map_err(|_| ProtocolError::MalformedPayload),)+
                    other => Err(ProtocolError::UnknownMessage(other)),
                }
            }
        }
    };
}

define_messages! {
    /// Host → device.
    Request {
        Initialize(Initialize) = 0x0001,
        GetFeatures(GetFeatures) = 0x0002,
        AttestationChallenge(AttestationChallenge) = 0x0003,
        SetupPin(SetupPin) = 0x0004,
        Unlock(Unlock) = 0x0005,
        GetCivicId(GetCivicId) = 0x0006,
        GetPairwiseId(GetPairwiseId) = 0x0007,
        SignDocument(SignDocument) = 0x0008,
        ProveAttribute(ProveAttribute) = 0x0009,
        GetVoterCommitment(GetVoterCommitment) = 0x000A,
        SignBallot(SignBallot) = 0x000B,
        Wipe(Wipe) = 0x000C,
        GetIdentityRequest(GetIdentityRequest) = 0x000D,
        LoadCredential(LoadCredential) = 0x000E,
        ButtonAck(ButtonAck) = 0x000F,
        Cancel(Cancel) = 0x0010,
        Ping(Ping) = 0x0011,
        FirmwareBegin(FirmwareBegin) = 0x0012,
        FirmwareChunk(FirmwareChunk) = 0x0013,
        FirmwareFinish(FirmwareFinish) = 0x0014,
    }
}

define_messages! {
    /// Device → host.
    Response {
        Features(Features) = 0x0101,
        AttestationResponse(AttestationResponse) = 0x0102,
        FirmwareStatus(FirmwareStatus) = 0x0103,
        ButtonRequest(ButtonRequest) = 0x0104,
        PinSet(PinSet) = 0x0105,
        Unlocked(Unlocked) = 0x0106,
        PinWrong(PinWrong) = 0x0107,
        CivicId(CivicId) = 0x0108,
        PairwiseId(PairwiseId) = 0x0109,
        DocumentSignature(DocumentSignature) = 0x010A,
        AttributeProof(AttributeProof) = 0x010B,
        VoterCommitment(VoterCommitment) = 0x010C,
        BallotSignature(BallotSignature) = 0x010D,
        WipeDone(WipeDone) = 0x010E,
        IdentityRequest(IdentityRequest) = 0x010F,
        CredentialLoaded(CredentialLoaded) = 0x0110,
        Success(Success) = 0x0111,
        Failure(Failure) = 0x0112,
    }
}

impl Request {
    pub fn to_frame(&self, request_id: u32) -> Frame {
        Frame::new(
            Frame::request_flags(),
            self.msg_type(),
            request_id,
            self.encode_payload(),
        )
    }

    pub fn from_frame(frame: &Frame) -> Result<Self, ProtocolError> {
        if frame.is_response() {
            return Err(ProtocolError::UnknownMessage(frame.msg_type));
        }
        Self::decode(frame.msg_type, &frame.payload)
    }
}

impl Response {
    pub fn to_frame(&self, request_id: u32) -> Frame {
        let is_err = matches!(self, Response::Failure(_));
        Frame::new(
            Frame::response_flags(is_err),
            self.msg_type(),
            request_id,
            self.encode_payload(),
        )
    }

    pub fn from_frame(frame: &Frame) -> Result<Self, ProtocolError> {
        if !frame.is_response() {
            return Err(ProtocolError::UnknownMessage(frame.msg_type));
        }
        Self::decode(frame.msg_type, &frame.payload)
    }

    pub fn failure(code: ErrorCode, message: &str) -> Self {
        Response::Failure(Failure::new(code, message))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn firmware_chunk_type_matches_frame_constant() {
        let m = Request::FirmwareChunk(FirmwareChunk {
            offset: 0,
            data: Vec::new(),
        });
        assert_eq!(m.msg_type(), crate::frame::FIRMWARE_CHUNK_TYPE);
    }

    /// Structural guard: no public field in this file may name a secret.
    #[test]
    fn no_secret_field_names() {
        let src = include_str!("messages.rs");
        let forbidden = [
            "pin", "private", "secret", "seed", "sk", "password", "mnemonic",
        ];
        for line in src.lines() {
            let t = line.trim_start();
            let Some(rest) = t.strip_prefix("pub ") else {
                continue;
            };
            let Some((name, _)) = rest.split_once(':') else {
                continue;
            };
            if name.contains(' ') || name.contains('(') {
                continue;
            }
            let lower = name.to_ascii_lowercase();
            for f in forbidden {
                let hit = lower == f
                    || lower.starts_with(&alloc::format!("{f}_"))
                    || lower.ends_with(&alloc::format!("_{f}"));
                assert!(!hit, "field `{name}` looks like a secret");
            }
        }
    }

    proptest! {
        #[test]
        fn request_roundtrip(nonce in proptest::collection::vec(any::<u8>(), 0..64), svc in ".{0,40}", rid in any::<u32>()) {
            for m in [
                Request::AttestationChallenge(AttestationChallenge { nonce: nonce.clone() }),
                Request::GetPairwiseId(GetPairwiseId { service_id: svc.clone() }),
            ] {
                let f = m.to_frame(rid);
                let back = Request::from_frame(&Frame::decode(&f.encode()).unwrap()).unwrap();
                prop_assert_eq!(back, m);
            }
        }

        #[test]
        fn decode_arbitrary_payload_never_panics(t in any::<u16>(), p in proptest::collection::vec(any::<u8>(), 0..256)) {
            let _ = Request::decode(t, &p);
            let _ = Response::decode(t, &p);
        }
    }
}
