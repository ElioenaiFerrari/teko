//! Domain-separation tags. Each use of a hash/KDF/signature has a unique,
//! versioned tag so outputs from one domain are never valid in another.

pub const IDENTITY: &str = "teko/v1/identity";
pub const CIVIC_ID: &str = "teko/v1/civic-id";
pub const PAIRWISE: &str = "teko/v1/pairwise";
pub const PAIRWISE_ID: &str = "teko/v1/pairwise-id";
pub const VOTER: &str = "teko/v1/voter";
pub const DOC_SIGN: &str = "teko/v1/doc-sign";
pub const ATTR_PROOF: &str = "teko/v1/attr-proof";
pub const ATTESTATION: &str = "teko/v1/attestation";
pub const ATTEST_CERT: &str = "teko/v1/attest-cert";
pub const FW_MANIFEST: &str = "teko/v1/fw-manifest";
pub const CREDENTIAL: &str = "teko/v1/credential";
pub const CLAIM: &str = "teko/v1/claim";
pub const ISSUE_REQUEST: &str = "teko/v1/issue-request";
pub const PIN: &str = "teko/v1/pin";
pub const ISSUE_POP: &str = "teko/v1/issue-pop";
pub const ISSUE_ATTEST: &str = "teko/v1/issue-attest";
pub const FW_BLOB: &str = "teko/v1/fw-blob";
pub const ELECTION_MANIFEST: &str = "teko/v2/election-manifest";
pub const ELECTION_TRANSITION: &str = "teko/v2/election-transition";
/// What the institutions of the vote chain sign about the result of one contest (ADR-0040).
pub const ELECTION_OUTCOME: &str = "teko/v2/election-outcome";
/// Hash-to-point input of the verifiable draw that breaks a tie (ADR-0043).
pub const DRAW_POINT: &str = "teko/v2/draw-point";
/// The seed of the verifiable draw, from the combined threshold-VRF point (ADR-0043).
pub const DRAW_SEED: &str = "teko/v2/draw-seed";
pub const BULLETIN_LEAF: &str = "teko/v2/bulletin-leaf";
pub const BULLETIN_NODE: &str = "teko/v2/bulletin-node";
pub const BULLETIN_HEAD: &str = "teko/v2/bulletin-head";
pub const CONTEST: &str = "teko/v2/contest";
pub const VOTER_KEY: &str = "teko/v2/voter-key";
pub const BALLOT: &str = "teko/v2/ballot";
pub const REGISTRATION: &str = "teko/v2/registration";
pub const REGISTRATION_REQUEST: &str = "teko/v2/registration-request";
pub const REGISTRATION_BLIND: &str = "teko/v2/registration-blind";
pub const BLOCK: &str = "teko/v2/block";
pub const BLOCK_ENTRIES: &str = "teko/v2/block-entries";
pub const CHAIN_GENESIS: &str = "teko/v2/chain-genesis";
pub const REVOCATION_LIST: &str = "teko/v2/revocation-list";
pub const BALLOT_BIT_PROOF: &str = "teko/v2/ballot-bit-proof";
pub const BALLOT_SUM_PROOF: &str = "teko/v2/ballot-sum-proof";
/// Proof that a multi-seat ballot's weighted total encrypts a value in `1..=seats` (ADR-0044).
pub const BALLOT_RANGE_PROOF: &str = "teko/v2/ballot-range-proof";
pub const DECRYPT_PROOF: &str = "teko/v2/decrypt-proof";
pub const DKG_POP: &str = "teko/v2/dkg-pop";
pub const BALLOT_CT: &str = "teko/v2/ballot-ciphertext";
/// Identifies a ballot payload in a pre-verification (it is never signed).
pub const BALLOT_PRECHECK: &str = "teko/v2/ballot-precheck";
pub const BFT_VOTE: &str = "teko/v2/bft-vote";
pub const BFT_PROPOSAL: &str = "teko/v2/bft-proposal";
pub const BFT_BLOCK: &str = "teko/v2/bft-block";
/// What the institutions sign to approve a new institution set (ADR-0045).
pub const VALIDATOR_SET_CHANGE: &str = "teko/v2/validator-set-change";
/// The identity of an ordered institution set (ADR-0045).
pub const VALIDATOR_SET: &str = "teko/v2/validator-set";
pub const LEDGER_TX: &str = "teko/v2/ledger-tx";
pub const LEDGER_TX_ID: &str = "teko/v2/ledger-tx-id";
pub const LEDGER_ADDRESS: &str = "teko/v2/ledger-address";
pub const LEDGER_STATE_LEAF: &str = "teko/v2/ledger-state-leaf";
pub const COIN_SLASH: &str = "teko/v2/coin-slash";
pub const ASSET_INTENT: &str = "teko/v2/asset-intent";
pub const ASSET_APPROVAL: &str = "teko/v2/asset-approval";
/// Address of the escrow that holds the price of a pending asset trade (ADR-0036); nobody holds its key.
pub const ASSET_ESCROW: &str = "teko/v2/asset-escrow";
/// Selects the wallet branch of a credential type (ADR-0039): `m/7240'/1'/H(type)'`.
pub const CRED_KEY: &str = "teko/v2/credential-key";
/// Commitment of a PRIVATE on-chain credential: a hash of its sorted per-claim digests (ADR-0038).
pub const CRED_CLAIMS_PRIVATE: &str = "teko/v2/credential-claims-private";
/// Per-claim salt derived from the credential salt and the claim name (ADR-0038).
pub const CRED_CLAIM_SALT: &str = "teko/v2/credential-claim-salt";
/// Key of the sealed attributes of a credential (ADR-0039).
pub const SEAL_KEY: &str = "teko/v2/seal-key";
/// Nonce of the sealed attributes of a credential (ADR-0039).
pub const SEAL_NONCE: &str = "teko/v2/seal-nonce";
/// Identifier of an on-chain credential (ADR-0037).
pub const CRED_ID: &str = "teko/v2/credential-id";
/// Commitment to the PUBLIC claims of an on-chain credential (ADR-0037).
pub const CRED_CLAIMS: &str = "teko/v2/credential-claims";
/// Hash of the off-chain metadata record of an asset (ADR-0032).
pub const ASSET_METADATA: &str = "teko/v2/asset-metadata";
/// Binds a node's Noise static key to its consensus key in the peer handshake (ADR-0048).
pub const NET_BIND: &str = "teko/v2/net-bind";
/// A peer's signed address record (ADR-0048).
pub const NET_RECORD: &str = "teko/v2/net-record";
