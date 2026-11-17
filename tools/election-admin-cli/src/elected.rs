//! Candidates from the chain, the signed results and the `eleito` credential (ADR-0040).
//!
//! * `build_election`: the sample election with the ACTIVE `candidato` credentials of the economy
//!   chain as candidates (each option binds its credential);
//! * `outcomes`: the results the vote chain's institutions signed on the board, grouped by outcome;
//! * `claim_elected`: carries every outcome signed by more than 2/3 of the institutions to the
//!   economy chain, which then issues the lifetime `eleito` credential;
//! * `start_runoff`: opens the second round of a contest whose outcome is a RUNOFF.

use std::path::Path;

use prost::Message;
use teko_assets::devnet::World;
use teko_attestation::SignerSignature;
use teko_bulletin::{BulletinBoard, EntryKind};
use teko_credentials::candidate::{CandidateEntry, Cargo, select_candidates};
use teko_credentials::{CANDIDATE_TYPE, ClaimElectedOp, Op};
use teko_election::{ElectionOutcome, OutcomeKind, SignedOutcome, count_outcome_signers};
use teko_node::load_network;

use crate::Admin;

/// What the institutions published about one contest, with every signature collected.
#[derive(Debug, Clone)]
pub struct CertifiedOutcome {
    pub outcome: ElectionOutcome,
    pub signatures: Vec<SignerSignature>,
    /// Distinct institutions that signed these exact bytes.
    pub signers: usize,
    /// How many institutions there are.
    pub institutions: usize,
}

impl CertifiedOutcome {
    /// More than 2/3 of the institutions signed (what the economy ledger requires).
    pub fn is_certified(&self) -> bool {
        self.signers * 3 > self.institutions * 2
    }
}

/// The institutions in force, read from the VERIFIED chain (ADR-0045): the genesis set until a
/// rotation, the newest approved set after it. A file board has no chain, so it falls back to the
/// genesis set of the network files.
fn institutions(dir: &Path, board: &crate::AnyBoard) -> Result<Vec<Vec<u8>>, String> {
    if let crate::AnyBoard::Net(nb) = board
        && let Some(chain) = nb.chain()
    {
        return Ok(chain.epoch_at(chain.height() + 1).set.clone());
    }
    let net = load_network(&dir.join("net")).map_err(|e| e.to_string())?;
    Ok(net.profile.validators)
}

/// The outcomes on the (verified) board, one per distinct outcome bytes, with their signatures.
pub fn outcomes(admin: &Admin) -> Result<Vec<CertifiedOutcome>, String> {
    let board = admin.read_board().map_err(|e| e.to_string())?;
    let inst = institutions(admin.dir(), &board)?;
    let mut groups: Vec<(ElectionOutcome, Vec<u8>, Vec<SignerSignature>)> = Vec::new();
    for i in 0..board.len() {
        let Some(e) = board.entry(i) else { continue };
        if e.kind != EntryKind::OUTCOME {
            continue;
        }
        let Ok(s) = SignedOutcome::decode(e.payload.as_slice()) else {
            continue;
        };
        let Ok(o) = ElectionOutcome::decode(s.outcome.as_slice()) else {
            continue;
        };
        let sig = SignerSignature {
            signer: s.signer,
            signature: s.signature,
        };
        match groups.iter_mut().find(|(_, b, _)| *b == s.outcome) {
            Some((_, _, sigs)) => sigs.push(sig),
            None => groups.push((o, s.outcome, vec![sig])),
        }
    }
    Ok(groups
        .into_iter()
        .map(|(outcome, _, signatures)| CertifiedOutcome {
            signers: count_outcome_signers(&inst, &outcome, &signatures),
            institutions: inst.len(),
            outcome,
            signatures,
        })
        .collect())
}

