//! End-to-end: emulator (scripted buttons) + issuer-cli + headless Suite client,
//! talking over real TCP with the real framing.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::net::{SocketAddr, TcpListener};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use device_emulator::{LinkSlot, Linked, serve};
use issuer_cli::{Issuer, Subject, scratch_dir};
use prost::Message;
use rand::SeedableRng;
use rand::rand_core::UnwrapErr;
use rand::rngs::SysRng;
use teko_attestation::{FirmwareManifest, TrustPolicy};
use teko_client::{
    ClientError, DeviceClient, UnlockOutcome, inspect_firmware, verify_signed_document,
};
use teko_device_core::testing::ScriptedHal;
use teko_device_core::{Decision, Device, DeviceConfig};
use teko_protocol::{ErrorCode, FirmwareBegin, Predicate, PredicateKind, Request};
use teko_transport::{TcpTransport, connect_tcp};

const PIN: &str = "482913";

struct Rig {
    addr: SocketAddr,
    issuer: Issuer,
    trust: TrustPolicy,
    hal: ScriptedHal,
}

fn start() -> Rig {
    let issuer = Issuer::open(scratch_dir("e2e"));
    let trust = issuer.dev_setup().unwrap();
    let factory = issuer.provision_device().unwrap();
    let hal = ScriptedHal::new();
    hal.set_default_pin(Some(PIN));
    // The scripted user copies the recovery words from the device screen.
    hal.set_auto_words(true);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let (hal2, trust2) = (hal.clone(), trust.clone());
    std::thread::spawn(move || {
        let slot = LinkSlot::default();
        let cfg = DeviceConfig {
            pin_kdf_iterations: 50,
            ..DeviceConfig::default()
        };
        let mut dev = Device::new(
            Linked::new(hal2, slot.clone()),
            UnwrapErr(SysRng),
            cfg,
            trust2,
            Some(factory),
        )
        .unwrap()
        .with_registration_blinder(Box::new(teko_registration::RsaRegistration));
        let _ = serve(&listener, &mut dev, &slot, |_| {});
    });
    Rig {
        addr,
        issuer,
        trust,
        hal,
    }
}

fn connect(rig: &Rig) -> DeviceClient<TcpTransport> {
    DeviceClient::new(connect_tcp(rig.addr, Duration::from_secs(3)).unwrap())
}

fn firmware(rig: &Rig, v: u32) -> (Vec<u8>, Vec<u8>) {
    let d = rig.issuer.dir();
    (
        fs::read(d.join(format!("firmware-v{v}.manifest"))).unwrap(),
        fs::read(d.join(format!("firmware-v{v}.blob"))).unwrap(),
    )
}

fn onboard(rig: &Rig, c: &mut DeviceClient<TcpTransport>) {
    let (m, b) = firmware(rig, 1);
    c.install_firmware(&rig.trust, &m, &b, &mut |_, _| {})
        .unwrap();
    c.setup_pin().unwrap();
    let wallet = c.wallet_setup().unwrap();
    assert!(wallet.address.starts_with("tekow1"));
    let id = c.civic_id().unwrap();
    assert!(id.id.starts_with("teko1"));
    let nonce = Issuer::new_nonce();
    let req = c.identity_request(&nonce, "Autoridade Cívica").unwrap();
    let subject = Subject {
        name: "Maria da Silva".into(),
        birth_date: "1990-05-17".into(),
        nationality: "BR".into(),
        electoral_state: "SP".into(),
        electoral_municipality: "3550308".into(),
    };
    let issued = rig.issuer.issue_credential(&nonce, &req, &subject).unwrap();
    c.load_credential(&issued.encode_to_vec()).unwrap();
}

fn device_code(e: ClientError) -> ErrorCode {
    match e {
        ClientError::Device(f) => f.code,
        other => panic!("expected a device failure, got {other:?}"),
    }
}

