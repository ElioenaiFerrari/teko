//! The lifetime `eleito` credential (ADR-0040): created from an outcome certified by more than 2/3
//! of the vote chain's institutions, superseding the previous holder of the same office and
//! jurisdiction.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rand::SeedableRng;
use rand::rngs::StdRng;
use teko_credentials::candidate::{Candidacy, Cargo, candidate_claims};
use teko_credentials::*;
use teko_crypto::{SigningKey, generate_signing_key, public_key_bytes};
use teko_election::{
    ElectionOutcome, Office, OutcomeKind, Scope, count_outcome_signers, sign_outcome,
};
use teko_ledger::{Address, Ledger, LedgerError, Tx, TxError, address_of};

const CHAIN: &str = "teko-devnet-econ";
const VOTE_CHAIN: &str = "teko-devnet-1";
const P: Address = [9; 32];

struct W {
    l: Ledger,
    organ: SigningKey,
    inst: Vec<SigningKey>,
    now: u64,
}

fn key(seed: u64) -> SigningKey {
    generate_signing_key(&mut StdRng::seed_from_u64(seed))
}

fn world(institutions: usize) -> W {
    let organ = key(100);
    let inst: Vec<SigningKey> = (0..institutions).map(|i| key(200 + i as u64)).collect();
    let mut l = Ledger::new(CHAIN, vec![module()]).unwrap();
    genesis_type(
        &mut l,
        CANDIDATE_TYPE.as_bytes(),
        &[address_of(&public_key_bytes(&organ)).to_vec()],
        1,
        60,
        31_536_000,
        true,
    )
    .unwrap();
    genesis_system_type(&mut l, ELECTED_TYPE.as_bytes(), true).unwrap();
    let keys: Vec<Vec<u8>> = inst.iter().map(public_key_bytes).collect();
    genesis_vote_set(&mut l, VOTE_CHAIN, &keys).unwrap();
    W {
        l,
        organ,
        inst,
        now: 1_000_000,
    }
}

fn send(w: &mut W, sender: &SigningKey, op: Op, dt: u64) -> Result<(), String> {
    w.now += dt;
    let nonce = w.l.nonce_of(&address_of(&public_key_bytes(sender)));
    let t = Tx::new_signed(sender, CHAIN, nonce, CREDENTIALS_MODULE, op.encode());
    let h = w.l.height() + 1;
    match w.l.apply_block_at(h, w.now, &P, &[t]) {
        Ok(_) => Ok(()),
        Err(LedgerError::Tx {
            error: TxError::Module(m),
            ..
        }) => Err(m),
        Err(e) => Err(format!("{e:?}")),
    }
}

/// The organ issues a candidacy (a public credential, quorum 1) and returns its id and holder.
fn candidacy(w: &mut W, c: &Candidacy, holder_seed: u64) -> (Vec<u8>, Vec<u8>) {
    let claims = candidate_claims(c).unwrap().into_bytes();
    let holder = public_key_bytes(&key(holder_seed));
    let valid_until = w.now + 20_000_000;
    let commitment = public_commitment(&claims);
    let id = credential_id(
        CANDIDATE_TYPE.as_bytes(),
        &holder,
        &commitment,
        &claims,
        valid_until,
    );
    let organ = w.organ.clone();
    send(
        w,
        &organ,
        Op::Propose(ProposeOp {
            ctype: CANDIDATE_TYPE.as_bytes().to_vec(),
            subject: holder.clone(),
            commitment: commitment.to_vec(),
            claims,
            valid_until,
            sealed: vec![],
        }),
        1,
    )
    .unwrap();
    (id.to_vec(), holder)
}

fn cand(cargo: Cargo, uf: &str, numero: &str, nome: &str, eleicao: &str) -> Candidacy {
    Candidacy {
        cargo,
        uf: uf.into(),
        municipio: String::new(),
        numero: numero.into(),
        nome: nome.into(),
        eleicao: eleicao.into(),
        ..Default::default()
    }
}