/// Redoes the verifiable draw of an outcome from the BOARD alone (ADR-0043): replays the verified
/// board, takes the shares of the draw that checked out against the key ceremony, recombines the
/// seed and decides the contest again; the result must be the very bytes the institutions signed.
/// Does not trust the outcome, the nodes or anyone's signature.
pub fn verify_draw(admin: &Admin, o: &ElectionOutcome) -> Result<String, String> {
    use teko_election::{contest_hash, decide_with, needs_draw};
    if !o.drawn {
        return Err("este resultado não foi sorteado".into());
    }
    let board = admin.read_board().map_err(|e| e.to_string())?;
    let mut t = admin.new_tallier();
    crate::feed_new(&mut t, &board);
    let m = t.manifest().cloned().ok_or("o quadro não tem manifesto")?;
    let contest = m
        .contest(&o.contest_id)
        .ok_or("pleito desconhecido")?
        .clone();
    let snap = t.snapshot();
    let ct = snap.contest(&o.contest_id).ok_or("pleito sem apuração")?;
    let need = needs_draw(&contest, ct).ok_or("a apuração verificada não tem empate a sortear")?;
    let seed = t.draw_seed(&o.contest_id).ok_or_else(|| {
        let (have, want) = t.draw_progress(&o.contest_id);
        format!("faltam partes do sorteio no quadro: {have} de {want}")
    })?;
    if seed.as_slice() != o.draw_seed.as_slice() {
        return Err("a semente do quadro NÃO é a do resultado".into());
    }
    let again = decide_with(
        &o.vote_chain_id,
        &m.election_id,
        &contest,
        &contest_hash(&contest),
        ct,
        Some(&seed),
    )
    .ok_or("a regra não decide este pleito")?;
    if again.encode_to_vec() != o.encode_to_vec() {
        return Err("refeito do quadro, o resultado é OUTRO".into());
    }
    Ok(format!(
        "sorteio refeito do quadro: empatados {:?}, ordem sorteada {:?}",
        need.tied, o.draw_tied
    ))
}

/// Sends the `ClaimElected` of every certified ELECTED outcome to the economy chain. Anyone could
/// do it: the ledger believes the institutions' signatures, not the sender. Returns what it did.
pub fn claim_elected(dir: &Path) -> Result<Vec<String>, String> {
    let admin = Admin::open(dir).map_err(|e| e.to_string())?;
    let mut world = teko_node::NodeWorld::open(&dir.join("net")).map_err(|e| e.to_string())?;
    let mut lines = Vec::new();
    for c in outcomes(&admin)? {
        let o = &c.outcome;
        if o.kind() != OutcomeKind::Elected {
            continue;
        }
        if !c.is_certified() {
            lines.push(format!(
                "{}: só {} de {} instituições assinaram (precisa de mais de 2/3)",
                o.contest_id, c.signers, c.institutions
            ));
            continue;
        }
        // any key sends it (no balance needed when the network charges no fee)
        let sender =
            teko_crypto::generate_signing_key(&mut rand::rand_core::UnwrapErr(rand::rngs::SysRng));
        let op = Op::ClaimElected(ClaimElectedOp {
            outcome: o.encode_to_vec(),
            signers: c.signatures.iter().map(|s| s.signer.clone()).collect(),
            signatures: c.signatures.iter().map(|s| s.signature.clone()).collect(),
        });
        let tx = teko_ledger::Tx::new_signed(
            &sender,
            world.chain_id(),
            0,
            teko_credentials::CREDENTIALS_MODULE,
            op.encode(),
        );
        match world.submit(tx) {
            Ok(()) => lines.push(format!("{}: eleito registrado na chain", o.contest_id)),
            Err(e) if e.contains("already claimed") => {
                lines.push(format!("{}: já estava registrado", o.contest_id));
            }
            Err(e) => lines.push(format!("{}: recusado ({e})", o.contest_id)),
        }
    }
    if lines.is_empty() {
        return Err("nenhum resultado de eleito no quadro (feche a votação e espere as instituições assinarem)".into());
    }
    Ok(lines)
}

