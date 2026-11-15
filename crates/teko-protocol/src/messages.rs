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
    LoadElection = 12,
    VoterKey = 13,
    Register = 14,
    /// "Create a wallet?" prompt.
    WalletCreate = 15,
    /// A page of the recovery phrase is on the DEVICE screen (never sent to the host).
    SeedWords = 16,
    /// The device asks the user to type words at given positions.
    WalletConfirm = 17,
    WalletRestore = 18,
    WalletCheck = 19,
    /// "Use a passphrase?" prompt.
    WalletPassphrase = 20,
    /// "Send N coins to ADDRESS?" prompt (everything shown comes from the device).
    SignTransfer = 21,
    /// "Consent to sell/buy asset X?" prompt.
    SignAssetConsent = 22,
    /// "Bond / unbond / claim stake?" prompt.
    SignStake = 23,
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
    /// The voter's domicile does not cover this contest.
    NotEligible = 17,
    /// The contest is not in the right phase (e.g. voting not open).
    WrongPhase = 18,
    /// This device already signed a ballot for this contest.
    AlreadyVoted = 19,
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
/// Starts wallet creation ON THE DEVICE: it generates the phrase, shows it on its own
/// screen and asks for word positions. The host only waits; no words ever reach it.
#[derive(Clone, PartialEq, prost::Message)]
pub struct SetupWallet {}
/// Restores a wallet from a phrase typed ON THE DEVICE.
#[derive(Clone, PartialEq, prost::Message)]
pub struct RestoreWallet {}
/// Re-checks a written backup: the user retypes the phrase on the device.
#[derive(Clone, PartialEq, prost::Message)]
pub struct CheckWalletBackup {}
#[derive(Clone, PartialEq, prost::Message)]
pub struct GetAddress {}
/// Asks the device to sign a staking action of this wallet (become/stop being a
/// validator). The device builds the transaction, shows it and signs after the button.
#[derive(Clone, PartialEq, prost::Message)]
pub struct SignStake {
    #[prost(string, tag = "1")]
    pub chain_id: String,
    /// 1 = bond (lock coins), 2 = unbond (start the waiting period), 3 = claim matured coins.
    #[prost(uint32, tag = "2")]
    pub action: u32,
    #[prost(uint64, tag = "3")]
    pub amount: u64,
    #[prost(uint64, tag = "4")]
    pub fee: u64,
    #[prost(uint64, tag = "5")]
    pub nonce: u64,
}
/// Asks the device to sign the CONSENT of one party (seller or buyer) to move an asset.
/// The device shows the asset, parties, price and version and signs only after the
/// physical confirmation; it refuses if the wallet is neither the seller nor the buyer.
#[derive(Clone, PartialEq, prost::Message)]
pub struct SignAssetConsent {
    #[prost(string, tag = "1")]
    pub chain_id: String,
    /// 2 = transfer, 3 = swap (delivery versus payment).
    #[prost(uint32, tag = "2")]
    pub kind: u32,
    #[prost(string, tag = "3")]
    pub class: String,
    #[prost(string, tag = "4")]
    pub asset: String,
    /// `tekow1…` of the current owner (seller).
    #[prost(string, tag = "5")]
    pub from_address: String,
    /// `tekow1…` of the buyer.
    #[prost(string, tag = "6")]
    pub to_address: String,
    /// Price in coin units (swap only; 0 for a plain transfer).
    #[prost(uint64, tag = "7")]
    pub price: u64,
    /// Asset version the consent is bound to.
    #[prost(uint64, tag = "8")]
    pub version: u64,
}
/// Asks the device to sign a coin transfer. The device BUILDS the transaction itself
/// from these fields, shows it on its screen (destination, amount, fee, network) and
/// signs only after the physical confirmation.
#[derive(Clone, PartialEq, prost::Message)]
pub struct SignTransfer {
    #[prost(string, tag = "1")]
    pub chain_id: String,
    /// bech32m `tekow1…` destination.
    #[prost(string, tag = "2")]
    pub to_address: String,
    #[prost(uint64, tag = "3")]
    pub amount: u64,
    #[prost(uint64, tag = "4")]
    pub fee: u64,
    /// Number of transactions the account already executed.
    #[prost(uint64, tag = "5")]
    pub nonce: u64,
}
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
/// Gives the device an election: manifest (signed M-of-N) plus the governance
/// transitions so far. The device verifies everything itself.
#[derive(Clone, PartialEq, prost::Message)]
pub struct LoadElection {
    #[prost(bytes = "vec", tag = "1")]
    pub manifest: Vec<u8>,
    #[prost(bytes = "vec", repeated, tag = "2")]
    pub transitions: Vec<Vec<u8>>,
}
/// Reads the election already stored on the device (re-verified); no confirmation.
#[derive(Clone, PartialEq, prost::Message)]
pub struct GetElection {}
/// The PUBLIC voter key for one contest (e.g. to register it).
#[derive(Clone, PartialEq, prost::Message)]
pub struct GetVoterKey {
    #[prost(string, tag = "1")]
    pub election_id: String,
    #[prost(string, tag = "2")]
    pub contest_id: String,
}
/// Start an ANONYMOUS registration for one contest (ADR-0008). The device builds
/// the blinded message itself, so the host cannot register a key of its choosing.
#[derive(Clone, PartialEq, prost::Message)]
pub struct BeginRegistration {
    #[prost(string, tag = "1")]
    pub election_id: String,
    #[prost(string, tag = "2")]
    pub contest_id: String,
}
/// Finish it: the authority's blind signature in, the unlinkable token out.
#[derive(Clone, PartialEq, prost::Message)]
pub struct FinishRegistration {
    #[prost(string, tag = "1")]
    pub election_id: String,
    #[prost(string, tag = "2")]
    pub contest_id: String,
    #[prost(bytes = "vec", tag = "3")]
    pub blind_signature: Vec<u8>,
}
/// Sign a ballot. The host names the option by INDEX only: the text shown on the
/// device (candidate name, office, jurisdiction) comes from the verified manifest.
#[derive(Clone, PartialEq, prost::Message)]
pub struct SignBallot {
    #[prost(string, tag = "1")]
    pub election_id: String,
    #[prost(string, tag = "2")]
    pub contest_id: String,
    #[prost(uint32, tag = "3")]
    pub option_index: u32,
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

/// Wallet created or restored (public data only).
#[derive(Clone, PartialEq, prost::Message)]
pub struct WalletReady {
    /// bech32m `tekow1…` address.
    #[prost(string, tag = "1")]
    pub address: String,
    #[prost(bool, tag = "2")]
    pub has_passphrase: bool,
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct Address {
    #[prost(string, tag = "1")]
    pub address: String,
    /// SEC1 public key of the address (verifies signatures made by the wallet).
    #[prost(bytes = "vec", tag = "2")]
    pub public_key: Vec<u8>,
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct WalletBackupOk {}
/// Signature of the transfer built by the device (public data).
/// A signed consent (public data).
#[derive(Clone, PartialEq, prost::Message)]
pub struct AssetConsentSigned {
    #[prost(bytes = "vec", tag = "1")]
    pub signature: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    pub public_key: Vec<u8>,
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct TransferSigned {
    #[prost(bytes = "vec", tag = "1")]
    pub signature: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    pub public_key: Vec<u8>,
}

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
    /// A wallet exists on the device (created and backup confirmed, or restored).
    #[prost(bool, tag = "9")]
    pub wallet_ready: bool,
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
/// One race as the DEVICE understands it (after verifying the manifest).
#[derive(Clone, PartialEq, prost::Message)]
pub struct ContestInfo {
    #[prost(string, tag = "1")]
    pub contest_id: String,
    #[prost(string, tag = "2")]
    pub title: String,
    /// 0 Draft, 1 CandidatesPublished, 2 RegistrationOpen, 3 VotingOpen,
    /// 4 VotingClosed, 5 Tallied, 6 Audited, 7 Cancelled.
    #[prost(uint32, tag = "3")]
    pub phase: u32,
    /// 1 National, 2 State, 3 Municipality.
    #[prost(uint32, tag = "4")]
    pub scope_level: u32,
    #[prost(string, tag = "5")]
    pub scope_code: String,
    #[prost(string, repeated, tag = "6")]
    pub option_names: Vec<String>,
    /// Computed by the device from ITS credential's electoral domicile.
    #[prost(bool, tag = "7")]
    pub eligible: bool,
    /// This device already signed a ballot for this contest.
    #[prost(bool, tag = "8")]
    pub voted: bool,
    #[prost(uint64, tag = "9")]
    pub voting_opens: u64,
    #[prost(uint64, tag = "10")]
    pub voting_closes: u64,
    /// The contest requires an anonymous registration before voting (ADR-0008).
    #[prost(bool, tag = "11")]
    pub registration_required: bool,
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct ElectionLoaded {
    #[prost(string, tag = "1")]
    pub election_id: String,
    #[prost(string, tag = "2")]
    pub title: String,
    #[prost(uint32, tag = "3")]
    pub revision: u32,
    #[prost(uint64, tag = "4")]
    pub seq: u64,
    #[prost(message, repeated, tag = "5")]
    pub contests: Vec<ContestInfo>,
}
/// Sent to the registration authority. It identifies the CITIZEN (identity key,
/// credential, proof of possession) but carries only a BLINDED message: the
/// voter key is hidden from the authority. Nothing here is secret.
#[derive(Clone, PartialEq, prost::Message)]
pub struct RegistrationRequest {
    #[prost(bytes = "vec", tag = "1")]
    pub blind_message: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    pub pk_identity: Vec<u8>,
    /// Identity-key signature over (election, contest, blind_message). The ids are
    /// NOT repeated here: the authority verifies the signature against the ids it
    /// is given, so a host that swaps them just breaks the signature.
    #[prost(bytes = "vec", tag = "3")]
    pub pop_signature: Vec<u8>,
    /// Encoded `IssuedCredential` (carries the domicile claims).
    #[prost(bytes = "vec", tag = "4")]
    pub credential: Vec<u8>,
}
/// Public: publish `(voter_key, token)` on the registry/board, NEVER together
/// with anything from the issuance exchange.
#[derive(Clone, PartialEq, prost::Message)]
pub struct RegistrationToken {
    #[prost(bytes = "vec", tag = "1")]
    pub voter_key: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    pub token: Vec<u8>,
}
/// PUBLIC key only.
#[derive(Clone, PartialEq, prost::Message)]
pub struct VoterKey {
    #[prost(bytes = "vec", tag = "1")]
    pub voter_key: Vec<u8>,
}
#[derive(Clone, PartialEq, prost::Message)]
pub struct BallotSignature {
    #[prost(bytes = "vec", tag = "1")]
    pub signature: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    pub voter_key: Vec<u8>,
    /// Encrypted mode (election has a `tally_key`): the encrypted ballot the DEVICE made.
    /// The signature covers its hash. Empty in plaintext mode.
    #[prost(bytes = "vec", tag = "3")]
    pub encrypted: Vec<u8>,
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
        GetVoterKey(GetVoterKey) = 0x000A,
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
        LoadElection(LoadElection) = 0x0015,
        GetElection(GetElection) = 0x0016,
        BeginRegistration(BeginRegistration) = 0x0017,
        FinishRegistration(FinishRegistration) = 0x0018,
        SetupWallet(SetupWallet) = 0x0019,
        RestoreWallet(RestoreWallet) = 0x001A,
        CheckWalletBackup(CheckWalletBackup) = 0x001B,
        GetAddress(GetAddress) = 0x001C,
        SignTransfer(SignTransfer) = 0x001D,
        SignAssetConsent(SignAssetConsent) = 0x001E,
        SignStake(SignStake) = 0x001F,
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
        VoterKey(VoterKey) = 0x010C,
        BallotSignature(BallotSignature) = 0x010D,
        WipeDone(WipeDone) = 0x010E,
        IdentityRequest(IdentityRequest) = 0x010F,
        CredentialLoaded(CredentialLoaded) = 0x0110,
        Success(Success) = 0x0111,
        Failure(Failure) = 0x0112,
        ElectionLoaded(ElectionLoaded) = 0x0113,
        RegistrationRequest(RegistrationRequest) = 0x0114,
        RegistrationToken(RegistrationToken) = 0x0115,
        WalletReady(WalletReady) = 0x0116,
        Address(Address) = 0x0117,
        WalletBackupOk(WalletBackupOk) = 0x0118,
        TransferSigned(TransferSigned) = 0x0119,
        AssetConsentSigned(AssetConsentSigned) = 0x011A,
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