fn outcome(
    winner: &[u8],
    office: Office,
    scope: Scope,
    election: &str,
    contest: &str,
) -> ElectionOutcome {
    ElectionOutcome {
        vote_chain_id: VOTE_CHAIN.into(),
        election_id: election.into(),
        contest_id: contest.into(),
        contest_hash: vec![5; 32],
        office: office as i32,
        scope: Some(scope),
        round: 1,
        kind: OutcomeKind::Elected as i32,
        winner_option: 0,
        winner_credential_id: winner.to_vec(),
        finalists: vec![],
        finalist_credentials: vec![],
        totals: vec![60, 40],
        accepted: 100,
        blank_votes: 0,
        null_votes: 0,
        drawn: false,
        draw_tied: vec![],
        draw_seed: vec![],
        seats: 0,
        elected: vec![],
        elected_credentials: vec![],
        ..Default::default()
    }
}

fn claim(w: &mut W, o: &ElectionOutcome, signers: &[usize]) -> Result<(), String> {
    let sigs: Vec<_> = signers
        .iter()
        .map(|i| sign_outcome(&w.inst[*i], o))
        .collect();
    let relayer = key(777);
    send(
        w,
        &relayer,
        Op::ClaimElected(ClaimElectedOp {
            outcome: prost::Message::encode_to_vec(o),
            signers: sigs.iter().map(|s| s.signer.clone()).collect(),
            signatures: sigs.iter().map(|s| s.signature.clone()).collect(),
        }),
        1,
    )
}

fn elected_of(w: &W, holder: &[u8]) -> Vec<(Vec<u8>, CredRec)> {
    credentials_of(&w.l, holder)
        .into_iter()
        .filter(|(t, _)| t == ELECTED_TYPE.as_bytes())
        .map(|(t, id)| {
            let r = credential(&w.l, &t, &id).unwrap();
            (id, r)
        })
        .collect()
}

#[test]
fn the_winner_gets_a_lifetime_credential_when_more_than_two_thirds_signed() {
    let mut w = world(4);
    let (id, holder) = candidacy(
        &mut w,
        &cand(Cargo::Governador, "SP", "13", "Ana Souza", "geral-2026"),
        1,
    );
    let o = outcome(
        &id,
        Office::Governor,
        Scope::state("SP"),
        "geral-2026",
        "governador-sp",
    );
    // 2 of 4 is not more than 2/3
    let e = claim(&mut w, &o, &[0, 1]).unwrap_err();
    assert!(e.contains("2/3"), "{e}");
    claim(&mut w, &o, &[0, 1, 2]).unwrap();
    let got = elected_of(&w, &holder);
    assert_eq!(got.len(), 1);
    let rec = &got[0].1;
    assert_eq!(rec.valid_until, 0, "lifetime");
    assert_eq!(rec.status_at(w.now + 1_000_000_000), Status::Active);
    let claims = String::from_utf8(rec.claims.clone()).unwrap();
    assert!(claims.starts_with("cargo=governador;uf=SP;numero=13;nome=Ana Souza;eleicao=geral-2026;pleito=governador-sp;votos=60;candidatura="), "{claims}");
    // the very same result cannot be claimed twice
    let e = claim(&mut w, &o, &[0, 1, 2, 3]).unwrap_err();
    assert!(e.contains("already claimed"), "{e}");
}

#[test]
fn three_institutions_need_all_three() {
    let mut w = world(3);
    let (id, _) = candidacy(
        &mut w,
        &cand(Cargo::Presidente, "", "10", "Pres", "geral-2026"),
        1,
    );
    let o = outcome(
        &id,
        Office::President,
        Scope::national(),
        "geral-2026",
        "presidente",
    );
    assert!(claim(&mut w, &o, &[0, 1]).is_err());
    claim(&mut w, &o, &[0, 1, 2]).unwrap();
}

#[test]
fn a_signature_over_other_bytes_a_stranger_or_another_chain_does_not_count() {
    let mut w = world(4);
    let (id, _) = candidacy(
        &mut w,
        &cand(Cargo::Governador, "SP", "13", "Ana", "geral-2026"),
        1,
    );
    let o = outcome(
        &id,
        Office::Governor,
        Scope::state("SP"),
        "geral-2026",
        "governador-sp",
    );
    // signatures over ANOTHER outcome (a different winner option)
    let mut other = o.clone();
    other.winner_option = 1;
    let sigs: Vec<_> = (0..3).map(|i| sign_outcome(&w.inst[i], &other)).collect();
    let relayer = key(777);
    let e = send(
        &mut w,
        &relayer,
        Op::ClaimElected(ClaimElectedOp {
            outcome: prost::Message::encode_to_vec(&o),
            signers: sigs.iter().map(|s| s.signer.clone()).collect(),
            signatures: sigs.iter().map(|s| s.signature.clone()).collect(),
        }),
        1,
    )
    .unwrap_err();
    assert!(e.contains("2/3"), "{e}");
    // strangers (not institutions) signing
    let strangers: Vec<SigningKey> = (0..4).map(|i| key(900 + i)).collect();
    let sigs: Vec<_> = strangers.iter().map(|k| sign_outcome(k, &o)).collect();
    assert_eq!(
        count_outcome_signers(
            &w.inst.iter().map(public_key_bytes).collect::<Vec<_>>(),
            &o,
            &sigs
        ),
        0
    );
    // an outcome of ANOTHER vote chain
    let mut foreign = o.clone();
    foreign.vote_chain_id = "outra-rede".into();
    assert!(
        claim(&mut w, &foreign, &[0, 1, 2])
            .unwrap_err()
            .contains("another vote chain")
    );
}