#[test]
fn full_onboarding_and_daily_use() {
    let rig = start();
    let mut c = connect(&rig);
    let prompts = Arc::new(AtomicUsize::new(0));
    let p2 = prompts.clone();
    c.set_on_button(move |_| {
        p2.fetch_add(1, Ordering::SeqCst);
    });

    // 1. detect + 2. authenticity
    let st = c.initialize().unwrap();
    assert_eq!(st.state, "virgin");
    let rep = c.verify_authenticity(&rig.trust).unwrap();
    assert!(rep.genuine, "{rep:?}");
    assert_eq!(rep.batch_id.as_deref(), Some("lote-dev-001"));

    // 3. firmware (progress is reported; fingerprint is checkable)
    let (m, b) = firmware(&rig, 1);
    let info = inspect_firmware(&rig.trust, &m, &b, 0).unwrap();
    assert_eq!(
        (info.valid_signers, info.threshold, info.total_signers),
        (2, 2, 3)
    );
    let mut ticks = 0;
    let v = c
        .install_firmware(&rig.trust, &m, &b, &mut |_, _| ticks += 1)
        .unwrap();
    assert_eq!(v, 1);
    assert!(ticks > 0);
    let device_fp = rig
        .hal
        .screens()
        .into_iter()
        .find(|s| s.title == "INSTALAR FIRMWARE?")
        .unwrap();
    assert!(
        device_fp.lines.contains(&info.fingerprint),
        "fingerprint must match on both screens"
    );

    // 4. PIN on the device, keys, 5. issuance
    c.setup_pin().unwrap();
    let wallet = c.wallet_setup().unwrap();
    assert!(wallet.address.starts_with("tekow1"));
    assert_eq!(c.features().unwrap().state, "initialized");
    let id = c.civic_id().unwrap();
    let on_device: String = rig
        .hal
        .screens()
        .into_iter()
        .rev()
        .find(|s| s.title == "CONFERIR ID CÍVICO")
        .unwrap()
        .lines
        .concat();
    assert!(
        on_device.contains(&id.id),
        "civic id shown on the device equals the Suite's"
    );

    let nonce = Issuer::new_nonce();
    let req = c.identity_request(&nonce, "Autoridade Cívica").unwrap();
    let subject = Subject {
        name: "Maria da Silva".into(),
        birth_date: "1990-05-17".into(),
        nationality: "BR".into(),
        electoral_state: "SP".into(),
        electoral_municipality: "3550308".into(),
    };
    let issued = rig.issuer.issue_credential(&nonce, &req, &subject).unwrap();
    c.load_credential(&issued.encode_to_vec()).unwrap();
    let st = c.features().unwrap();
    assert_eq!(
        (st.state.as_str(), st.has_credential),
        ("credentials_issued", true)
    );
    assert!(
        prompts.load(Ordering::SeqCst) >= 5,
        "Suite saw 'confirm on device' prompts"
    );

    // sign a document: file -> hash -> device confirmation -> verifiable signature
    let file = b"Contrato de locacao\nValor: R$ 1.000,00\n".to_vec();
    let doc = c.sign_document(&file, "Contrato de locação").unwrap();
    assert_eq!(doc.civic_id, id.id);
    let ok = verify_signed_document(&doc, Some(&file)).unwrap();
    assert_eq!(ok.matches_file, Some(true));
    assert!(verify_signed_document(&doc, Some(b"outro arquivo")).is_err());
    let mut forged = doc.clone();
    forged.hash_sha3_256 = "00".repeat(32);
    assert!(verify_signed_document(&forged, None).is_err());
    // The hash shown on the device screen is the file's SHA3-256.
    let shown = rig
        .hal
        .screens()
        .into_iter()
        .rev()
        .find(|s| s.title == "ASSINAR DOCUMENTO?")
        .unwrap();
    assert!(
        shown
            .lines
            .concat()
            .replace(' ', "")
            .contains(&doc.hash_sha3_256)
    );

    // prove an attribute without revealing the birth date
    let proof = c
        .prove_attribute(
            &rig.trust,
            Predicate {
                kind: PredicateKind::AgeAtLeast as i32,
                arg: 18,
                name: String::new(),
            },
            "Loja Exemplo",
        )
        .unwrap();
    assert!(proof.verified);
    assert_eq!(
        proof.disclosed,
        vec![("age_over_18".to_string(), "true".to_string())]
    );
    assert!(!format!("{proof:?}").contains("1990"));

    // pairwise ids
    let a = c.pairwise_id("banco.exemplo").unwrap();
    assert_ne!(a, c.pairwise_id("loja.exemplo").unwrap());

    // firmware update v1 -> v2 while unlocked; downgrade refused by Suite AND device
    let (m2, b2) = firmware(&rig, 2);
    assert_eq!(
        c.install_firmware(&rig.trust, &m2, &b2, &mut |_, _| {})
            .unwrap(),
        2
    );
    assert!(matches!(
        c.install_firmware(&rig.trust, &m, &b, &mut |_, _| {}),
        Err(ClientError::Verification(_))
    ));
    let raw = c.request(Request::FirmwareBegin(FirmwareBegin {
        manifest: m.clone(),
    }));
    assert_eq!(device_code(raw.unwrap_err()), ErrorCode::Downgrade);

    // reconnecting = replugging: the device is locked again until the PIN is typed
    drop(c);
    let mut c = connect(&rig);
    let st = c.initialize().unwrap();
    assert!(st.locked);
    assert_eq!(
        device_code(c.sign_document(b"x", "x").unwrap_err()),
        ErrorCode::InvalidState
    );
    rig.hal.set_default_pin(Some("135792"));
    assert_eq!(
        c.unlock().unwrap(),
        UnlockOutcome::WrongPin { remaining: 7 }
    );
    rig.hal.set_default_pin(Some(PIN));
    rig.hal.advance_ms(60_000);
    assert_eq!(c.unlock().unwrap(), UnlockOutcome::Unlocked);
    assert!(c.sign_document(b"depois do desbloqueio", "ok").is_ok());

    // explicit wipe, confirmed on the device
    c.wipe().unwrap();
    assert_eq!(c.features().unwrap().state, "firmware_installed");
    let _ = fs::remove_dir_all(rig.issuer.dir());
}

