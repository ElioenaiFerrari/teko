use prost::Message;

/// An issuing ORGAN: a credential type governed by an M-of-N set of member keys (ADR-0037).
#[derive(Clone, PartialEq, Eq, Message)]
pub struct TypeRec {
    /// Wallet ADDRESSES of the members (32 bytes each: the hash of their device wallet key).
    #[prost(bytes = "vec", repeated, tag = "1")]
    pub members: Vec<Vec<u8>>,
    #[prost(uint32, tag = "2")]
    pub threshold: u32,
    /// Seconds a credential stays PENDING after the quorum approved it; inside this window the
    /// quorum may veto (the contest window).
    #[prost(uint64, tag = "3")]
    pub contest_secs: u64,
    /// 0 = the credential never expires (`valid_until` must be 0); otherwise `valid_until` is
    /// required and at most this many seconds after the approval.
    #[prost(uint64, tag = "4")]
    pub validity_secs: u64,
    /// Public types (candidate, elected) keep their claims in clear; the others keep only a
    /// commitment, the claims stay with the holder.
    #[prost(bool, tag = "5")]
    pub public: bool,
    /// A SYSTEM type has no organ: its credentials are created by the module's own rules (the
    /// `eleito` credential, from an outcome certified by the vote chain's institutions, ADR-0040).
    #[prost(bool, tag = "6")]
    pub system: bool,
}

/// The institutions of the VOTE chain whose signed outcomes this ledger accepts (ADR-0040).
#[derive(Clone, PartialEq, Eq, Message)]
pub struct VoteSet {
    #[prost(string, tag = "1")]
    pub chain_id: String,
    /// SEC1 consensus keys, equal power: more than 2/3 must sign an outcome.
    #[prost(bytes = "vec", repeated, tag = "2")]
    pub validators: Vec<Vec<u8>>,
}

#[derive(Clone, PartialEq, Eq, Message)]
pub struct CredRec {
    /// The holder's CREDENTIAL key (derived per type on the device; unlinkable to identity).
    #[prost(bytes = "vec", tag = "1")]
    pub subject: Vec<u8>,
    /// Salted commitment to the claims (private types) or hash of the public claims.
    #[prost(bytes = "vec", tag = "2")]
    pub commitment: Vec<u8>,
    /// The claims in clear: only for public types.
    #[prost(bytes = "vec", tag = "3")]
    pub claims: Vec<u8>,
    /// Unix seconds; 0 = never expires.
    #[prost(uint64, tag = "4")]
    pub valid_until: u64,
    #[prost(uint64, tag = "5")]
    pub proposed_at: u64,
    #[prost(bytes = "vec", repeated, tag = "6")]
    pub approvals: Vec<Vec<u8>>,
    /// Block time at which the quorum approved; 0 while only proposed.
    #[prost(uint64, tag = "7")]
    pub approved_at: u64,
    /// From this block time on the credential is ACTIVE; 0 while only proposed.
    #[prost(uint64, tag = "8")]
    pub active_at: u64,
    #[prost(uint32, tag = "9")]
    pub state: u32,
    #[prost(bytes = "vec", repeated, tag = "10")]
    pub veto_votes: Vec<Vec<u8>>,
    #[prost(bytes = "vec", repeated, tag = "11")]
    pub revoke_votes: Vec<Vec<u8>>,
    /// PUBLIC reason of the veto or revocation.
    #[prost(string, tag = "12")]
    pub reason: String,
    /// The private attributes (`salt ‖ claims`) SEALED to the holder's credential key (ADR-0039):
    /// only the holder's device opens them, so a wallet restore recovers them from the chain.
    #[prost(bytes = "vec", tag = "13")]
    pub sealed: Vec<u8>,
}

pub(crate) const STATE_PROPOSED: u32 = 0;
pub(crate) const STATE_APPROVED: u32 = 1;
pub(crate) const STATE_VETOED: u32 = 2;
pub(crate) const STATE_REVOKED: u32 = 3;
pub(crate) const STATE_SUPERSEDED: u32 = 4;