#[test]
fn only_a_decided_contest_with_a_matching_candidacy_elects() {
    let mut w = world(4);
    let (id, _) = candidacy(
        &mut w,
        &cand(Cargo::Governador, "SP", "13", "Ana", "geral-2026"),
        1,
    );
    // a runoff or a tie elects nobody
    for kind in [OutcomeKind::Runoff, OutcomeKind::Tie] {
        let mut o = outcome(
            &id,
            Office::Governor,
            Scope::state("SP"),
            "geral-2026",
            "governador-sp",
        );
        o.kind = kind as i32;
        assert!(
            claim(&mut w, &o, &[0, 1, 2])
                .unwrap_err()
                .contains("decided"),
            "{kind:?}"
        );
    }
    // the winner must be a candidacy of THIS contest: another state, office or election
    let rj = outcome(
        &id,
        Office::Governor,
        Scope::state("RJ"),
        "geral-2026",
        "governador-rj",
    );
    assert!(
        claim(&mut w, &rj, &[0, 1, 2])
            .unwrap_err()
            .contains("not for this contest")
    );
    let pres = outcome(
        &id,
        Office::President,
        Scope::national(),
        "geral-2026",
        "presidente",
    );
    assert!(
        claim(&mut w, &pres, &[0, 1, 2])
            .unwrap_err()
            .contains("not for this contest")
    );
    let other = outcome(
        &id,
        Office::Governor,
        Scope::state("SP"),
        "outra-2030",
        "governador-sp",
    );
    assert!(
        claim(&mut w, &other, &[0, 1, 2])
            .unwrap_err()
            .contains("not for this contest")
    );
    // an unknown credential, and an office elected automatically only when single-seat
    let unknown = outcome(
        &[1; 32],
        Office::Governor,
        Scope::state("SP"),
        "geral-2026",
        "governador-sp",
    );
    assert!(claim(&mut w, &unknown, &[0, 1, 2]).is_err());
    // a proportional office elects a CHAMBER (see the chamber tests): a result naming one winner is
    // refused (a senator is elected too, see the senate tests)
    for office in [
        Office::FederalDeputy,
        Office::StateDeputy,
        Office::CityCouncillor,
    ] {
        let scope = if office == Office::CityCouncillor {
            Scope::municipality("3550308")
        } else {
            Scope::state("SP")
        };
        let prop = outcome(&id, office, scope, "geral-2026", "dep-sp");
        assert!(
            claim(&mut w, &prop, &[0, 1, 2])
                .unwrap_err()
                .contains("chamber"),
            "{office:?}"
        );
    }
    // office and jurisdiction must agree
    let bad = outcome(
        &id,
        Office::President,
        Scope::state("SP"),
        "geral-2026",
        "x",
    );
    assert!(
        claim(&mut w, &bad, &[0, 1, 2])
            .unwrap_err()
            .contains("do not match")
    );
}

#[test]
fn a_revoked_or_vetoed_candidacy_cannot_win_a_credential() {
    let mut w = world(4);
    let (id, _) = candidacy(
        &mut w,
        &cand(Cargo::Governador, "SP", "13", "Ana", "geral-2026"),
        1,
    );
    // the organ revokes the candidacy after it became active
    let organ = w.organ.clone();
    w.now += 100;
    send(
        &mut w,
        &organ,
        Op::Revoke(VoteOp {
            ctype: CANDIDATE_TYPE.as_bytes().to_vec(),
            id: id.clone(),
            reason: "inelegível".into(),
        }),
        0,
    )
    .unwrap();
    let o = outcome(
        &id,
        Office::Governor,
        Scope::state("SP"),
        "geral-2026",
        "governador-sp",
    );
    assert!(
        claim(&mut w, &o, &[0, 1, 2])
            .unwrap_err()
            .contains("revoked")
    );
}