#[test]
fn firmware_without_m_of_n_never_reaches_the_device() {
    let rig = start();
    let mut c = connect(&rig);
    let (m, b) = firmware(&rig, 1);
    let mut manifest = FirmwareManifest::decode(m.as_slice()).unwrap();
    manifest.signatures.truncate(1);
    let weak = manifest.encode_to_vec();

    let err = c
        .install_firmware(&rig.trust, &weak, &b, &mut |_, _| {})
        .unwrap_err();
    assert!(matches!(err, ClientError::Verification(_)), "{err:?}");
    assert_eq!(
        rig.hal.confirms_requested(),
        0,
        "the user was never even prompted"
    );

    // A malicious Suite that skips its own checks is still stopped by the bootloader.
    let r = c.request(Request::FirmwareBegin(FirmwareBegin { manifest: weak }));
    assert_eq!(
        device_code(r.unwrap_err()),
        ErrorCode::InsufficientSignatures
    );
    assert_eq!(c.features().unwrap().state, "virgin");

    // Corrupted blob: caught by the Suite before sending anything.
    let mut bad = b.clone();
    bad[10] ^= 1;
    assert!(matches!(
        c.install_firmware(&rig.trust, &m, &bad, &mut |_, _| {}),
        Err(ClientError::Verification(_))
    ));
    let _ = fs::remove_dir_all(rig.issuer.dir());
}

#[test]
fn device_from_an_unknown_authority_fails_attestation() {
    let rig = start();
    let other = Issuer::open(scratch_dir("other-authority"));
    let foreign_trust = other.dev_setup().unwrap();
    let mut c = connect(&rig);
    let rep = c.verify_authenticity(&foreign_trust).unwrap();
    assert!(!rep.genuine);
    assert!(rep.reason.is_some());
    let _ = fs::remove_dir_all(other.dir());
    let _ = fs::remove_dir_all(rig.issuer.dir());
}

#[test]
fn rejecting_on_the_device_prevents_any_signature() {
    let rig = start();
    let mut c = connect(&rig);
    onboard(&rig, &mut c);
    rig.hal.push_decision(Decision::Reject);
    assert_eq!(
        device_code(c.sign_document(b"doc", "x").unwrap_err()),
        ErrorCode::UserRejected
    );
    // and approving works again afterwards
    assert!(c.sign_document(b"doc", "x").is_ok());
    let _ = fs::remove_dir_all(rig.issuer.dir());
}

#[test]
fn eight_wrong_pins_wipe_the_device_over_the_wire() {
    let rig = start();
    let mut c = connect(&rig);
    onboard(&rig, &mut c);
    drop(c);
    let mut c = connect(&rig);
    rig.hal.set_default_pin(Some("135792"));
    for i in 1..8u32 {
        rig.hal.advance_ms(10_000_000);
        assert_eq!(
            c.unlock().unwrap(),
            UnlockOutcome::WrongPin { remaining: 8 - i }
        );
    }
    rig.hal.advance_ms(10_000_000);
    assert_eq!(device_code(c.unlock().unwrap_err()), ErrorCode::DeviceWiped);
    assert_eq!(c.features().unwrap().state, "firmware_installed");
    let _ = fs::remove_dir_all(rig.issuer.dir());
}

// ───────────────────────── elections over the wire ─────────────────────────

fn election_bundle(rig: &Rig) -> Vec<u8> {
    fs::read(rig.issuer.dir().join("election.bundle")).unwrap()
}