/// What a credential is worth at a given block time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Proposed; the quorum has not approved yet.
    Proposed,
    /// Approved, inside the contest window: not valid yet, the quorum may still veto.
    Pending,
    Active,
    /// Past `valid_until`.
    Expired,
    Vetoed,
    Revoked,
    /// Replaced by a newer `eleito` of the same office and jurisdiction (the history stays).
    Superseded,
}

impl CredRec {
    pub fn status_at(&self, time: u64) -> Status {
        match self.state {
            STATE_VETOED => Status::Vetoed,
            STATE_REVOKED => Status::Revoked,
            STATE_SUPERSEDED => Status::Superseded,
            STATE_APPROVED => {
                if self.valid_until != 0 && time >= self.valid_until {
                    Status::Expired
                } else if time < self.active_at {
                    Status::Pending
                } else {
                    Status::Active
                }
            }
            _ => Status::Proposed,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Message)]
pub struct ProposeOp {
    #[prost(bytes = "vec", tag = "1")]
    pub ctype: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    pub subject: Vec<u8>,
    #[prost(bytes = "vec", tag = "3")]
    pub commitment: Vec<u8>,
    #[prost(bytes = "vec", tag = "4")]
    pub claims: Vec<u8>,
    #[prost(uint64, tag = "5")]
    pub valid_until: u64,
    /// Private types only: the attributes sealed to the holder (ADR-0039); may be empty.
    #[prost(bytes = "vec", tag = "6")]
    pub sealed: Vec<u8>,
}

/// A member approves, vetoes or revokes the credential `id` (one member per transaction).
#[derive(Clone, PartialEq, Eq, Message)]
pub struct VoteOp {
    #[prost(bytes = "vec", tag = "1")]
    pub ctype: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    pub id: Vec<u8>,
    /// PUBLIC reason (veto and revoke only).
    #[prost(string, tag = "3")]
    pub reason: String,
}

/// Anyone claims the credential of the contest's winner: the outcome (an encoded
/// `ElectionOutcome`) and the signatures of the vote chain's institutions.
#[derive(Clone, PartialEq, Eq, Message)]
pub struct ClaimElectedOp {
    #[prost(bytes = "vec", tag = "1")]
    pub outcome: Vec<u8>,
    #[prost(bytes = "vec", repeated, tag = "2")]
    pub signers: Vec<Vec<u8>>,
    #[prost(bytes = "vec", repeated, tag = "3")]
    pub signatures: Vec<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    Propose(ProposeOp),
    Approve(VoteOp),
    Veto(VoteOp),
    Revoke(VoteOp),
    ClaimElected(ClaimElectedOp),
}

impl Op {
    pub fn encode(&self) -> Vec<u8> {
        let (tag, body) = match self {
            Op::Propose(m) => (1u8, m.encode_to_vec()),
            Op::Approve(m) => (2, m.encode_to_vec()),
            Op::Veto(m) => (3, m.encode_to_vec()),
            Op::Revoke(m) => (4, m.encode_to_vec()),
            Op::ClaimElected(m) => (5, m.encode_to_vec()),
        };
        let mut out = vec![tag];
        out.extend_from_slice(&body);
        out
    }

    /// Strict decoding: unknown op or non-canonical bytes are refused.
    pub fn decode(bytes: &[u8]) -> Option<Op> {
        let (tag, body) = bytes.split_first()?;
        let op = match tag {
            1 => Op::Propose(ProposeOp::decode(body).ok()?),
            2 => Op::Approve(VoteOp::decode(body).ok()?),
            3 => Op::Veto(VoteOp::decode(body).ok()?),
            4 => Op::Revoke(VoteOp::decode(body).ok()?),
            5 => Op::ClaimElected(ClaimElectedOp::decode(body).ok()?),
            _ => return None,
        };
        (op.encode() == bytes).then_some(op)
    }
}