#[test]
fn the_next_elected_of_the_same_office_supersedes_the_previous_one() {
    let mut w = world(4);
    let (a, holder_a) = candidacy(
        &mut w,
        &cand(Cargo::Governador, "SP", "13", "Ana", "geral-2026"),
        1,
    );
    let (b, holder_b) = candidacy(
        &mut w,
        &cand(Cargo::Governador, "SP", "22", "Beto", "geral-2030"),
        2,
    );
    let (rio, holder_rio) = candidacy(
        &mut w,
        &cand(Cargo::Governador, "RJ", "33", "Rio", "geral-2030"),
        3,
    );
    claim(
        &mut w,
        &outcome(
            &a,
            Office::Governor,
            Scope::state("SP"),
            "geral-2026",
            "governador-sp",
        ),
        &[0, 1, 2],
    )
    .unwrap();
    let first = elected_of(&w, &holder_a);
    assert_eq!(first[0].1.status_at(w.now), Status::Active);
    // another STATE does not interfere
    claim(
        &mut w,
        &outcome(
            &rio,
            Office::Governor,
            Scope::state("RJ"),
            "geral-2030",
            "governador-rj",
        ),
        &[0, 1, 2],
    )
    .unwrap();
    assert_eq!(
        elected_of(&w, &holder_a)[0].1.status_at(w.now),
        Status::Active
    );
    // the next election in SP: Beto takes over, Ana loses her credential (the record stays)
    claim(
        &mut w,
        &outcome(
            &b,
            Office::Governor,
            Scope::state("SP"),
            "geral-2030",
            "governador-sp",
        ),
        &[0, 1, 2],
    )
    .unwrap();
    assert_eq!(
        elected_of(&w, &holder_a)[0].1.status_at(w.now),
        Status::Superseded
    );
    assert_eq!(
        elected_of(&w, &holder_b)[0].1.status_at(w.now),
        Status::Active
    );
    assert_eq!(
        elected_of(&w, &holder_rio)[0].1.status_at(w.now),
        Status::Active
    );
}

#[test]
fn a_reelected_person_gets_a_new_credential_and_the_old_one_is_superseded() {
    let mut w = world(4);
    let (a1, holder) = candidacy(
        &mut w,
        &cand(Cargo::Governador, "SP", "13", "Ana", "geral-2026"),
        1,
    );
    // the SAME person (same credential key) stands again in the next election
    let claims = candidate_claims(&cand(Cargo::Governador, "SP", "13", "Ana", "geral-2030"))
        .unwrap()
        .into_bytes();
    let valid_until = w.now + 20_000_000;
    let commitment = public_commitment(&claims);
    let a2 = credential_id(
        CANDIDATE_TYPE.as_bytes(),
        &holder,
        &commitment,
        &claims,
        valid_until,
    )
    .to_vec();
    let organ = w.organ.clone();
    send(
        &mut w,
        &organ,
        Op::Propose(ProposeOp {
            ctype: CANDIDATE_TYPE.as_bytes().to_vec(),
            subject: holder.clone(),
            commitment: commitment.to_vec(),
            claims,
            valid_until,
            sealed: vec![],
        }),
        1,
    )
    .unwrap();
    claim(
        &mut w,
        &outcome(
            &a1,
            Office::Governor,
            Scope::state("SP"),
            "geral-2026",
            "governador-sp",
        ),
        &[0, 1, 2],
    )
    .unwrap();
    claim(
        &mut w,
        &outcome(
            &a2,
            Office::Governor,
            Scope::state("SP"),
            "geral-2030",
            "governador-sp",
        ),
        &[0, 1, 2],
    )
    .unwrap();
    let all = elected_of(&w, &holder);
    assert_eq!(all.len(), 2);
    let active = all
        .iter()
        .filter(|(_, r)| r.status_at(w.now) == Status::Active)
        .count();
    assert_eq!(active, 1, "only the newest is active");
    let n = all
        .iter()
        .filter(|(_, r)| r.status_at(w.now) == Status::Superseded)
        .count();
    assert_eq!(n, 1);
}