#[test]
fn election_flow_president_for_all_governor_and_mayor_by_domicile() {
    let rig = start();
    let mut c = connect(&rig);
    onboard(&rig, &mut c); // the voter lives in SP / São Paulo (3550308)

    // Suite verifies governance first; the device verifies again and asks on ITS screen.
    let e = c.load_election(&rig.trust, &election_bundle(&rig)).unwrap();
    let eid = e.election_id.clone();
    assert!(eid.starts_with("eleicoes-exemplo-"));
    assert_eq!(e.contests.len(), 7);
    let get = |id: &str| e.contests.iter().find(|x| x.contest_id == id).unwrap();
    // Different contests, different situations at the same moment, decided by the DEVICE:
    assert_eq!(
        (get("presidente").phase, get("presidente").eligible),
        ("voting_open", true)
    );
    assert_eq!(
        (get("governador-sp").phase, get("governador-sp").eligible),
        ("voting_open", true)
    );
    assert_eq!(
        (get("governador-rj").phase, get("governador-rj").eligible),
        ("voting_open", false)
    );
    assert_eq!(
        (get("governador-mg").phase, get("governador-mg").eligible),
        ("voting_open", false)
    );
    assert_eq!(
        (
            get("prefeito-3550308").phase,
            get("prefeito-3550308").eligible
        ),
        ("candidates_published", true)
    );
    assert_eq!(get("governador-sp").option_names.len(), 12);
    let shown = rig
        .hal
        .screens()
        .into_iter()
        .rev()
        .find(|s| s.title == "CARREGAR ELEIÇÃO?")
        .unwrap();
    assert!(shown.lines.join(" ").contains("Governança: 2 de 3"));

    // The President belongs to everyone: vote, verify, see the NAME on the device screen.
    let ballot = c.sign_ballot(&eid, "presidente", 1).unwrap();
    assert_eq!(ballot.option_name, "Candidato Boto-Cor-de-Rosa");
    let screen = rig
        .hal
        .screens()
        .into_iter()
        .rev()
        .find(|s| s.title == "CONFIRMAR VOTO?")
        .unwrap();
    let text = screen.lines.join(" ");
    assert!(
        text.contains("Candidato Boto-Cor-de-Rosa") && text.contains("Presidente da República"),
        "{text}"
    );
    assert!(
        !ballot.signature.is_empty()
            && !ballot.voter_key.is_empty()
            && ballot.contest_hash.len() == 64
    );

    // São Paulo's governor is MY contest and open: vote there too (a different key).
    let gov = c.sign_ballot(&eid, "governador-sp", 3).unwrap();
    assert_eq!(gov.option_name, "Candidata Arara-Canindé");
    assert_ne!(
        gov.voter_key, ballot.voter_key,
        "keys are per contest: the two votes cannot be linked"
    );

    // Other states' governors: voting is open but it is not my state -> refused, no prompt.
    let before = rig.hal.confirms_requested();
    for other in [
        "governador-rj",
        "governador-mg",
        "governador-ba",
        "governador-rs",
    ] {
        assert_eq!(
            device_code(c.sign_ballot(&eid, other, 0).unwrap_err()),
            ErrorCode::NotEligible,
            "{other}"
        );
    }
    // The mayor: mine, but only the candidates are announced.
    assert_eq!(
        device_code(c.sign_ballot(&eid, "prefeito-3550308", 0).unwrap_err()),
        ErrorCode::WrongPhase
    );
    assert_eq!(
        rig.hal.confirms_requested(),
        before,
        "refused requests never reach the user"
    );

    // One ballot per contest.
    assert_eq!(
        device_code(c.sign_ballot(&eid, "presidente", 0).unwrap_err()),
        ErrorCode::AlreadyVoted
    );
    let after = c.election().unwrap();
    let voted: Vec<&str> = after
        .contests
        .iter()
        .filter(|x| x.voted)
        .map(|x| x.contest_id.as_str())
        .collect();
    assert_eq!(voted, ["presidente", "governador-sp"]);

    // Registration keys are per contest, unrelated, and the registered key is the one that signs.
    let k_pres = c.voter_key(&eid, "presidente").unwrap();
    assert_eq!(k_pres, ballot.voter_key);
    assert_eq!(
        device_code(c.voter_key(&eid, "prefeito-3550308").unwrap_err()),
        ErrorCode::WrongPhase
    );

    // What the device signed is counted by the PUBLIC tally, once, from the board alone.
    let admin = election_admin::Admin::open(rig.issuer.dir()).unwrap();
    admin.publish().unwrap();

    // ANONYMOUS REGISTRATION (ADR-0008): the device blinds its own voter key, the
    // authority checks WHO (identity + domicile + phase) and blind-signs, the device
    // unblinds, and `(voter_key, token)` goes to the board on its own channel.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    for contest in ["presidente", "governador-sp"] {
        let req = c.begin_registration(&eid, contest).unwrap();
        let sig = rig
            .issuer
            .sign_registration(&eid, contest, &req, now)
            .unwrap();
        let token = c.finish_registration(&eid, contest, &sig).unwrap();
        // The authority saw the citizen and a blinded value: never the voter key.
        let key = teko_crypto::hex::decode(&c.voter_key(&eid, contest).unwrap()).unwrap();
        assert_eq!(token.voter_key, key);
        assert!(!req.blind_message.windows(key.len()).any(|w| w == &key[..]));
        let ledger = fs::read_to_string(rig.issuer.dir().join("registrations.log")).unwrap();
        assert!(ledger.contains(contest));
        assert!(
            !ledger.contains(&teko_crypto::hex::encode(&key)),
            "the authority's ledger never holds a voter key"
        );
        // Asking again (lost reply) is answered; the SAME blinded message is re-signed.
        let again = c.begin_registration(&eid, contest).unwrap();
        assert_eq!(again.blind_message, req.blind_message);
        assert!(
            rig.issuer
                .sign_registration(&eid, contest, &again, now)
                .is_ok()
        );
        admin
            .submit_registration(&teko_election::SubmittedRegistration {
                election_id: eid.clone(),
                contest_id: contest.to_string(),
                voter_key: token.voter_key,
                token: token.token,
            })
            .unwrap();
    }
    // The authority refuses what the device would also refuse, and tampered requests.
    let mut forged = c.begin_registration(&eid, "presidente").unwrap();
    forged.blind_message[3] ^= 1; // the host swaps the blinded message
    assert!(
        rig.issuer
            .sign_registration(&eid, "presidente", &forged, now)
            .is_err()
    );
    let req = c.begin_registration(&eid, "presidente").unwrap();
    assert!(
        rig.issuer
            .sign_registration(&eid, "governador-rj", &req, now)
            .is_err(),
        "request is bound to its contest (and RJ is not my state)"
    );
    assert!(
        rig.issuer
            .sign_registration(&eid, "prefeito-3550308", &req, now)
            .is_err(),
        "bound to its contest; the mayor's registration is not open yet"
    );
    for b in [&ballot, &gov] {
        admin
            .submit_ballot(&teko_election::SubmittedBallot {
                election_id: b.election_id.clone(),
                contest_id: b.contest_id.clone(),
                contest_hash: teko_crypto::hex::decode(&b.contest_hash).unwrap(),
                option_index: b.option_index,
                voter_key: teko_crypto::hex::decode(&b.voter_key).unwrap(),
                signature: teko_crypto::hex::decode(&b.signature).unwrap(),
                encrypted: Vec::new(),
            })
            .unwrap();
    }
    let t = admin.tally().unwrap();
    assert_eq!(
        (
            t.contest("presidente").unwrap().accepted,
            t.contest("presidente").unwrap().options[1].votes
        ),
        (1, 1)
    );
    assert_eq!(t.contest("governador-sp").unwrap().options[3].votes, 1);
    assert_eq!(
        t.contests.iter().map(|x| x.rejected.total()).sum::<u64>(),
        0,
        "real device ballots are valid"
    );
    let _ = fs::remove_dir_all(rig.issuer.dir());
}