/// The sample election with the candidates of the CHAIN: for each contest, the ACTIVE `candidato`
/// credentials of `election_id` for that office and jurisdiction whose validity outlasts the voting
/// (plus `margin_secs`). A contest with NO candidacy keeps the example list; a contest whose
/// candidacies break the rules (repeated number or holder, fewer than 2) is an error.
pub fn build_election(
    dir: &Path,
    election_id: &str,
    margin_secs: u64,
    now: u64,
) -> Result<Vec<String>, String> {
    let world = teko_node::NodeWorld::open(&dir.join("net")).map_err(|e| e.to_string())?;
    let ledger = world.ledger_ref();
    // with no empty blocks the chain's last block time can lag: use the clock if it is later (the
    // validators accept block times within a minute of theirs)
    let at = now.max(world.chain().last_timestamp());
    let entries: Vec<CandidateEntry> =
        teko_credentials::credentials_of_type(ledger, CANDIDATE_TYPE.as_bytes())
            .into_iter()
            .map(|(id, rec)| CandidateEntry {
                active: rec.status_at(at) == teko_credentials::Status::Active,
                id,
                subject: rec.subject.clone(),
                claims: rec.claims.clone(),
                valid_until: rec.valid_until,
            })
            .collect();
    let mut cands = issuer_cli::DevCandidates::new();
    let mut lines = Vec::new();
    let encrypted = dir.join("net").join("tally-key.pub").exists();
    for p in issuer_cli::dev_contest_plan(now) {
        if p.seats > 0 && !encrypted {
            continue; // the Senate exists only in an encrypted election (ADR-0044)
        }
        let cargo = Cargo::parse(p.cargo).ok_or("cargo")?;
        let any = entries.iter().any(|e| {
            teko_credentials::candidate::parse_candidate(&e.claims).is_some_and(|c| {
                c.eleicao == election_id && c.cargo == cargo && c.scope_code() == p.scope_code
            })
        });
        if !any && p.chamber > 0 {
            lines.push(format!(
                "{}: sem candidaturas na chain, fora da eleição",
                p.id
            ));
            continue;
        }
        if !any {
            lines.push(format!(
                "{}: sem candidaturas na chain, lista de exemplo",
                p.id
            ));
            continue;
        }
        let sel = select_candidates(
            &entries,
            election_id,
            cargo,
            p.scope_code,
            p.voting_closes,
            margin_secs,
        )
        .map_err(|e| format!("{}: {e}", p.id))?;
        if p.chamber > 0 {
            let mut parties: Vec<&str> = sel.iter().map(|s| s.partido.as_str()).collect();
            parties.sort_unstable();
            parties.dedup();
            if parties.len() < 2 {
                return Err(format!("{}: são precisos ao menos 2 partidos", p.id));
            }
            lines.push(format!(
                "{}: {} candidatos de {} partidos, {} cadeiras",
                p.id,
                sel.len(),
                parties.len(),
                p.chamber
            ));
        } else {
            lines.push(format!("{}: {} candidatos da chain", p.id, sel.len()));
        }
        cands.insert(
            p.id.to_string(),
            sel.into_iter()
                .map(|s| issuer_cli::DevCandidate {
                    credential_id: s.id,
                    numero: s.numero,
                    nome: s.nome,
                    partido: s.partido,
                    legenda: s.legenda,
                })
                .collect(),
        );
    }
    issuer_cli::Issuer::open(dir)
        .dev_election_with(now, Some(election_id), &cands)
        .map_err(|e| e.to_string())?;
    Ok(lines)
}

/// Opens the second round of `contest_id` from its certified RUNOFF outcome: the two finalists'
/// candidacies must still be known to the chain. The revision is appended to `election.bundle`;
/// publish it on the board afterwards. Returns the new contest id.
pub fn start_runoff(dir: &Path, contest_id: &str, now: u64) -> Result<String, String> {
    let admin = Admin::open(dir).map_err(|e| e.to_string())?;
    let found = outcomes(&admin)?
        .into_iter()
        .find(|c| c.outcome.contest_id == contest_id && c.outcome.kind() == OutcomeKind::Runoff)
        .ok_or("não há resultado de 2º turno para este pleito")?;
    if !found.is_certified() {
        return Err(format!(
            "só {} de {} instituições assinaram o resultado",
            found.signers, found.institutions
        ));
    }
    let world = teko_node::NodeWorld::open(&dir.join("net")).map_err(|e| e.to_string())?;
    let mut fin = Vec::new();
    for id in &found.outcome.finalist_credentials {
        let rec = teko_credentials::credential(world.ledger_ref(), CANDIDATE_TYPE.as_bytes(), id)
            .ok_or("uma candidatura finalista não está mais na chain")?;
        let c = teko_credentials::candidate::parse_candidate(&rec.claims)
            .ok_or("candidatura finalista inválida")?;
        fin.push(issuer_cli::DevCandidate {
            credential_id: id.clone(),
            numero: c.numero,
            nome: c.nome,
            ..Default::default()
        });
    }
    let [a, b]: [issuer_cli::DevCandidate; 2] = fin
        .try_into()
        .map_err(|_| "o 2º turno precisa de exatamente 2 finalistas")?;
    issuer_cli::Issuer::open(dir)
        .dev_add_runoff(now, contest_id, &[a, b])
        .map_err(|e| e.to_string())
}
