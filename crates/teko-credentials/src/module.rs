use prost::Message;
use teko_ledger::{Ctx, Ledger, Module, ModuleError};

use crate::types::{
    CredRec, Op, ProposeOp, STATE_APPROVED, STATE_PROPOSED, STATE_REVOKED, STATE_VETOED, Status,
    TypeRec, VoteOp,
};

/// Module id of the credentials module in `Tx::module`.
pub const CREDENTIALS_MODULE: u32 = 3;

const MAX_TYPE: usize = 32;
const MAX_MEMBERS: usize = 16;
const MAX_CLAIMS: usize = 2_048;
const MAX_REASON: usize = 200;
/// Default contest window of a type created without one (7 days).
pub const DEFAULT_CONTEST_SECS: u64 = 7 * 86_400;

pub(crate) fn k_type(ctype: &[u8]) -> Vec<u8> {
    [b"t:".as_slice(), ctype].concat()
}
pub(crate) fn k_cred(ctype: &[u8], id: &[u8]) -> Vec<u8> {
    let mut k = b"c:".to_vec();
    k.push(ctype.len() as u8);
    k.extend_from_slice(ctype);
    k.extend_from_slice(id);
    k
}
pub(crate) fn k_subject(subject: &[u8], ctype: &[u8], id: &[u8]) -> Vec<u8> {
    let mut k = b"s:".to_vec();
    k.push(subject.len() as u8);
    k.extend_from_slice(subject);
    k.push(ctype.len() as u8);
    k.extend_from_slice(ctype);
    k.extend_from_slice(id);
    k
}

fn err(m: &str) -> ModuleError {
    ModuleError::new(m)
}

/// The identifier of a credential: a hash of everything the organ proposed.
pub fn credential_id(
    ctype: &[u8],
    subject: &[u8],
    commitment: &[u8],
    claims: &[u8],
    valid_until: u64,
) -> [u8; 32] {
    teko_crypto::ledger::cred_id(ctype, subject, commitment, claims, valid_until)
}

/// The commitment of a PUBLIC credential: a hash of its claims in clear.
pub fn public_commitment(claims: &[u8]) -> [u8; 32] {
    teko_crypto::ledger::cred_public_commitment(claims)
}

/// Creates a credential type (an issuing organ) at genesis.
pub fn genesis_type(
    ledger: &mut Ledger,
    ctype: &[u8],
    members: &[Vec<u8>],
    threshold: u32,
    contest_secs: u64,
    validity_secs: u64,
    public: bool,
) -> Result<(), String> {
    check_members(ctype, members, threshold).map_err(|e| e.0)?;
    if contest_secs == 0 {
        return Err("the contest window must be positive".into());
    }
    let rec = TypeRec {
        members: members.to_vec(),
        threshold,
        contest_secs,
        validity_secs,
        public,
    };
    ledger
        .genesis_put(CREDENTIALS_MODULE, &k_type(ctype), rec.encode_to_vec())
        .map_err(|e| format!("{e:?}"))
}

fn check_members(ctype: &[u8], members: &[Vec<u8>], threshold: u32) -> Result<(), ModuleError> {
    if ctype.is_empty() || ctype.len() > MAX_TYPE {
        return Err(err("invalid credential type name"));
    }
    if members.is_empty() || members.len() > MAX_MEMBERS {
        return Err(err("invalid number of members"));
    }
    if threshold == 0 || threshold as usize > members.len() {
        return Err(err("invalid threshold"));
    }
    for (i, m) in members.iter().enumerate() {
        if m.len() != 32 {
            return Err(err("invalid member address"));
        }
        if members.iter().take(i).any(|o| o == m) {
            return Err(err("duplicate member"));
        }
    }
    Ok(())
}

pub fn module() -> Box<dyn Module> {
    Box::new(CredentialsModule)
}

pub struct CredentialsModule;

fn load_type(ctx: &Ctx<'_, '_>, ctype: &[u8]) -> Result<TypeRec, ModuleError> {
    let raw = ctx
        .get(&k_type(ctype))
        .ok_or_else(|| err("unknown credential type"))?;
    TypeRec::decode(raw.as_slice()).map_err(|_| err("corrupt credential type"))
}

fn load_cred(ctx: &Ctx<'_, '_>, ctype: &[u8], id: &[u8]) -> Result<CredRec, ModuleError> {
    let raw = ctx
        .get(&k_cred(ctype, id))
        .ok_or_else(|| err("unknown credential"))?;
    CredRec::decode(raw.as_slice()).map_err(|_| err("corrupt credential"))
}

fn save(ctx: &mut Ctx<'_, '_>, ctype: &[u8], id: &[u8], rec: &CredRec) {
    ctx.set(&k_cred(ctype, id), rec.encode_to_vec());
}

/// The signer must be a member of the organ (identified by the wallet ADDRESS, which the
/// ledger already authenticated from the transaction signature).
fn member<'a>(ctx: &'a Ctx<'_, '_>, t: &TypeRec) -> Result<&'a [u8], ModuleError> {
    if t.members.iter().any(|m| m.as_slice() == ctx.sender) {
        Ok(&ctx.sender)
    } else {
        Err(err("only a member of the issuing organ can do this"))
    }
}

fn add_vote(votes: &mut Vec<Vec<u8>>, key: &[u8]) -> Result<(), ModuleError> {
    if votes.iter().any(|v| v.as_slice() == key) {
        return Err(err("this member already voted"));
    }
    votes.push(key.to_vec());
    Ok(())
}

fn check_reason(reason: &str) -> Result<(), ModuleError> {
    if reason.is_empty() || reason.len() > MAX_REASON {
        return Err(err("a public reason is required (at most 200 bytes)"));
    }
    Ok(())
}