#[test]
fn a_forged_or_old_election_never_reaches_a_vote() {
    let rig = start();
    let mut c = connect(&rig);
    onboard(&rig, &mut c);
    let bundle = election_bundle(&rig);

    // Governance signatures stripped down to one institution: the Suite refuses first...
    let mut b = teko_election::ElectionBundle::decode(bundle.as_slice()).unwrap();
    let mut m = teko_election::ElectionManifest::decode(b.manifest.as_slice()).unwrap();
    m.signatures.truncate(1);
    b.manifest = m.encode_to_vec();
    let err = c.load_election(&rig.trust, &b.encode_to_vec()).unwrap_err();
    assert!(matches!(err, ClientError::Verification(_)), "{err:?}");
    // ...and a Suite that skips its own check is still stopped by the device.
    let raw = c.request(teko_protocol::Request::LoadElection(
        teko_protocol::LoadElection {
            manifest: b.manifest.clone(),
            transitions: b.transitions.clone(),
        },
    ));
    assert_eq!(
        device_code(raw.unwrap_err()),
        ErrorCode::InsufficientSignatures
    );
    assert_eq!(
        device_code(c.election().unwrap_err()),
        ErrorCode::InvalidState,
        "nothing was stored"
    );

    // Voting without a loaded election is impossible.
    assert!(matches!(
        c.sign_ballot("any-election", "presidente", 0),
        Err(ClientError::Unexpected(_))
    ));
    let _ = fs::remove_dir_all(rig.issuer.dir());
}

#[test]
fn losing_the_device_means_reissue_the_old_credential_is_revoked_and_the_citizen_stays_one() {
    use teko_crypto::credential::RevocationList;
    let rig = start();
    let mut c = connect(&rig);
    onboard(&rig, &mut c);
    let old_civic = c.civic_id().unwrap().id;
    let e = c.load_election(&rig.trust, &election_bundle(&rig)).unwrap();
    let eid = e.election_id.clone();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let attr = |c: &mut DeviceClient<TcpTransport>| {
        c.prove_attribute(
            &rig.trust,
            teko_protocol::Predicate {
                kind: teko_protocol::PredicateKind::AgeAtLeast as i32,
                arg: 18,
                name: String::new(),
            },
            "Loja",
        )
    };
    assert!(attr(&mut c).is_ok(), "valid before any revocation");

    // The citizen registers for the President with device A.
    let req = c.begin_registration(&eid, "presidente").unwrap();
    let sig = rig
        .issuer
        .sign_registration(&eid, "presidente", &req, now)
        .unwrap();
    c.finish_registration(&eid, "presidente", &sig).unwrap();

    // Device A is LOST: the citizen asks the authority to block it at once.
    rig.issuer.revoke_civic_id(&old_civic, "lost", now).unwrap();
    let blocked = RevocationList::decode(
        fs::read(rig.issuer.dir().join("revocations.signed"))
            .unwrap()
            .as_slice(),
    )
    .unwrap();
    c.use_revocations(&rig.trust, blocked).unwrap();
    let err = attr(&mut c).unwrap_err();
    assert!(
        err.to_string().contains("REVOGADA"),
        "the lost device's proof is refused: {err}"
    );
    // Then she goes in person with a new device.
    c.wipe().unwrap(); // the new/clean device (same emulator here, stands for a new one)
    c.setup_pin().unwrap();
    let wallet = c.wallet_setup().unwrap();
    assert!(wallet.address.starts_with("tekow1"));
    let nonce = Issuer::new_nonce();
    let new_req = c.identity_request(&nonce, "Autoridade Cívica").unwrap();
    let subject = Subject {
        name: "Maria da Silva".into(),
        birth_date: "1990-05-17".into(),
        nationality: "BR".into(),
        electoral_state: "SP".into(),
        electoral_municipality: "3550308".into(),
    };
    // A plain first issuance is refused: she already has a record.
    let refused = rig.issuer.issue_credential(&nonce, &new_req, &subject);
    assert!(refused.unwrap_err().to_string().contains("REEMISSÃO"));
    // The in-person reissue works and REVOKES the old credential.
    let issued = rig
        .issuer
        .reissue_credential_at(&nonce, &new_req, &subject, &old_civic, "lost", now)
        .unwrap();
    c.load_credential(&issued.encode_to_vec()).unwrap();
    let new_civic = c.civic_id().unwrap().id;
    assert_ne!(new_civic, old_civic, "a new device has a new identity key");

    // Verifiers holding the signed list reject the OLD credential id and accept the new one.
    let list = RevocationList::decode(
        fs::read(rig.issuer.dir().join("revocations.signed"))
            .unwrap()
            .as_slice(),
    )
    .unwrap();
    let old_id = rig.issuer.revocation_list().unwrap().unwrap().revoked[0].clone();
    assert!(list.is_revoked(&old_id));
    c.use_revocations(&rig.trust, list.clone()).unwrap();
    assert!(
        attr(&mut c).is_ok(),
        "the NEW credential verifies against the list"
    );
    // A stale (lower seq) or forged list is refused.
    let mut forged = list.clone();
    forged.revoked.clear();
    assert!(c.use_revocations(&rig.trust, forged).is_err());

    // Same CITIZEN: a new device cannot register again for the contest she already registered.
    c.load_election(&rig.trust, &election_bundle(&rig)).unwrap();
    let req2 = c.begin_registration(&eid, "presidente").unwrap();
    let err = rig
        .issuer
        .sign_registration(&eid, "presidente", &req2, now)
        .unwrap_err();
    assert!(err.to_string().contains("já se registrou"), "{err}");
    // ...but a contest she had NOT registered for is fine.
    let req3 = c.begin_registration(&eid, "governador-sp").unwrap();
    assert!(
        rig.issuer
            .sign_registration(&eid, "governador-sp", &req3, now)
            .is_ok()
    );
    let _ = fs::remove_dir_all(rig.issuer.dir());
}