#[test]
fn the_elected_type_has_no_organ_so_nobody_can_propose_it() {
    let mut w = world(4);
    let organ = w.organ.clone();
    let holder = public_key_bytes(&key(5));
    let claims = b"cargo=governador".to_vec();
    let e = send(
        &mut w,
        &organ,
        Op::Propose(ProposeOp {
            ctype: ELECTED_TYPE.as_bytes().to_vec(),
            subject: holder,
            commitment: public_commitment(&claims).to_vec(),
            claims,
            valid_until: 0,
            sealed: vec![],
        }),
        1,
    )
    .unwrap_err();
    assert!(e.contains("member"), "{e}");
}

#[test]
fn a_network_without_vote_chain_institutions_accepts_no_claim() {
    let mut l = Ledger::new(CHAIN, vec![module()]).unwrap();
    genesis_system_type(&mut l, ELECTED_TYPE.as_bytes(), true).unwrap();
    let mut w = W {
        l,
        organ: key(100),
        inst: (0..4).map(|i| key(200 + i)).collect(),
        now: 1_000_000,
    };
    let o = outcome(&[1; 32], Office::Governor, Scope::state("SP"), "e", "c");
    assert!(
        claim(&mut w, &o, &[0, 1, 2])
            .unwrap_err()
            .contains("no vote chain")
    );
}

#[test]
fn a_drawn_winner_elects_like_any_other_the_ledger_trusts_the_institutions_signatures() {
    let mut w = world(4);
    let (id, holder) = candidacy(
        &mut w,
        &cand(Cargo::Governador, "SP", "13", "Ana", "geral-2026"),
        1,
    );
    // a tie at the top, broken by the verifiable draw (ADR-0043): same kind, plus the evidence
    let mut o = outcome(
        &id,
        Office::Governor,
        Scope::state("SP"),
        "geral-2026",
        "governador-sp",
    );
    o.totals = vec![50, 50];
    o.drawn = true;
    o.draw_tied = vec![0, 1];
    o.draw_seed = vec![9; 32];
    claim(&mut w, &o, &[0, 1, 2]).unwrap();
    assert_eq!(elected_of(&w, &holder).len(), 1);
    // a tie still waiting for the draw (no seed) elects nobody
    let mut tie = outcome(
        &id,
        Office::Governor,
        Scope::state("SP"),
        "geral-2026",
        "governador-rj",
    );
    tie.kind = OutcomeKind::Tie as i32;
    assert!(
        claim(&mut w, &tie, &[0, 1, 2])
            .unwrap_err()
            .contains("decided")
    );
}

/// A senate result (ADR-0044): `seats` seats, the winners (option index + candidacy id), best first.
fn senate_outcome(
    winners: &[(u32, &[u8])],
    seats: u32,
    election: &str,
    contest: &str,
) -> ElectionOutcome {
    let mut o = outcome(&[], Office::Senator, Scope::state("SP"), election, contest);
    o.totals = vec![40, 30, 20, 10];
    o.accepted = 100;
    o.seats = seats;
    o.elected = winners.iter().map(|(i, _)| *i).collect();
    o.elected_credentials = winners.iter().map(|(_, id)| id.to_vec()).collect();
    o
}