/// The credential was approved: it waits out the contest window from now on.
fn approve_now(ctx: &Ctx<'_, '_>, t: &TypeRec, rec: &mut CredRec) -> Result<(), ModuleError> {
    rec.approved_at = ctx.time;
    rec.active_at = ctx.time.saturating_add(t.contest_secs);
    if rec.valid_until != 0 && rec.valid_until <= rec.active_at {
        return Err(err("the credential would expire before it becomes active"));
    }
    rec.state = STATE_APPROVED;
    Ok(())
}

fn propose(ctx: &mut Ctx<'_, '_>, op: ProposeOp) -> Result<(), ModuleError> {
    let t = load_type(ctx, &op.ctype)?;
    let me = member(ctx, &t)?.to_vec();
    if op.subject.len() != 33 && op.subject.len() != 65 {
        return Err(err("invalid subject key"));
    }
    if op.claims.len() > MAX_CLAIMS {
        return Err(err("claims too long"));
    }
    if op.commitment.len() != 32 {
        return Err(err("the commitment must be 32 bytes"));
    }
    if t.public {
        if op.claims.is_empty() {
            return Err(err("a public credential carries its claims"));
        }
        if op.commitment != public_commitment(&op.claims) {
            return Err(err("the commitment does not match the public claims"));
        }
    } else if !op.claims.is_empty() {
        return Err(err("this type keeps its claims off the chain"));
    }
    if t.validity_secs == 0 {
        if op.valid_until != 0 {
            return Err(err("this type does not expire"));
        }
    } else if op.valid_until <= ctx.time
        || op.valid_until > ctx.time.saturating_add(t.validity_secs)
    {
        return Err(err("valid_until is outside the validity of this type"));
    }
    if op.valid_until != 0 && op.valid_until <= ctx.time.saturating_add(t.contest_secs) {
        return Err(err("the credential would expire before it becomes active"));
    }
    let id = credential_id(
        &op.ctype,
        &op.subject,
        &op.commitment,
        &op.claims,
        op.valid_until,
    );
    if ctx.get(&k_cred(&op.ctype, &id)).is_some() {
        return Err(err("this credential was already proposed"));
    }
    let mut rec = CredRec {
        subject: op.subject.clone(),
        commitment: op.commitment,
        claims: op.claims,
        valid_until: op.valid_until,
        proposed_at: ctx.time,
        approvals: vec![me],
        state: STATE_PROPOSED,
        ..Default::default()
    };
    if rec.approvals.len() >= t.threshold as usize {
        approve_now(ctx, &t, &mut rec)?;
    }
    save(ctx, &op.ctype, &id, &rec);
    ctx.set(&k_subject(&op.subject, &op.ctype, &id), vec![1]);
    ctx.emit("proposed", id.to_vec());
    Ok(())
}

fn approve(ctx: &mut Ctx<'_, '_>, op: VoteOp) -> Result<(), ModuleError> {
    let t = load_type(ctx, &op.ctype)?;
    let me = member(ctx, &t)?.to_vec();
    let mut rec = load_cred(ctx, &op.ctype, &op.id)?;
    if rec.state != STATE_PROPOSED {
        return Err(err("the credential is not waiting for approvals"));
    }
    add_vote(&mut rec.approvals, &me)?;
    if rec.approvals.len() >= t.threshold as usize {
        approve_now(ctx, &t, &mut rec)?;
        ctx.emit("approved", op.id.clone());
    }
    save(ctx, &op.ctype, &op.id, &rec);
    Ok(())
}

fn veto(ctx: &mut Ctx<'_, '_>, op: VoteOp) -> Result<(), ModuleError> {
    let t = load_type(ctx, &op.ctype)?;
    let me = member(ctx, &t)?.to_vec();
    check_reason(&op.reason)?;
    let mut rec = load_cred(ctx, &op.ctype, &op.id)?;
    if rec.status_at(ctx.time) != Status::Pending {
        return Err(err("a veto is only possible inside the contest window"));
    }
    add_vote(&mut rec.veto_votes, &me)?;
    if rec.veto_votes.len() >= t.threshold as usize {
        rec.state = STATE_VETOED;
        rec.reason = op.reason;
        ctx.emit("vetoed", op.id.clone());
    }
    save(ctx, &op.ctype, &op.id, &rec);
    Ok(())
}

fn revoke(ctx: &mut Ctx<'_, '_>, op: VoteOp) -> Result<(), ModuleError> {
    let t = load_type(ctx, &op.ctype)?;
    let me = member(ctx, &t)?.to_vec();
    check_reason(&op.reason)?;
    let mut rec = load_cred(ctx, &op.ctype, &op.id)?;
    if rec.status_at(ctx.time) != Status::Active {
        return Err(err(
            "only an active credential can be revoked (use a veto while pending)",
        ));
    }
    add_vote(&mut rec.revoke_votes, &me)?;
    if rec.revoke_votes.len() >= t.threshold as usize {
        rec.state = STATE_REVOKED;
        rec.reason = op.reason;
        ctx.emit("revoked", op.id.clone());
    }
    save(ctx, &op.ctype, &op.id, &rec);
    Ok(())
}

impl Module for CredentialsModule {
    fn id(&self) -> u32 {
        CREDENTIALS_MODULE
    }

    fn execute(&self, ctx: &mut Ctx<'_, '_>, payload: &[u8]) -> Result<(), ModuleError> {
        match Op::decode(payload).ok_or_else(|| err("malformed credentials payload"))? {
            Op::Propose(op) => propose(ctx, op),
            Op::Approve(op) => approve(ctx, op),
            Op::Veto(op) => veto(ctx, op),
            Op::Revoke(op) => revoke(ctx, op),
        }
    }
}