#[test]
fn an_encrypted_election_the_device_encrypts_the_vote_and_the_public_tally_cannot_see_it() {
    use p256::elliptic_curve::Field;
    use teko_ballot_crypto::{Point, Scalar, point_to_bytes};
    let rig = start();
    // The election carries a key (here a test key: the ceremony itself is tested elsewhere).
    let x = Scalar::random(&mut rand::rngs::StdRng::seed_from_u64(5));
    let net = rig.issuer.dir().join("net");
    fs::create_dir_all(&net).unwrap();
    fs::write(
        net.join("tally-key.pub"),
        teko_crypto::hex::encode(&point_to_bytes(&(Point::GENERATOR * x))),
    )
    .unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    rig.issuer.dev_election(now).unwrap();

    let mut c = connect(&rig);
    onboard(&rig, &mut c);
    let e = c.load_election(&rig.trust, &election_bundle(&rig)).unwrap();
    let eid = e.election_id.clone();

    // Register (anonymously), then vote: the DEVICE encrypts; the client verifies the proofs.
    let req = c.begin_registration(&eid, "presidente").unwrap();
    let sig = rig
        .issuer
        .sign_registration(&eid, "presidente", &req, now)
        .unwrap();
    let token = c.finish_registration(&eid, "presidente", &sig).unwrap();
    let ballot = c.sign_ballot(&eid, "presidente", 1).unwrap();
    assert!(
        !ballot.encrypted.is_empty(),
        "an encrypted ballot came back"
    );
    let screen = rig
        .hal
        .screens()
        .into_iter()
        .rev()
        .find(|s| s.title == "CONFIRMAR VOTO?")
        .unwrap();
    assert!(screen.lines.join(" ").contains("CIFRADO"));

    // Publish through the public path: the tally counts it but cannot see the choice.
    let admin = election_admin::Admin::open(rig.issuer.dir()).unwrap();
    admin.publish().unwrap();
    admin
        .submit_registration(&teko_election::SubmittedRegistration {
            election_id: eid.clone(),
            contest_id: "presidente".into(),
            voter_key: token.voter_key,
            token: token.token,
        })
        .unwrap();
    admin
        .submit_ballot(&teko_election::SubmittedBallot {
            election_id: ballot.election_id.clone(),
            contest_id: ballot.contest_id.clone(),
            contest_hash: teko_crypto::hex::decode(&ballot.contest_hash).unwrap(),
            option_index: 0,
            voter_key: teko_crypto::hex::decode(&ballot.voter_key).unwrap(),
            signature: teko_crypto::hex::decode(&ballot.signature).unwrap(),
            encrypted: teko_crypto::hex::decode(&ballot.encrypted).unwrap(),
        })
        .unwrap();
    let t = admin.tally().unwrap();
    let p = t.contest("presidente").unwrap();
    assert_eq!(
        (p.accepted, p.rejected.total(), p.encrypted, p.revealed),
        (1, 0, true, false)
    );
    assert!(p.options.iter().all(|o| o.votes == 0));
    let _ = fs::remove_dir_all(rig.issuer.dir());
}

