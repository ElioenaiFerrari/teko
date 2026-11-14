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