#[test]
fn a_senate_result_elects_all_its_winners_in_one_claim_and_each_class_replaces_only_itself() {
    let mut w = world(4);
    let (a, ha) = candidacy(
        &mut w,
        &cand(Cargo::Senador, "SP", "450", "Ana", "e-2026"),
        1,
    );
    let (b, hb) = candidacy(
        &mut w,
        &cand(Cargo::Senador, "SP", "451", "Beto", "e-2026"),
        2,
    );
    let (c, hc) = candidacy(
        &mut w,
        &cand(Cargo::Senador, "SP", "452", "Cris", "e-2030"),
        3,
    );
    let (d, hd) = candidacy(
        &mut w,
        &cand(Cargo::Senador, "SP", "453", "Dani", "e-2030"),
        4,
    );
    let (x, _) = candidacy(
        &mut w,
        &cand(Cargo::Senador, "RJ", "454", "Edu", "e-2026"),
        5,
    );

    // 2 seats: both winners get a lifetime credential from ONE claim, with their own votes
    let two = senate_outcome(&[(0, &a), (1, &b)], 2, "e-2026", "senador-sp-2026");
    claim(&mut w, &two, &[0, 1, 2]).unwrap();
    let (ea, eb) = (elected_of(&w, &ha), elected_of(&w, &hb));
    assert_eq!((ea.len(), eb.len()), (1, 1));
    let claims = String::from_utf8(ea[0].1.claims.clone()).unwrap();
    assert!(claims.starts_with("cargo=senador;uf=SP;numero=450;nome=Ana;eleicao=e-2026;pleito=senador-sp-2026;votos=40;"), "{claims}");
    assert!(
        String::from_utf8(eb[0].1.claims.clone())
            .unwrap()
            .contains("votos=30;")
    );
    assert_eq!(ea[0].1.valid_until, 0);
    // the same result cannot be claimed twice
    assert!(
        claim(&mut w, &two, &[0, 1, 2, 3])
            .unwrap_err()
            .contains("already claimed")
    );

    // a 1-seat contest of the same state is ANOTHER class: it does not replace the 2-seat holders
    let one = senate_outcome(&[(0, &c)], 1, "e-2030", "senador-sp-2030a");
    claim(&mut w, &one, &[0, 1, 2]).unwrap();
    assert_eq!(
        elected_of(&w, &ha)[0].1.status_at(w.now + 1_000),
        Status::Active
    );
    assert_eq!(
        elected_of(&w, &hb)[0].1.status_at(w.now + 1_000),
        Status::Active
    );
    assert_eq!(elected_of(&w, &hc).len(), 1);

    // the NEXT 2-seat election supersedes BOTH previous 2-seat holders (and only them)
    let next = senate_outcome(&[(0, &d), (1, &x)], 2, "e-2026", "senador-sp-next");
    // x is a candidacy of RJ: the whole claim is refused and nothing is issued
    let e = claim(&mut w, &next, &[0, 1, 2]).unwrap_err();
    assert!(e.contains("not for this contest"), "{e}");
    assert_eq!(elected_of(&w, &hd).len(), 0, "all or nothing");
}

#[test]
fn malformed_senate_results_are_refused() {
    let mut w = world(4);
    let (a, _) = candidacy(
        &mut w,
        &cand(Cargo::Senador, "SP", "450", "Ana", "e-2026"),
        1,
    );
    let (b, _) = candidacy(
        &mut w,
        &cand(Cargo::Senador, "SP", "451", "Beto", "e-2026"),
        2,
    );
    let (c, _) = candidacy(
        &mut w,
        &cand(Cargo::Senador, "SP", "452", "Cris", "e-2026"),
        3,
    );
    // more winners than seats, an empty list, a repeated winner, lists of different lengths, 3 seats
    let bad = [
        senate_outcome(&[(0, &a), (1, &b), (2, &c)], 2, "e-2026", "s1"),
        senate_outcome(&[], 2, "e-2026", "s2"),
        senate_outcome(&[(0, &a), (0, &b)], 2, "e-2026", "s3"),
        senate_outcome(&[(0, &a), (1, &b)], 3, "e-2026", "s4"),
        senate_outcome(&[(0, &a), (1, &b)], 0, "e-2026", "s5"),
    ];
    for o in &bad {
        assert!(claim(&mut w, o, &[0, 1, 2]).is_err(), "{}", o.contest_id);
    }
    let mut lengths = senate_outcome(&[(0, &a), (1, &b)], 2, "e-2026", "s6");
    lengths.elected_credentials.pop();
    assert!(claim(&mut w, &lengths, &[0, 1, 2]).is_err());
    // a single-seat office does not carry a list
    let mut gov = outcome(&a, Office::Governor, Scope::state("SP"), "e-2026", "g1");
    gov.elected = vec![0];
    assert!(claim(&mut w, &gov, &[0, 1, 2]).is_err());
    // the sound result still works
    claim(
        &mut w,
        &senate_outcome(&[(0, &a), (1, &b)], 2, "e-2026", "ok"),
        &[0, 1, 2],
    )
    .unwrap();
}

// ───────────────────────── rotation of the vote chain institutions (ADR-0045) ─────────────────────────