#[test]
fn identity_is_blocked_until_the_wallet_backup_is_confirmed() {
    let rig = start();
    let mut c = connect(&rig);
    let (m, b) = firmware(&rig, 1);
    c.install_firmware(&rig.trust, &m, &b, &mut |_, _| {})
        .unwrap();
    c.setup_pin().unwrap();
    assert!(!c.initialize().unwrap().wallet_ready);
    assert_eq!(
        device_code(c.civic_id().unwrap_err()),
        ErrorCode::InvalidState
    );
    c.wallet_setup().unwrap();
    assert!(c.initialize().unwrap().wallet_ready);
    assert!(c.civic_id().is_ok());
}

#[test]
fn a_wallet_restores_on_a_new_device_with_the_same_address() {
    // Device A creates the wallet; only its SCREEN ever showed the words.
    let a = start();
    let mut ca = connect(&a);
    let (m, b) = firmware(&a, 1);
    ca.install_firmware(&a.trust, &m, &b, &mut |_, _| {})
        .unwrap();
    ca.setup_pin().unwrap();
    let created = ca.wallet_setup().unwrap();
    let info = ca.wallet_address().unwrap();
    assert_eq!(info.address, created.address);
    let words = a.hal.shown_words();
    assert_eq!(words.len(), 24);

    // Device B is brand new (lost device): restore by typing the words on it.
    let bdev = start();
    let mut cb = connect(&bdev);
    let (m, b) = firmware(&bdev, 1);
    cb.install_firmware(&bdev.trust, &m, &b, &mut |_, _| {})
        .unwrap();
    cb.setup_pin().unwrap();
    for w in &words {
        bdev.hal.push_word(Some(w));
    }
    let restored = cb.wallet_restore().unwrap();
    assert_eq!(restored.address, created.address);
    assert_eq!(cb.wallet_address().unwrap().public_key, info.public_key);

    // "Check backup": retype the phrase on the device.
    for w in &words {
        bdev.hal.push_word(Some(w));
    }
    cb.wallet_check_backup().unwrap();
    let mut wrong = words.clone();
    wrong.swap(3, 7);
    wrong[0] = "abandon".into();
    for w in &wrong {
        bdev.hal.push_word(Some(w));
    }
    assert!(cb.wallet_check_backup().is_err());
}

#[test]
fn sending_coins_is_signed_on_the_device_and_lands_in_the_devnet_economy() {
    use teko_coin::devnet::{CHAIN_ID, ECONOMY_FILE, Economy, append_allocation};

    let rig = start();
    let mut c = connect(&rig);
    let (m, b) = firmware(&rig, 1);
    c.install_firmware(&rig.trust, &m, &b, &mut |_, _| {})
        .unwrap();
    c.setup_pin().unwrap();
    let wallet = c.wallet_setup().unwrap();
    let me = c.wallet_address().unwrap();
    let pk = teko_crypto::hex::decode(&me.public_key).unwrap();
    let mine = teko_crypto::ledger_address(&pk);

    // test coins for this wallet, and somebody to pay
    let dir = scratch_dir("e2e-econ");
    let friend = teko_crypto::ledger_address(&[9u8; 33]);
    let friend_addr = teko_crypto::wallet_address(&friend).unwrap();
    append_allocation(&dir.join(ECONOMY_FILE), &wallet.address, 10_000).unwrap();
    let mut econ = Economy::open(&dir).unwrap();
    assert_eq!(econ.balance(&mine), 10_000);

    // the device shows the transfer and signs; the Suite submits
    let nonce = econ.nonce(&mine);
    let fee = econ.estimate_fee(&pk, nonce, &friend, 2500);
    let prompts = Arc::new(AtomicUsize::new(0));
    let p2 = prompts.clone();
    c.set_on_button(move |k| {
        if k == teko_protocol::ButtonKind::SignTransfer {
            p2.fetch_add(1, Ordering::SeqCst);
        }
    });
    let tx = c
        .wallet_sign_transfer(CHAIN_ID, &friend_addr, 2500, fee, nonce)
        .unwrap();
    assert_eq!(prompts.load(Ordering::SeqCst), 1);
    assert!(rig.hal.screens().iter().any(|s| {
        s.title == "ENVIAR MOEDAS" && s.lines.iter().any(|l| l.contains("Valor: 2500"))
    }));
    econ.submit(tx).unwrap();
    assert_eq!(econ.balance(&friend), 2500);
    assert_eq!(econ.balance(&mine), 10_000 - 2500 - fee);
    // restart the economy: the signed transaction was stored and replays
    let again = Economy::open(&dir).unwrap();
    assert_eq!(again.balance(&friend), 2500);
    assert_eq!(again.history(&mine).len(), 1);

    // the user rejects on the device: no signature, nothing changes
    rig.hal.push_decision(Decision::Reject);
    let nonce = again.nonce(&mine);
    let r = c.wallet_sign_transfer(CHAIN_ID, &friend_addr, 1, fee, nonce);
    assert_eq!(device_code(r.unwrap_err()), ErrorCode::UserRejected);
    assert_eq!(Economy::open(&dir).unwrap().balance(&friend), 2500);
}

