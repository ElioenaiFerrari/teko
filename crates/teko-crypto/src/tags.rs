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
pub const DECRYPT_PROOF: &str = "teko/v2/decrypt-proof";
pub const DKG_POP: &str = "teko/v2/dkg-pop";
pub const BALLOT_CT: &str = "teko/v2/ballot-ciphertext";
/// Identifies a ballot payload in a pre-verification (it is never signed).
pub const BALLOT_PRECHECK: &str = "teko/v2/ballot-precheck";
pub const BFT_VOTE: &str = "teko/v2/bft-vote";
pub const BFT_PROPOSAL: &str = "teko/v2/bft-proposal";
pub const BFT_BLOCK: &str = "teko/v2/bft-block";
pub const LEDGER_TX: &str = "teko/v2/ledger-tx";
pub const LEDGER_TX_ID: &str = "teko/v2/ledger-tx-id";
pub const LEDGER_ADDRESS: &str = "teko/v2/ledger-address";
pub const LEDGER_STATE_LEAF: &str = "teko/v2/ledger-state-leaf";
pub const COIN_SLASH: &str = "teko/v2/coin-slash";
pub const ASSET_INTENT: &str = "teko/v2/asset-intent";
pub const ASSET_APPROVAL: &str = "teko/v2/asset-approval";
/// Hash of the off-chain metadata record of an asset (ADR-0032).
pub const ASSET_METADATA: &str = "teko/v2/asset-metadata";