fn rotation(
    newcomer: &SigningKey,
    epoch: u64,
    prev_epoch_keys: &[SigningKey],
    approvers: &[usize],
) -> teko_bft::ValidatorSetChange {
    let current: Vec<Vec<u8>> = prev_epoch_keys.iter().map(public_key_bytes).collect();
    let mut new = current.clone();
    let last = new.len() - 1;
    new[last] = public_key_bytes(newcomer);
    let mut c = teko_bft::ValidatorSetChange {
        chain_id: VOTE_CHAIN.into(),
        epoch,
        effective_height: 100,
        prev_set_hash: teko_bft::set_hash(&current).to_vec(),
        validators: new,
        ..Default::default()
    };
    for i in approvers {
        c.sign(&prev_epoch_keys[*i]);
    }
    c
}

#[test]
fn a_rotation_approved_by_more_than_two_thirds_changes_who_can_certify_a_result() {
    let mut w = world(4);
    let (id, holder) = candidacy(
        &mut w,
        &cand(Cargo::Governador, "SP", "13", "Ana", "geral-2026"),
        1,
    );
    let newcomer = key(900);
    let old = w.inst.clone();
    let relayer = key(777);
    // 2 of 4 is not more than 2/3; a wrong epoch and a wrong replaced set are refused
    let e = send(
        &mut w,
        &relayer,
        Op::RotateVoteSet(rotation(&newcomer, 1, &old, &[0, 1])),
        1,
    )
    .unwrap_err();
    assert!(e.contains("2/3"), "{e}");
    let e = send(
        &mut w,
        &relayer,
        Op::RotateVoteSet(rotation(&newcomer, 2, &old, &[0, 1, 2])),
        1,
    )
    .unwrap_err();
    assert!(e.contains("epoch"), "{e}");
    let mut other = old.clone();
    other[0] = key(901);
    let e = send(
        &mut w,
        &relayer,
        Op::RotateVoteSet(rotation(&newcomer, 1, &other, &[0, 1, 2])),
        1,
    )
    .unwrap_err();
    assert!(e.contains("replace"), "{e}");
    // 3 of 4 approve: the stored set changes, and the same change cannot be replayed
    let change = rotation(&newcomer, 1, &old, &[0, 1, 2]);
    send(&mut w, &relayer, Op::RotateVoteSet(change.clone()), 1).unwrap();
    assert!(send(&mut w, &relayer, Op::RotateVoteSet(change), 1).is_err());

    // a result signed by the OLD set (with the outgoing institution) no longer reaches 2/3
    let o = outcome(
        &id,
        Office::Governor,
        Scope::state("SP"),
        "geral-2026",
        "governador-sp",
    );
    let sign_with = |keys: &[&SigningKey]| {
        let sigs: Vec<_> = keys.iter().map(|k| sign_outcome(k, &o)).collect();
        Op::ClaimElected(ClaimElectedOp {
            outcome: prost::Message::encode_to_vec(&o),
            signers: sigs.iter().map(|s| s.signer.clone()).collect(),
            signatures: sigs.iter().map(|s| s.signature.clone()).collect(),
        })
    };
    let (i0, i1, i3) = (&old[0], &old[1], &old[3]);
    let e = send(&mut w, &relayer, sign_with(&[i0, i1, i3]), 1).unwrap_err();
    assert!(
        e.contains("2/3"),
        "the outgoing institution does not count: {e}"
    );
    // with the newcomer in its place the claim works
    send(&mut w, &relayer, sign_with(&[i0, i1, &newcomer]), 1).unwrap();
    assert_eq!(elected_of(&w, &holder).len(), 1);
}

fn deputy(partido: &str, legenda: &str, numero: &str, nome: &str) -> Candidacy {
    Candidacy {
        cargo: Cargo::DeputadoEstadual,
        uf: "SP".into(),
        partido: partido.into(),
        legenda: legenda.into(),
        numero: numero.into(),
        nome: nome.into(),
        eleicao: "e-2026".into(),
        ..Default::default()
    }
}

/// A chamber result (ADR-0046): options [P13, a, b, P45, c, d], `elected` = (option, candidacy id).
fn chamber_outcome(
    elected: &[(u32, &[u8])],
    seats: u32,
    parties: &[(&str, u32)],
) -> ElectionOutcome {
    let mut o = outcome(
        &[],
        Office::StateDeputy,
        Scope::state("SP"),
        "e-2026",
        "dep-sp-2026",
    );
    o.totals = vec![10, 50, 30, 5, 40, 20, 4, 1];
    o.accepted = 160;
    o.seats = seats;
    o.elected = elected.iter().map(|(i, _)| *i).collect();
    o.elected_credentials = elected.iter().map(|(_, id)| id.to_vec()).collect();
    o.quotient = 50;
    o.parties = parties
        .iter()
        .enumerate()
        .map(|(i, (n, s))| teko_election::PartyResult {
            option: (i * 3) as u32,
            votes: 100,
            seats: *s,
            number: (*n).into(),
        })
        .collect();
    o
}

