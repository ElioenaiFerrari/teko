//! An election published on the bulletin board, audited independently:
//! entries are read back, proven included in a signed head, and the lifecycle
//! is replayed from scratch. Nothing here trusts the board operator.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use prost::Message;
use rand::SeedableRng;
use rand::rngs::StdRng;
use teko_bulletin::{
    BulletinBoard, Entry, EntryKind, MemoryBoard, sign_head, verify_entry_inclusion, verify_head,
};
use teko_crypto::{SigningKey, generate_signing_key, public_key_bytes};
use teko_election::{
    BALLOT_KIND_PUBLIC, ElectionError, ElectionManifest, ElectionOption, GovernancePolicy, Phase,
    Schedule, Transition, manifest_hash, replay, sign_manifest, sign_transition,
};

struct World {
    gov: Vec<SigningKey>,
    policy: GovernancePolicy,
    operator: SigningKey,
    manifest: ElectionManifest,
}

fn world() -> World {
    let mut rng = StdRng::seed_from_u64(77);
    let gov: Vec<SigningKey> = (0..3).map(|_| generate_signing_key(&mut rng)).collect();
    let policy = GovernancePolicy::new(gov.iter().map(public_key_bytes).collect(), 2).unwrap();
    let mut manifest = ElectionManifest {
        format_version: 1,
        election_id: "plebiscito-2026".into(),
        title: "Plebiscito de exemplo".into(),
        options: ["Sim", "Não"]
            .iter()
            .enumerate()
            .map(|(i, n)| ElectionOption {
                index: i as u32,
                name: (*n).into(),
                number: String::new(),
            })
            .collect(),
        schedule: Some(Schedule {
            registration_opens: 100,
            voting_opens: 200,
            voting_closes: 300,
        }),
        ballot_kind: BALLOT_KIND_PUBLIC.into(),
        registry_key: vec![],
        revision: 1,
        signatures: vec![],
    };
    manifest.signatures = [0usize, 1]
        .iter()
        .map(|i| sign_manifest(&gov[*i], &manifest))
        .collect();
    World {
        gov,
        policy,
        operator: generate_signing_key(&mut rng),
        manifest,
    }
}

fn transition(w: &World, from: Phase, to: Phase, seq: u64, at: u64) -> Transition {
    let mut t = Transition {
        election_id: w.manifest.election_id.clone(),
        manifest_hash: manifest_hash(&w.manifest).to_vec(),
        from_phase: from as i32,
        to_phase: to as i32,
        seq,
        at,
        reason: String::new(),
        signatures: vec![],
    };
    t.signatures = [1usize, 2]
        .iter()
        .map(|i| sign_transition(&w.gov[*i], &t))
        .collect();
    t
}

fn publish(w: &World, board: &mut MemoryBoard) -> Vec<Transition> {
    let ts = vec![
        transition(w, Phase::Draft, Phase::CandidatesPublished, 1, 50),
        transition(
            w,
            Phase::CandidatesPublished,
            Phase::RegistrationOpen,
            2,
            100,
        ),
        transition(w, Phase::RegistrationOpen, Phase::VotingOpen, 3, 200),
    ];
    board
        .append(Entry::new(EntryKind::MANIFEST, w.manifest.encode_to_vec()))
        .unwrap();
    for t in &ts {
        board
            .append(Entry::new(EntryKind::TRANSITION, t.encode_to_vec()))
            .unwrap();
    }
    ts
}

/// What an independent auditor does with nothing but the board and the policy.
fn audit(
    board: &MemoryBoard,
    operator_pk: &[u8],
    head: &teko_bulletin::SignedHead,
    policy: &GovernancePolicy,
) -> Result<Phase, String> {
    if !verify_head(operator_pk, head) {
        return Err("head signature".into());
    }
    let mut manifest = None;
    let mut transitions = Vec::new();
    for i in 0..head.size {
        let e = board.entry(i).ok_or("missing entry")?;
        let proof = board.inclusion_proof(i).map_err(|e| e.to_string())?;
        if !verify_entry_inclusion(e, i, head, &proof) {
            return Err(format!("entry {i} not included in the signed head"));
        }
        match e.kind {
            EntryKind::MANIFEST => {
                manifest = Some(
                    ElectionManifest::decode(e.payload.as_slice()).map_err(|e| e.to_string())?,
                )
            }
            EntryKind::TRANSITION => transitions
                .push(Transition::decode(e.payload.as_slice()).map_err(|e| e.to_string())?),
            _ => {}
        }
    }
    let manifest = manifest.ok_or("no manifest on the board")?;
    replay(policy, &manifest, &transitions)
        .map(|s| s.phase)
        .map_err(|e: ElectionError| e.to_string())
}

#[test]
fn an_auditor_replays_the_whole_election_from_the_board() {
    let w = world();
    let mut board = MemoryBoard::new();
    publish(&w, &mut board);
    let head = sign_head(&w.operator, board.len(), board.root());
    let phase = audit(&board, &public_key_bytes(&w.operator), &head, &w.policy).unwrap();
    assert_eq!(phase, Phase::VotingOpen);
    assert!(phase.can_vote());
}

#[test]
fn a_forged_transition_does_not_pass_the_audit_even_if_the_operator_cooperates() {
    let w = world();
    let mut board = MemoryBoard::new();
    publish(&w, &mut board);
    // The operator appends a transition signed by only ONE institution (a rogue admin).
    let mut rogue = transition(&w, Phase::VotingOpen, Phase::VotingClosed, 4, 300);
    rogue.signatures.truncate(1);
    board
        .append(Entry::new(EntryKind::TRANSITION, rogue.encode_to_vec()))
        .unwrap();
    let head = sign_head(&w.operator, board.len(), board.root());
    // Perfectly included and signed by the operator, but governance rejects it.
    let err = audit(&board, &public_key_bytes(&w.operator), &head, &w.policy).unwrap_err();
    assert!(err.contains("insufficient signatures"), "{err}");
}

#[test]
fn rewriting_history_is_caught_by_a_previously_pinned_head() {
    let w = world();
    let mut honest = MemoryBoard::new();
    publish(&w, &mut honest);
    let pinned = sign_head(&w.operator, honest.len(), honest.root());

    // Dishonest operator publishes a different history of the same length.
    let mut evil = MemoryBoard::new();
    evil.append(Entry::new(EntryKind::MANIFEST, w.manifest.encode_to_vec()))
        .unwrap();
    let swapped = transition(&w, Phase::Draft, Phase::CandidatesPublished, 1, 51); // other timestamp
    evil.append(Entry::new(EntryKind::TRANSITION, swapped.encode_to_vec()))
        .unwrap();
    for i in 2..honest.len() {
        evil.append(honest.entry(i).unwrap().clone()).unwrap();
    }
    assert_eq!(evil.len(), pinned.size);
    assert_ne!(evil.root(), pinned.root, "any rewrite changes the root");
    let p = evil.inclusion_proof(1).unwrap();
    assert!(
        !verify_entry_inclusion(evil.entry(1).unwrap(), 1, &pinned, &p),
        "the pinned head exposes the rewrite"
    );
}
