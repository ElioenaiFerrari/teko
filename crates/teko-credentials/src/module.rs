use prost::Message;
use teko_ledger::{Ctx, Ledger, Module, ModuleError};

use teko_attestation::SignerSignature;
use teko_crypto::{tagged_hash, tags};
use teko_election::{ElectionOutcome, Office, OutcomeKind, ScopeLevel, count_outcome_signers};

use crate::candidate::{Cargo, parse_candidate};
use crate::types::{
    ClaimElectedOp, CredRec, Op, ProposeOp, STATE_APPROVED, STATE_PROPOSED, STATE_REVOKED,
    STATE_SUPERSEDED, STATE_VETOED, Status, TypeRec, VoteOp, VoteSet,
};

/// Module id of the credentials module in `Tx::module`.
pub const CREDENTIALS_MODULE: u32 = 3;

const MAX_TYPE: usize = 32;
const MAX_MEMBERS: usize = 16;
const MAX_CLAIMS: usize = 2_048;
/// Ephemeral key + tag + salt + claims.
const MAX_SEALED: usize = 33 + 16 + 16 + MAX_CLAIMS;
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

const K_VOTE_SET: &[u8] = b"v:set";
/// The type of the lifetime credential of an elected person (ADR-0040).
pub const ELECTED_TYPE: &str = "eleito";
/// The type candidates hold.
pub const CANDIDATE_TYPE: &str = "candidato";

/// The institutions of the vote chain whose certified outcomes this ledger accepts (ADR-0040),
/// fixed at genesis (rotating the set is PENDING).
pub fn genesis_vote_set(
    ledger: &mut Ledger,
    chain_id: &str,
    validators: &[Vec<u8>],
) -> Result<(), String> {
    if chain_id.is_empty() || validators.is_empty() || validators.len() > 64 {
        return Err("invalid vote chain institutions".into());
    }
    for (i, v) in validators.iter().enumerate() {
        if v.len() != 33 && v.len() != 65 {
            return Err("invalid institution key".into());
        }
        if validators.iter().take(i).any(|o| o == v) {
            return Err("duplicate institution".into());
        }
    }
    let rec = VoteSet {
        chain_id: chain_id.into(),
        validators: validators.to_vec(),
    };
    ledger
        .genesis_put(CREDENTIALS_MODULE, K_VOTE_SET, rec.encode_to_vec())
        .map_err(|e| format!("{e:?}"))
}