#[test]
fn a_chamber_elects_every_winner_in_one_claim_with_its_party_and_the_next_chamber_replaces_it() {
    let mut w = world(4);
    let (a, ha) = candidacy(&mut w, &deputy("13", "Verde", "13001", "Ana"), 1);
    let (b, hb) = candidacy(&mut w, &deputy("13", "Verde", "13002", "Beto"), 2);
    let (c, hc) = candidacy(&mut w, &deputy("45", "Azul", "45001", "Cris"), 3);
    let o = chamber_outcome(&[(1, &a), (2, &b), (4, &c)], 3, &[("13", 2), ("45", 1)]);
    claim(&mut w, &o, &[0, 1, 2]).unwrap();
    let (ea, eb, ec) = (
        elected_of(&w, &ha),
        elected_of(&w, &hb),
        elected_of(&w, &hc),
    );
    assert_eq!((ea.len(), eb.len(), ec.len()), (1, 1, 1));
    let claims = String::from_utf8(ea[0].1.claims.clone()).unwrap();
    assert!(
        claims.starts_with("cargo=deputado-estadual;uf=SP;partido=13;numero=13001;nome=Ana;eleicao=e-2026;pleito=dep-sp-2026;votos=50;"),
        "{claims}"
    );
    assert!(
        String::from_utf8(ec[0].1.claims.clone())
            .unwrap()
            .contains("partido=45;")
    );
    // the next chamber of the same office and state supersedes all of this one
    let (d, hd) = candidacy(&mut w, &deputy("45", "Azul", "45002", "Dani"), 4);
    let mut next = chamber_outcome(&[(5, &d)], 3, &[("13", 0), ("45", 1)]);
    next.contest_id = "dep-sp-2030".into();
    claim(&mut w, &next, &[0, 1, 2]).unwrap();
    assert_eq!(elected_of(&w, &hd).len(), 1);
    assert_ne!(
        elected_of(&w, &ha)[0].1.status_at(w.now + 1_000),
        Status::Active
    );
    assert_ne!(
        elected_of(&w, &hc)[0].1.status_at(w.now + 1_000),
        Status::Active
    );
}

#[test]
fn a_chamber_result_that_does_not_add_up_is_refused_and_nothing_is_issued() {
    let mut w = world(4);
    let (a, ha) = candidacy(&mut w, &deputy("13", "Verde", "13001", "Ana"), 1);
    let (b, _) = candidacy(&mut w, &deputy("13", "Verde", "13002", "Beto"), 2);
    let (c, _) = candidacy(&mut w, &deputy("45", "Azul", "45001", "Cris"), 3);
    // more winners than seats
    let e = claim(
        &mut w,
        &chamber_outcome(&[(1, &a), (2, &b), (4, &c)], 2, &[("13", 2), ("45", 1)]),
        &[0, 1, 2],
    )
    .unwrap_err();
    assert!(e.contains("chamber"), "{e}");
    // the parties' seats do not add up to the winners
    let e = claim(
        &mut w,
        &chamber_outcome(&[(1, &a), (2, &b)], 3, &[("13", 1), ("45", 0)]),
        &[0, 1, 2],
    )
    .unwrap_err();
    assert!(e.contains("chamber"), "{e}");
    // a winner whose candidacy belongs to another party than the result says
    let e = claim(
        &mut w,
        &chamber_outcome(&[(1, &a), (4, &c)], 2, &[("13", 0), ("45", 2)]),
        &[0, 1, 2],
    )
    .unwrap_err();
    assert!(e.contains("party"), "{e}");
    // the same winner twice
    let e = claim(
        &mut w,
        &chamber_outcome(&[(1, &a), (1, &a)], 3, &[("13", 2), ("45", 0)]),
        &[0, 1, 2],
    )
    .unwrap_err();
    assert!(e.contains("twice"), "{e}");
    // a candidate of the state's SENATE race cannot take a deputy seat
    assert_eq!(elected_of(&w, &ha).len(), 0, "all or nothing");
}