#[test]
fn an_asset_consent_is_signed_on_the_device_and_verified_by_the_client() {
    let rig = start();
    let mut c = connect(&rig);
    let (m, b) = firmware(&rig, 1);
    c.install_firmware(&rig.trust, &m, &b, &mut |_, _| {})
        .unwrap();
    c.setup_pin().unwrap();
    let me = c.wallet_setup().unwrap().address;
    let buyer = teko_crypto::wallet_address(&[4u8; 32]).unwrap();
    let prompts = Arc::new(AtomicUsize::new(0));
    let p2 = prompts.clone();
    c.set_on_button(move |k| {
        if k == teko_protocol::ButtonKind::SignAssetConsent {
            p2.fetch_add(1, Ordering::SeqCst);
        }
    });
    let a = c
        .wallet_sign_asset_consent(
            "teko-devnet",
            3,
            "imovel",
            "matricula-9",
            &me,
            &buyer,
            750,
            2,
        )
        .unwrap();
    assert_eq!(prompts.load(Ordering::SeqCst), 1);
    assert!(rig.hal.screens().iter().any(|s| {
        s.title == "VENDA DE BEM" && s.lines.iter().any(|l| l.contains("Preço: 750"))
    }));
    assert_eq!(
        a.key,
        c.wallet_address()
            .map(|x| teko_crypto::hex::decode(&x.public_key).unwrap())
            .unwrap()
    );
    // the user refuses on the device: no consent is produced
    rig.hal.push_decision(Decision::Reject);
    let r = c.wallet_sign_asset_consent(
        "teko-devnet",
        3,
        "imovel",
        "matricula-9",
        &me,
        &buyer,
        750,
        2,
    );
    assert_eq!(device_code(r.unwrap_err()), ErrorCode::UserRejected);
}

#[test]
fn selling_and_buying_an_asset_with_my_device_consent_in_the_devnet_world() {
    use teko_assets::devnet::{self, CLASS};
    use teko_coin::devnet::{CHAIN_ID, ECONOMY_FILE, append_allocation};

    let rig = start();
    let mut c = connect(&rig);
    let (m, b) = firmware(&rig, 1);
    c.install_firmware(&rig.trust, &m, &b, &mut |_, _| {})
        .unwrap();
    c.setup_pin().unwrap();
    c.wallet_setup().unwrap();
    let me_info = c.wallet_address().unwrap();
    let pk = teko_crypto::hex::decode(&me_info.public_key).unwrap();
    let me = teko_crypto::ledger_address(&pk);

    let dir = scratch_dir("e2e-assets");
    let (_, cast) = devnet::open(&dir).unwrap();
    let cp = cast.counterparty_address();
    append_allocation(&dir.join(ECONOMY_FILE), &me_info.address, 1_000).unwrap();
    append_allocation(
        &dir.join(ECONOMY_FILE),
        &teko_crypto::wallet_address(&cp).unwrap(),
        8_000,
    )
    .unwrap();
    let (mut econ, cast) = devnet::open(&dir).unwrap();
    devnet::register(&mut econ, &cast, &me, "casa-7").unwrap();

    // The consent goes through MY DEVICE (screen + physical confirmation).
    let consents = Arc::new(AtomicUsize::new(0));
    let cc = consents.clone();
    c.set_on_button(move |k| {
        if k == teko_protocol::ButtonKind::SignAssetConsent {
            cc.fetch_add(1, Ordering::SeqCst);
        }
    });
    let mut device_consent = |kind: u8,
                              id: &str,
                              seller: &teko_ledger::Address,
                              buyer: &teko_ledger::Address,
                              price: u64,
                              version: u64| {
        let s = teko_crypto::wallet_address(seller).unwrap();
        let b = teko_crypto::wallet_address(buyer).unwrap();
        c.wallet_sign_asset_consent(CHAIN_ID, kind, CLASS, id, &s, &b, price, version)
            .map_err(|e| e.to_string())
    };
    let after = devnet::trade_with_counterparty(
        &mut econ,
        &cast,
        &me,
        "casa-7",
        3_000,
        true,
        &mut device_consent,
    )
    .unwrap();
    assert!(after > 3_000 && after <= 4_000, "{after}"); // 1000 + 3000 - nothing else
    assert!(devnet::owned(&econ, &me).is_empty());
    assert_eq!(consents.load(Ordering::SeqCst), 1);
    assert!(rig.hal.screens().iter().any(|s| {
        s.title == "VENDA DE BEM"
            && s.lines.iter().any(|l| l.contains("VENDEDOR"))
            && s.lines.iter().any(|l| l.contains("Preço: 3000"))
    }));

    // Buying it back: my device signs as the BUYER.
    let spent = devnet::trade_with_counterparty(
        &mut econ,
        &cast,
        &me,
        "casa-7",
        2_000,
        false,
        &mut device_consent,
    )
    .unwrap();
    assert_eq!(spent, after - 2_000);
    assert_eq!(devnet::owned(&econ, &me).len(), 1);
    assert_eq!(consents.load(Ordering::SeqCst), 2);

    // The user rejects on the device: nothing moves.
    rig.hal.push_decision(Decision::Reject);
    let r = devnet::trade_with_counterparty(
        &mut econ,
        &cast,
        &me,
        "casa-7",
        500,
        true,
        &mut device_consent,
    );
    assert!(r.is_err());
    assert_eq!(devnet::owned(&econ, &me).len(), 1);
    teko_coin::audit(econ.ledger()).unwrap();
}