/// Creates a SYSTEM credential type (no organ): its credentials come from the module's own rules.
pub fn genesis_system_type(ledger: &mut Ledger, ctype: &[u8], public: bool) -> Result<(), String> {
    if ctype.is_empty() || ctype.len() > MAX_TYPE {
        return Err("invalid credential type name".into());
    }
    let rec = TypeRec {
        members: Vec::new(),
        threshold: 0,
        contest_secs: 0,
        validity_secs: 0,
        public,
        system: true,
    };
    ledger
        .genesis_put(CREDENTIALS_MODULE, &k_type(ctype), rec.encode_to_vec())
        .map_err(|e| format!("{e:?}"))
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
        system: false,
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
    if t.public && !op.sealed.is_empty() {
        return Err(err("a public credential has nothing to seal"));
    }
    if op.sealed.len() > MAX_SEALED {
        return Err(err("sealed attributes too long"));
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
        sealed: op.sealed,
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

fn office_name(o: Office) -> Option<&'static str> {
    match o {
        Office::President => Some(Cargo::Presidente.name()),
        Office::Governor => Some(Cargo::Governador.name()),
        Office::Mayor => Some(Cargo::Prefeito.name()),
        _ => None,
    }
}

/// Anyone can carry the result of a contest here (ADR-0040). The ledger believes it because MORE
/// THAN 2/3 of the vote chain's institutions (keys fixed at genesis) signed the very same outcome:
/// the same trust as the vote chain's own consensus, with no change to its blocks. The winner gets
/// a lifetime `eleito` credential and the previous holder of the same office and jurisdiction is
/// SUPERSEDED (kept in the history).
fn claim_elected(ctx: &mut Ctx<'_, '_>, op: ClaimElectedOp) -> Result<(), ModuleError> {
    let set = ctx
        .get(K_VOTE_SET)
        .and_then(|raw| VoteSet::decode(raw.as_slice()).ok())
        .ok_or_else(|| err("this network has no vote chain institutions"))?;
    let t = load_type(ctx, ELECTED_TYPE.as_bytes())?;
    if !t.system {
        return Err(err("the elected type is not a system type"));
    }
    if op.outcome.len() > 4_096 || op.signers.len() != op.signatures.len() || op.signers.len() > 64
    {
        return Err(err("malformed claim"));
    }
    let outcome =
        ElectionOutcome::decode(op.outcome.as_slice()).map_err(|_| err("malformed outcome"))?;
    // canonical bytes: what the institutions signed is exactly what was decoded
    if outcome.encode_to_vec() != op.outcome {
        return Err(err("non-canonical outcome"));
    }
    if outcome.vote_chain_id != set.chain_id {
        return Err(err("the outcome is from another vote chain"));
    }
    if outcome.kind() != OutcomeKind::Elected {
        return Err(err("only a decided contest elects someone"));
    }
    let sigs: Vec<SignerSignature> = op
        .signers
        .iter()
        .zip(&op.signatures)
        .map(|(signer, signature)| SignerSignature {
            signer: signer.clone(),
            signature: signature.clone(),
        })
        .collect();
    let signed = count_outcome_signers(&set.validators, &outcome, &sigs);
    if signed * 3 <= set.validators.len() * 2 {
        return Err(err(
            "more than 2/3 of the institutions must sign the outcome",
        ));
    }
    // the office and the jurisdiction agree (the ledger re-checks what the manifest enforced)
    let office = outcome.office();
    let cargo =
        office_name(office).ok_or_else(|| err("this office is not elected automatically"))?;
    let scope = outcome
        .scope
        .clone()
        .ok_or_else(|| err("outcome without jurisdiction"))?;
    if office.required_level() != Some(scope.level()) {
        return Err(err("office and jurisdiction do not match"));
    }
    // one claim per contest
    let marker = {
        let mut k = b"o:".to_vec();
        k.extend_from_slice(&tagged_hash(
            tags::CRED_ID,
            &[
                outcome.election_id.as_bytes(),
                outcome.contest_id.as_bytes(),
                &outcome.contest_hash,
            ],
        ));
        k
    };
    if ctx.get(&marker).is_some() {
        return Err(err("this contest's result was already claimed"));
    }
    // the winner is a candidate the chain knows: its holder is who is elected
    let cand = load_cred(
        ctx,
        CANDIDATE_TYPE.as_bytes(),
        &outcome.winner_credential_id,
    )?;
    if matches!(cand.state, STATE_VETOED | STATE_REVOKED) {
        return Err(err("the winner's candidacy was vetoed or revoked"));
    }
    let c =
        parse_candidate(&cand.claims).ok_or_else(|| err("the winner's candidacy is not valid"))?;
    let scope_code = match scope.level() {
        ScopeLevel::National => "",
        _ => scope.code.as_str(),
    };
    if c.cargo.name() != cargo || c.scope_code() != scope_code || c.eleicao != outcome.election_id {
        return Err(err("the winner's candidacy is not for this contest"));
    }
    let votes = outcome
        .totals
        .get(outcome.winner_option as usize)
        .copied()
        .ok_or_else(|| err("winner outside the ballot"))?;
    let mut parts = vec![format!("cargo={cargo}")];
    if office == Office::Governor {
        parts.push(format!("uf={scope_code}"));
    }
    if office == Office::Mayor {
        parts.push(format!("municipio={scope_code}"));
    }
    parts.push(format!("numero={}", c.numero));
    parts.push(format!("nome={}", c.nome));
    parts.push(format!("eleicao={}", outcome.election_id));
    parts.push(format!("pleito={}", outcome.contest_id));
    parts.push(format!("votos={votes}"));
    parts.push(format!(
        "candidatura={}",
        teko_crypto::hex::encode(&outcome.winner_credential_id)
    ));
    let claims = parts.join(";").into_bytes();
    let commitment = public_commitment(&claims);
    let id = credential_id(
        ELECTED_TYPE.as_bytes(),
        &cand.subject,
        &commitment,
        &claims,
        0,
    );
    if ctx.get(&k_cred(ELECTED_TYPE.as_bytes(), &id)).is_some() {
        return Err(err("this credential already exists"));
    }
    // the previous holder of this office and jurisdiction is superseded
    let slot = {
        let mut k = b"e:".to_vec();
        k.push(office as u8);
        k.push(scope.level() as u8);
        k.extend_from_slice(scope_code.as_bytes());
        k
    };
    if let Some(old_id) = ctx.get(&slot)
        && let Some(raw) = ctx.get(&k_cred(ELECTED_TYPE.as_bytes(), &old_id))
        && let Ok(mut old) = CredRec::decode(raw.as_slice())
    {
        old.state = STATE_SUPERSEDED;
        old.reason = format!("sucedido por {}", teko_crypto::hex::encode(&id[..8]));
        save(ctx, ELECTED_TYPE.as_bytes(), &old_id, &old);
        ctx.emit("superseded", old_id);
    }
    let rec = CredRec {
        subject: cand.subject.clone(),
        commitment: commitment.to_vec(),
        claims,
        valid_until: 0,
        proposed_at: ctx.time,
        approved_at: ctx.time,
        active_at: ctx.time,
        state: STATE_APPROVED,
        ..Default::default()
    };
    save(ctx, ELECTED_TYPE.as_bytes(), &id, &rec);
    ctx.set(
        &k_subject(&cand.subject, ELECTED_TYPE.as_bytes(), &id),
        vec![1],
    );
    ctx.set(&slot, id.to_vec());
    ctx.set(&marker, vec![1]);
    ctx.emit("elected", id.to_vec());
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
            Op::ClaimElected(op) => claim_elected(ctx, op),
        }
    }
}
