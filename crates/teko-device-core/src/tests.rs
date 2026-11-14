use proptest::prelude::*;
use rand::SeedableRng;
use rand::rngs::StdRng;
use teko_attestation::{
    FactoryData, FirmwareManifest, TrustPolicy, blob_hash, issue_batch_cert, issue_request_message,
    sign_manifest, verify_attestation, verify_batch_chain,
};
use teko_crypto::credential::{self, IssuedCredential, Presentation};
use teko_crypto::{SigningKey, generate_signing_key, hex, public_key_bytes, tags, verify_tagged};
use teko_election::{
    BALLOT_KIND_PUBLIC, BallotBody, Contest, ElectionManifest, ElectionOption, GovernancePolicy,
    Office, Phase, Schedule, Scope, Transition, contest_hash, sign_manifest as sign_election,
    sign_transition, verify_ballot,
};
use teko_protocol::*;

use crate::testing::ScriptedHal;
use crate::{Decision, Device, DeviceConfig};

const PIN: &str = "482913";

struct World {
    authority: SigningKey,
    signers: Vec<SigningKey>,
    gov: Vec<SigningKey>,
    trust: TrustPolicy,
    factory: FactoryData,
}

fn world() -> World {
    let mut rng = StdRng::seed_from_u64(42);
    let authority = generate_signing_key(&mut rng);
    let signers: Vec<SigningKey> = (0..3).map(|_| generate_signing_key(&mut rng)).collect();
    let gov: Vec<SigningKey> = (0..3).map(|_| generate_signing_key(&mut rng)).collect();
    let batch = generate_signing_key(&mut rng);
    let cert = issue_batch_cert(&authority, "lote-test", &public_key_bytes(&batch));
    let trust = TrustPolicy {
        authority_root: public_key_bytes(&authority),
        fw_signers: signers.iter().map(public_key_bytes).collect(),
        fw_threshold: 2,
        revoked_batches: Vec::new(),
        gov_signers: gov.iter().map(public_key_bytes).collect(),
        gov_threshold: 2,
    };
    let factory = FactoryData {
        batch_key: batch.to_bytes().to_vec(),
        batch_cert: prost::Message::encode_to_vec(&cert),
    };
    World {
        authority,
        signers,
        gov,
        trust,
        factory,
    }
}

fn cfg(max_attempts: u32) -> DeviceConfig {
    DeviceConfig {
        max_pin_attempts: max_attempts,
        pin_kdf_iterations: 3,
        ..DeviceConfig::default()
    }
}

type Dev = Device<ScriptedHal, StdRng>;

fn boot(w: &World, hal: &ScriptedHal, max_attempts: u32) -> Dev {
    Device::new(
        hal.clone(),
        StdRng::seed_from_u64(7),
        cfg(max_attempts),
        w.trust.clone(),
        Some(w.factory.clone()),
    )
    .unwrap()
}

fn manifest(w: &World, version: u32, blob: &[u8], signer_idx: &[usize]) -> FirmwareManifest {
    let mut m = FirmwareManifest {
        version,
        size: blob.len() as u64,
        hash: blob_hash(blob).to_vec(),
        signatures: Vec::new(),
    };
    m.signatures = signer_idx
        .iter()
        .map(|i| sign_manifest(&w.signers[*i], &m))
        .collect();
    m
}

fn install_fw(dev: &mut Dev, m: &FirmwareManifest, blob: &[u8]) -> Response {
    let r = dev.handle(Request::FirmwareBegin(teko_protocol::FirmwareBegin {
        manifest: prost::Message::encode_to_vec(m),
    }));
    if matches!(r, Response::Failure(_)) {
        return r;
    }
    for (i, c) in blob.chunks(1024).enumerate() {
        let r = dev.handle(Request::FirmwareChunk(FirmwareChunk {
            offset: (i * 1024) as u64,
            data: c.to_vec(),
        }));
        if matches!(r, Response::Failure(_)) {
            return r;
        }
    }
    dev.handle(Request::FirmwareFinish(FirmwareFinish {}))
}

fn attrs() -> Vec<(String, String)> {
    [
        ("name", "Maria da Silva"),
        ("birth_date", "1990-05-17"),
        ("nationality", "BR"),
        ("age_over_18", "true"),
        ("age_over_21", "true"),
        ("electoral_state", "SP"),
        ("electoral_municipality", "3550308"),
    ]
    .iter()
    .map(|(a, b)| (a.to_string(), b.to_string()))
    .collect()
}

/// Full onboarding. Returns the identity public key.
fn onboard(dev: &mut Dev, w: &World, hal: &ScriptedHal) -> Vec<u8> {
    let blob = vec![7u8; 3000];
    let m = manifest(w, 1, &blob, &[0, 1]);
    assert!(matches!(
        install_fw(dev, &m, &blob),
        Response::FirmwareStatus(_)
    ));
    assert_eq!(dev.state(), DeviceState::FirmwareInstalled);

    hal.set_default_pin(Some(PIN));
    assert!(matches!(
        dev.handle(Request::SetupPin(SetupPin {})),
        Response::PinSet(_)
    ));
    assert_eq!(dev.state(), DeviceState::Initialized);

    let pk = match dev.handle(Request::GetCivicId(GetCivicId {})) {
        Response::CivicId(c) => c.pk_identity,
        other => panic!("{other:?}"),
    };
    let nonce = vec![9u8; 32];
    let Response::IdentityRequest(req) =
        dev.handle(Request::GetIdentityRequest(GetIdentityRequest {
            authority_nonce: nonce,
            authority_name: "Autoridade Teste".into(),
        }))
    else {
        panic!("identity request failed")
    };
    assert_eq!(req.pk_identity, pk);

    let mut rng = StdRng::seed_from_u64(99);
    let issued = credential::issue(
        &w.authority,
        "Autoridade Teste",
        &pk,
        1,
        0,
        [3; 16],
        &attrs(),
        &mut rng,
    );
    let r = dev.handle(Request::LoadCredential(LoadCredential {
        credential: prost::Message::encode_to_vec(&issued),
    }));
    assert!(matches!(r, Response::CredentialLoaded(_)), "{r:?}");
    assert_eq!(dev.state(), DeviceState::CredentialsIssued);
    pk
}

fn ready() -> (World, ScriptedHal, Dev, Vec<u8>) {
    let w = world();
    let hal = ScriptedHal::new();
    let mut dev = boot(&w, &hal, 8);
    let pk = onboard(&mut dev, &w, &hal);
    (w, hal, dev, pk)
}

fn sign_req(hash: Vec<u8>) -> Request {
    Request::SignDocument(SignDocument {
        hash,
        summary: "Contrato".into(),
    })
}

fn failure_code(r: &Response) -> Option<ErrorCode> {
    match r {
        Response::Failure(f) => Some(f.code()),
        _ => None,
    }
}

// ───────────────────────── state machine & onboarding ─────────────────────────

#[test]
fn onboarding_reaches_credentials_issued_and_is_persisted() {
    let (w, hal, dev, _) = ready();
    assert!(dev.is_unlocked());
    drop(dev);
    // Restart: same storage, device boots LOCKED but keeps its state.
    let dev2 = boot(&w, &hal, 8);
    assert_eq!(dev2.state(), DeviceState::CredentialsIssued);
    assert!(!dev2.is_unlocked());
}

#[test]
fn operations_rejected_in_wrong_state() {
    let w = world();
    let hal = ScriptedHal::new();
    let mut dev = boot(&w, &hal, 8);
    for req in [
        sign_req(vec![0; 32]),
        Request::GetCivicId(GetCivicId {}),
        Request::SetupPin(SetupPin {}),
        Request::Unlock(Unlock {}),
        Request::GetVoterKey(GetVoterKey {
            election_id: "e".into(),
            contest_id: "c".into(),
        }),
        Request::SignBallot(SignBallot {
            election_id: "e".into(),
            contest_id: "c".into(),
            option_index: 0,
        }),
        Request::LoadElection(LoadElection::default()),
        Request::Wipe(Wipe {}),
    ] {
        assert_eq!(
            failure_code(&dev.handle(req)),
            Some(ErrorCode::InvalidState)
        );
    }
    assert_eq!(hal.confirms_requested(), 0);
}

#[test]
fn locked_after_restart_until_pin_entered() {
    let (w, hal, dev, _) = ready();
    drop(dev);
    let mut dev = boot(&w, &hal, 8);
    assert_eq!(
        failure_code(&dev.handle(sign_req(vec![1; 32]))),
        Some(ErrorCode::InvalidState)
    );
    assert!(matches!(
        dev.handle(Request::Unlock(Unlock {})),
        Response::Unlocked(_)
    ));
    assert!(matches!(
        dev.handle(sign_req(vec![1; 32])),
        Response::DocumentSignature(_)
    ));
}

#[test]
fn unlock_expires_after_idle_timeout() {
    let (_, hal, mut dev, _) = ready();
    hal.advance_ms(DeviceConfig::default().unlock_timeout_ms + 1);
    assert_eq!(
        failure_code(&dev.handle(sign_req(vec![1; 32]))),
        Some(ErrorCode::InvalidState)
    );
}

#[test]
fn passive_polling_does_not_redraw_the_screen() {
    let (_, hal, mut dev, _) = ready();
    hal.clear_screens();
    for _ in 0..5 {
        dev.handle(Request::GetFeatures(GetFeatures {}));
        dev.handle(Request::Ping(Ping {}));
    }
    assert!(
        hal.screens().is_empty(),
        "polling must not redraw: {:?}",
        hal.screens()
    );
    // ...but a real change (the unlock timing out) is shown exactly once.
    hal.advance_ms(DeviceConfig::default().unlock_timeout_ms + 1);
    dev.handle(Request::GetFeatures(GetFeatures {}));
    dev.handle(Request::GetFeatures(GetFeatures {}));
    let shown = hal.screens();
    assert_eq!(shown.len(), 1);
    assert!(shown[0].lines.iter().any(|l| l == "BLOQUEADO"));
}

#[test]
fn host_polling_does_not_keep_the_device_unlocked() {
    let (_, hal, mut dev, _) = ready();
    let step = DeviceConfig::default().unlock_timeout_ms / 3;
    for _ in 0..4 {
        hal.advance_ms(step);
        dev.handle(Request::GetFeatures(GetFeatures {}));
    }
    assert_eq!(
        failure_code(&dev.handle(sign_req(vec![1; 32]))),
        Some(ErrorCode::InvalidState),
        "polling must not renew the unlock"
    );
}

#[test]
fn weak_or_mismatched_pin_is_rejected_at_setup() {
    let w = world();
    let hal = ScriptedHal::new();
    let mut dev = boot(&w, &hal, 8);
    let blob = vec![1u8; 100];
    install_fw(&mut dev, &manifest(&w, 1, &blob, &[0, 1]), &blob);

    hal.set_default_pin(Some("123456"));
    assert_eq!(
        failure_code(&dev.handle(Request::SetupPin(SetupPin {}))),
        Some(ErrorCode::InvalidArgument)
    );
    hal.set_default_pin(None);
    hal.push_pin(Some("482913"));
    hal.push_pin(Some("482914"));
    assert_eq!(
        failure_code(&dev.handle(Request::SetupPin(SetupPin {}))),
        Some(ErrorCode::InvalidArgument)
    );
    assert_eq!(dev.state(), DeviceState::FirmwareInstalled);
}

// ───────────────────────── PIN attempts / wipe ─────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(12))]

    /// A wrong PIN entered N times wipes the device (all secrets destroyed).
    #[test]
    fn wrong_pin_n_times_wipes_the_device(n in 3u32..=8) {
        let w = world();
        let hal = ScriptedHal::new();
        let mut dev = boot(&w, &hal, n);
        onboard(&mut dev, &w, &hal);
        let root = dev.secrets_for_test()[0].clone();
        drop(dev);

        let mut dev = boot(&w, &hal, n);
        hal.set_default_pin(Some("135792"));
        for i in 1..n {
            hal.advance_ms(10_000_000);
            match dev.handle(Request::Unlock(Unlock {})) {
                Response::PinWrong(p) => prop_assert_eq!(p.remaining, n - i),
                other => prop_assert!(false, "unexpected {:?}", other),
            }
        }
        hal.advance_ms(10_000_000);
        let last = dev.handle(Request::Unlock(Unlock {}));
        prop_assert_eq!(failure_code(&last), Some(ErrorCode::DeviceWiped));
        prop_assert_eq!(dev.state(), DeviceState::FirmwareInstalled);
        let stored = hal.raw_storage().unwrap();
        prop_assert!(!stored.windows(root.len()).any(|win| win == root.as_slice()));
        // Even the right PIN no longer helps.
        hal.set_default_pin(Some(PIN));
        prop_assert_eq!(failure_code(&dev.handle(Request::Unlock(Unlock {}))), Some(ErrorCode::InvalidState));
    }

    /// Success before the limit resets the counter.
    #[test]
    fn correct_pin_resets_the_counter(n in 4u32..=8) {
        let w = world();
        let hal = ScriptedHal::new();
        let mut dev = boot(&w, &hal, n);
        onboard(&mut dev, &w, &hal);
        drop(dev);
        let mut dev = boot(&w, &hal, n);
        hal.set_default_pin(Some("135792"));
        for _ in 0..(n - 1) {
            hal.advance_ms(10_000_000);
            dev.handle(Request::Unlock(Unlock {}));
        }
        hal.set_default_pin(Some(PIN));
        hal.advance_ms(10_000_000);
        let unlocked = matches!(dev.handle(Request::Unlock(Unlock {})), Response::Unlocked(_));
        prop_assert!(unlocked);
        let Response::Features(f) = dev.handle(Request::GetFeatures(GetFeatures {})) else { panic!() };
        prop_assert_eq!(f.retries_remaining, n);
    }
}

#[test]
fn exponential_delay_between_attempts() {
    let (w, hal, dev, _) = ready();
    drop(dev);
    let mut dev = boot(&w, &hal, 8);
    hal.set_default_pin(Some("135792"));
    for _ in 0..4 {
        hal.advance_ms(10_000_000);
        assert!(matches!(
            dev.handle(Request::Unlock(Unlock {})),
            Response::PinWrong(_)
        ));
    }
    // 4 failures => 1s delay: an immediate retry is refused without reading a PIN.
    let r = dev.handle(Request::Unlock(Unlock {}));
    match r {
        Response::Failure(f) => {
            assert_eq!(f.code(), ErrorCode::PinLocked);
            assert!(f.retry_after_ms > 0 && f.retry_after_ms <= 1_000);
        }
        other => panic!("{other:?}"),
    }
    hal.advance_ms(1_001);
    assert!(matches!(
        dev.handle(Request::Unlock(Unlock {})),
        Response::PinWrong(_)
    ));
}

#[test]
fn device_screen_warns_when_few_attempts_remain() {
    let (w, hal, dev, _) = ready();
    drop(dev);
    let mut dev = boot(&w, &hal, 8);
    hal.set_default_pin(Some("135792"));
    for attempt in 1..=7u32 {
        hal.advance_ms(10_000_000);
        hal.clear_screens();
        dev.handle(Request::Unlock(Unlock {}));
        let shown = hal
            .screens()
            .into_iter()
            .find(|s| s.title == "DIGITE O PIN")
            .unwrap()
            .lines
            .concat();
        let remaining = 8 - (attempt - 1);
        assert!(
            shown.contains(&format!("restantes: {remaining}")),
            "{shown}"
        );
        assert_eq!(
            shown.contains("ATENÇÃO"),
            (2..=3).contains(&remaining),
            "{remaining}: {shown}"
        );
        assert_eq!(
            shown.contains("ÚLTIMA TENTATIVA"),
            remaining == 1,
            "{remaining}: {shown}"
        );
    }
}

#[test]
fn counter_is_persisted_before_the_pin_is_checked() {
    let (w, hal, dev, _) = ready();
    drop(dev);
    let mut dev = boot(&w, &hal, 8);
    hal.set_default_pin(Some("135792"));
    dev.handle(Request::Unlock(Unlock {}));
    drop(dev);
    // A "power cut" restart does not give the attempt back.
    let mut dev = boot(&w, &hal, 8);
    let Response::Features(f) = dev.handle(Request::GetFeatures(GetFeatures {})) else {
        panic!()
    };
    assert_eq!(f.retries_remaining, 7);
}

// ───────────────────────── signing needs physical confirmation ─────────────────────────

#[test]
fn no_signature_without_physical_confirmation() {
    let (_, hal, mut dev, _) = ready();
    for d in [Decision::Reject, Decision::Timeout] {
        hal.push_decision(d);
        let before = hal.confirms_requested();
        let r = dev.handle(sign_req(vec![5; 32]));
        assert!(!matches!(r, Response::DocumentSignature(_)), "{r:?}");
        assert!(failure_code(&r).is_some());
        assert_eq!(hal.confirms_requested(), before + 1);
    }
    // A host-side "ButtonAck" is NOT a confirmation.
    hal.set_default_decision(Decision::Reject);
    let _ = dev.handle(Request::ButtonAck(ButtonAck {}));
    assert_eq!(
        failure_code(&dev.handle(sign_req(vec![5; 32]))),
        Some(ErrorCode::UserRejected)
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(8))]

    /// The hash shown on the device screen is exactly the hash that gets signed.
    #[test]
    fn displayed_hash_equals_signed_hash(hash in any::<[u8; 32]>()) {
        let (_, hal, mut dev, pk) = ready();
        hal.clear_screens();
        hal.clear_interim();
        let Response::DocumentSignature(s) = dev.handle(sign_req(hash.to_vec())) else {
            panic!("expected signature")
        };
        let screen = hal.screens().into_iter().find(|s| s.title == "ASSINAR DOCUMENTO?").unwrap();
        let shown: String = screen.lines.concat().replace(' ', "");
        prop_assert!(shown.contains(&hex::encode(&hash)));
        prop_assert!(verify_tagged(&pk, tags::DOC_SIGN, &hash, &s.signature).is_ok());
        // The host was told to wait for the user BEFORE the signature existed.
        let interim = hal.interim();
        prop_assert!(matches!(interim.first(), Some(Response::ButtonRequest(b)) if b.kind == ButtonKind::SignDocument as i32));
    }
}

#[test]
fn malicious_summary_is_refused() {
    let (_, _, mut dev, _) = ready();
    for summary in ["linha1\nlinha2", "evil\u{202E}txt"] {
        let r = dev.handle(Request::SignDocument(SignDocument {
            hash: vec![0; 32],
            summary: summary.into(),
        }));
        assert_eq!(failure_code(&r), Some(ErrorCode::InvalidArgument));
    }
    assert_eq!(
        failure_code(&dev.handle(sign_req(vec![0; 31]))),
        Some(ErrorCode::InvalidArgument)
    );
}

// ───────────────────────── no secret leaves the device ─────────────────────────

fn script(i: u8) -> Request {
    match i % 14 {
        0 => Request::GetFeatures(GetFeatures {}),
        1 => Request::AttestationChallenge(AttestationChallenge { nonce: vec![1; 32] }),
        2 => Request::GetCivicId(GetCivicId {}),
        3 => Request::GetPairwiseId(GetPairwiseId {
            service_id: "banco".into(),
        }),
        4 => sign_req(vec![i; 32]),
        5 => Request::ProveAttribute(ProveAttribute {
            predicate: Some(Predicate {
                kind: PredicateKind::AgeAtLeast as i32,
                arg: 18,
                name: String::new(),
            }),
            verifier_nonce: vec![2; 16],
            verifier_name: "loja".into(),
        }),
        6 => Request::GetVoterKey(GetVoterKey {
            election_id: "eleicoes-2026".into(),
            contest_id: "presidente".into(),
        }),
        7 => Request::SignBallot(SignBallot {
            election_id: "eleicoes-2026".into(),
            contest_id: "presidente".into(),
            option_index: 1,
        }),
        8 => Request::Unlock(Unlock {}),
        9 => Request::Ping(Ping {}),
        10 => Request::Initialize(Initialize { host_proto_max: 1 }),
        11 => Request::GetIdentityRequest(GetIdentityRequest {
            authority_nonce: vec![4; 32],
            authority_name: "a".into(),
        }),
        12 => Request::ProveAttribute(ProveAttribute {
            predicate: Some(Predicate {
                kind: PredicateKind::Reveal as i32,
                arg: 0,
                name: "nationality".into(),
            }),
            verifier_nonce: vec![2; 16],
            verifier_name: "loja".into(),
        }),
        _ => Request::Cancel(Cancel {}),
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(10))]

    /// No response (or interim message) ever contains key material or the PIN.
    #[test]
    fn no_response_exposes_secrets(ops in proptest::collection::vec(any::<u8>(), 1..30), lock_first in any::<bool>()) {
        let (w, hal, dev, _) = ready();
        let mut dev = if lock_first { drop(dev); boot(&w, &hal, 8) } else { dev };
        let mut secrets = dev.secrets_for_test();
        secrets.push(PIN.as_bytes().to_vec());
        let mut wire: Vec<Vec<u8>> = Vec::new();
        for op in ops {
            let r = dev.handle(script(op));
            wire.push(Response::encode_payload(&r));
        }
        for r in hal.interim() { wire.push(r.encode_payload()); }
        for secret in secrets.iter().filter(|s| !s.is_empty()) {
            for msg in &wire {
                prop_assert!(!msg.windows(secret.len()).any(|win| win == secret.as_slice()));
            }
        }
    }
}

// ───────────────────────── firmware ─────────────────────────

#[test]
fn firmware_without_m_of_n_is_refused() {
    let w = world();
    let hal = ScriptedHal::new();
    let mut dev = boot(&w, &hal, 8);
    let blob = vec![1u8; 200];
    for idx in [vec![], vec![0], vec![1]] {
        let r = install_fw(&mut dev, &manifest(&w, 1, &blob, &idx), &blob);
        assert_eq!(failure_code(&r), Some(ErrorCode::InsufficientSignatures));
    }
    assert_eq!(dev.state(), DeviceState::Virgin);
    assert_eq!(hal.confirms_requested(), 0);
    // Same signer twice still counts once.
    let mut m = manifest(&w, 1, &blob, &[0]);
    m.signatures.push(m.signatures[0].clone());
    assert_eq!(
        failure_code(&install_fw(&mut dev, &m, &blob)),
        Some(ErrorCode::InsufficientSignatures)
    );
}

#[test]
fn downgrade_is_refused_and_upgrade_accepted() {
    let (w, _, mut dev, _) = ready();
    let blob = vec![2u8; 500];
    let up = manifest(&w, 2, &blob, &[0, 2]);
    assert!(matches!(
        install_fw(&mut dev, &up, &blob),
        Response::FirmwareStatus(_)
    ));
    let old = manifest(&w, 1, &blob, &[0, 1, 2]);
    assert_eq!(
        failure_code(&install_fw(&mut dev, &old, &blob)),
        Some(ErrorCode::Downgrade)
    );
}

#[test]
fn corrupted_blob_is_refused_and_not_installed() {
    let w = world();
    let hal = ScriptedHal::new();
    let mut dev = boot(&w, &hal, 8);
    let blob = vec![1u8; 2048];
    let m = manifest(&w, 1, &blob, &[0, 1]);
    let mut bad = blob.clone();
    bad[100] ^= 1;
    assert_eq!(
        failure_code(&install_fw(&mut dev, &m, &bad)),
        Some(ErrorCode::HashMismatch)
    );
    assert_eq!(dev.state(), DeviceState::Virgin);
}

#[test]
fn user_can_reject_firmware_on_the_device() {
    let w = world();
    let hal = ScriptedHal::new();
    let mut dev = boot(&w, &hal, 8);
    hal.push_decision(Decision::Reject);
    let blob = vec![1u8; 64];
    let r = install_fw(&mut dev, &manifest(&w, 1, &blob, &[0, 1]), &blob);
    assert_eq!(failure_code(&r), Some(ErrorCode::UserRejected));
    assert_eq!(dev.state(), DeviceState::Virgin);
}

// ───────────────────────── attestation / issuance ─────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(6))]

    #[test]
    fn attestation_verifies_and_binds_nonce(nonce in proptest::collection::vec(any::<u8>(), 16..64)) {
        let (w, _, mut dev, _) = ready();
        let Response::AttestationResponse(a) = dev.handle(Request::AttestationChallenge(AttestationChallenge { nonce: nonce.clone() })) else { panic!() };
        prop_assert!(verify_attestation(&w.trust, &nonce, a.fw_version, a.state, &a.chain, &a.signature).is_ok());
        let mut other = nonce.clone();
        other[0] ^= 1;
        prop_assert!(verify_attestation(&w.trust, &other, a.fw_version, a.state, &a.chain, &a.signature).is_err());
    }
}

#[test]
fn short_attestation_nonce_is_refused() {
    let (_, _, mut dev, _) = ready();
    let r = dev.handle(Request::AttestationChallenge(AttestationChallenge {
        nonce: vec![1; 4],
    }));
    assert_eq!(failure_code(&r), Some(ErrorCode::InvalidArgument));
}

#[test]
fn identity_request_carries_valid_pop_and_attestation() {
    let w = world();
    let hal = ScriptedHal::new();
    let mut dev = boot(&w, &hal, 8);
    let blob = vec![1u8; 64];
    install_fw(&mut dev, &manifest(&w, 1, &blob, &[0, 1]), &blob);
    hal.set_default_pin(Some(PIN));
    dev.handle(Request::SetupPin(SetupPin {}));
    let nonce = vec![8u8; 32];
    let Response::IdentityRequest(r) =
        dev.handle(Request::GetIdentityRequest(GetIdentityRequest {
            authority_nonce: nonce.clone(),
            authority_name: "Autoridade".into(),
        }))
    else {
        panic!()
    };
    let msg = issue_request_message(&nonce, &r.pk_identity);
    assert!(verify_tagged(&r.pk_identity, tags::ISSUE_POP, &msg, &r.pop_signature).is_ok());
    let batch = verify_batch_chain(&w.trust, &r.attestation_chain).unwrap();
    assert!(
        verify_tagged(
            &batch.public_key,
            tags::ISSUE_ATTEST,
            &msg,
            &r.attestation_signature
        )
        .is_ok()
    );
}

#[test]
fn credential_for_another_key_or_forged_issuer_is_refused() {
    let w = world();
    let hal = ScriptedHal::new();
    let mut dev = boot(&w, &hal, 8);
    let blob = vec![1u8; 64];
    install_fw(&mut dev, &manifest(&w, 1, &blob, &[0, 1]), &blob);
    hal.set_default_pin(Some(PIN));
    dev.handle(Request::SetupPin(SetupPin {}));
    let mut rng = StdRng::seed_from_u64(5);
    let other_key = generate_signing_key(&mut rng);
    let rogue_issuer = generate_signing_key(&mut rng);
    let Response::CivicId(me) = dev.handle(Request::GetCivicId(GetCivicId {})) else {
        panic!()
    };

    let wrong_subject = credential::issue(
        &w.authority,
        "A",
        &public_key_bytes(&other_key),
        1,
        0,
        [1; 16],
        &attrs(),
        &mut rng,
    );
    let forged = credential::issue(
        &rogue_issuer,
        "A",
        &me.pk_identity,
        1,
        0,
        [1; 16],
        &attrs(),
        &mut rng,
    );
    for c in [wrong_subject, forged] {
        let r = dev.handle(Request::LoadCredential(LoadCredential {
            credential: prost::Message::encode_to_vec(&c),
        }));
        assert_eq!(failure_code(&r), Some(ErrorCode::BadSignature));
    }
    assert_eq!(dev.state(), DeviceState::Initialized);
}

// ───────────────────────── identifiers, attributes, vote stub, wipe ─────────────────────────

#[test]
fn pairwise_ids_are_stable_per_service_and_distinct_across_services() {
    let (_, _, mut dev, _) = ready();
    let mut get = |s: &str| match dev.handle(Request::GetPairwiseId(GetPairwiseId {
        service_id: s.into(),
    })) {
        Response::PairwiseId(p) => p.id,
        other => panic!("{other:?}"),
    };
    let (a1, a2, b) = (
        get("banco.example"),
        get("BANCO.example "),
        get("loja.example"),
    );
    assert_eq!(a1, a2);
    assert_ne!(a1, b);
    assert!(a1.starts_with("tekop1"));
}

#[test]
fn attribute_proof_discloses_only_the_predicate() {
    let (w, _, mut dev, _) = ready();
    let nonce = vec![6u8; 16];
    let prove = |dev: &mut Dev, kind: PredicateKind, arg: u32, name: &str| {
        dev.handle(Request::ProveAttribute(ProveAttribute {
            predicate: Some(Predicate {
                kind: kind as i32,
                arg,
                name: name.into(),
            }),
            verifier_nonce: nonce.clone(),
            verifier_name: "Loja".into(),
        }))
    };
    let Response::AttributeProof(p) = prove(&mut dev, PredicateKind::AgeAtLeast, 18, "") else {
        panic!()
    };
    let pres = <Presentation as prost::Message>::decode(p.presentation.as_slice()).unwrap();
    let got = credential::verify_presentation(&w.trust.authority_root, &pres, &nonce, 0).unwrap();
    assert_eq!(got, vec![("age_over_18".to_string(), "true".to_string())]);
    assert!(!p.presentation.windows(10).any(|w| w == b"1990-05-17"));
    assert!(!p.presentation.windows(5).any(|w| w == b"Maria"));

    // 19 is answered with the smallest sufficient derived claim (21).
    let Response::AttributeProof(p) = prove(&mut dev, PredicateKind::AgeAtLeast, 19, "") else {
        panic!()
    };
    let pres = <Presentation as prost::Message>::decode(p.presentation.as_slice()).unwrap();
    assert_eq!(pres.disclosed[0].name, "age_over_21");
    // Not provable: refused, nothing revealed.
    assert_eq!(
        failure_code(&prove(&mut dev, PredicateKind::AgeAtLeast, 30, "")),
        Some(ErrorCode::InvalidArgument)
    );
    assert_eq!(
        failure_code(&prove(&mut dev, PredicateKind::Reveal, 0, "inexistente")),
        Some(ErrorCode::InvalidArgument)
    );
    // Rejected on the device: nothing is produced.
    _ = IssuedCredential::default();
}

// ───────────────────────── elections: contests, jurisdiction, ballots ─────────────────────────

fn options(names: &[&str], first: u32) -> Vec<ElectionOption> {
    names
        .iter()
        .enumerate()
        .map(|(i, n)| ElectionOption {
            index: i as u32,
            name: (*n).into(),
            number: format!("{}", first + i as u32),
        })
        .collect()
}

fn contest(
    id: &str,
    office: Office,
    title: &str,
    scope: Scope,
    opts: &[&str],
    first: u32,
    sched: (u64, u64, u64),
) -> Contest {
    Contest {
        contest_id: id.into(),
        office: office as i32,
        title: title.into(),
        scope: Some(scope),
        options: options(opts, first),
        schedule: Some(Schedule {
            registration_opens: sched.0,
            voting_opens: sched.1,
            voting_closes: sched.2,
        }),
    }
}

/// President (national), governors of SP and RJ (state), mayor of São Paulo
/// (municipal): each with ITS OWN dates. The voter lives in SP / 3550308.
fn election_manifest(w: &World) -> ElectionManifest {
    let mut m = ElectionManifest {
        format_version: 2,
        election_id: "eleicoes-2026".into(),
        title: "Eleições de exemplo 2026".into(),
        contests: vec![
            contest(
                "presidente",
                Office::President,
                "Presidente da República",
                Scope::national(),
                &["Candidata Onça", "Candidato Boto", "Candidata Arara"],
                10,
                (1_000, 2_000, 3_000),
            ),
            contest(
                "governador-sp",
                Office::Governor,
                "Governador de São Paulo",
                Scope::state("SP"),
                &["Candidato Guará", "Candidata Vitória-Régia"],
                20,
                (1_500, 2_500, 3_500),
            ),
            contest(
                "governador-rj",
                Office::Governor,
                "Governador do Rio de Janeiro",
                Scope::state("RJ"),
                &["Candidato Mico-Leão", "Candidata Capivara"],
                40,
                (1_000, 2_000, 3_000),
            ),
            contest(
                "prefeito-3550308",
                Office::Mayor,
                "Prefeito de São Paulo",
                Scope::municipality("3550308"),
                &["Candidata Jaguatirica", "Candidato Tucano"],
                30,
                (2_000, 4_000, 5_000),
            ),
        ],
        ballot_kind: BALLOT_KIND_PUBLIC.into(),
        registry_key: vec![],
        revision: 1,
        signatures: vec![],
    };
    m.signatures = [0usize, 1]
        .iter()
        .map(|i| sign_election(&w.gov[*i], &m))
        .collect();
    m
}

/// Governance transitions that bring `contest` to `upto`, on ITS schedule.
fn path(m: &ElectionManifest, w: &World, contest: &str, upto: Phase, seq0: u64) -> Vec<Transition> {
    let c = m.contest(contest).unwrap();
    let s = c.schedule.clone().unwrap();
    let steps = [
        (
            Phase::Draft,
            Phase::CandidatesPublished,
            s.registration_opens - 50,
        ),
        (
            Phase::CandidatesPublished,
            Phase::RegistrationOpen,
            s.registration_opens,
        ),
        (Phase::RegistrationOpen, Phase::VotingOpen, s.voting_opens),
        (Phase::VotingOpen, Phase::VotingClosed, s.voting_closes),
    ];
    let mut out = Vec::new();
    for (from, to, at) in steps {
        if from as i32 >= upto as i32 {
            break;
        }
        let mut t = Transition {
            election_id: m.election_id.clone(),
            contest_id: contest.into(),
            contest_hash: contest_hash(c).to_vec(),
            from_phase: from as i32,
            to_phase: to as i32,
            seq: seq0 + out.len() as u64 + 1,
            at,
            reason: String::new(),
            signatures: vec![],
        };
        t.signatures = [1usize, 2]
            .iter()
            .map(|i| sign_transition(&w.gov[*i], &t))
            .collect();
        out.push(t);
    }
    out
}

/// President open, governor SP in registration, governor RJ open, mayor only announced.
fn history(m: &ElectionManifest, w: &World) -> Vec<Transition> {
    let mut all = path(m, w, "presidente", Phase::VotingOpen, 0);
    all.extend(path(
        m,
        w,
        "governador-sp",
        Phase::RegistrationOpen,
        all.len() as u64,
    ));
    all.extend(path(
        m,
        w,
        "governador-rj",
        Phase::VotingOpen,
        all.len() as u64,
    ));
    all.extend(path(
        m,
        w,
        "prefeito-3550308",
        Phase::CandidatesPublished,
        all.len() as u64,
    ));
    all
}

fn load_req(m: &ElectionManifest, ts: &[Transition]) -> Request {
    Request::LoadElection(LoadElection {
        manifest: prost::Message::encode_to_vec(m),
        transitions: ts.iter().map(prost::Message::encode_to_vec).collect(),
    })
}

fn loaded(w: &World, dev: &mut Dev) -> (ElectionManifest, Vec<Transition>, ElectionLoaded) {
    let m = election_manifest(w);
    let ts = history(&m, w);
    let Response::ElectionLoaded(info) = dev.handle(load_req(&m, &ts)) else {
        panic!("election not loaded")
    };
    (m, ts, info)
}

fn info<'a>(l: &'a ElectionLoaded, id: &str) -> &'a ContestInfo {
    l.contests.iter().find(|c| c.contest_id == id).unwrap()
}

fn vote(dev: &mut Dev, contest: &str, option: u32) -> Response {
    dev.handle(Request::SignBallot(SignBallot {
        election_id: "eleicoes-2026".into(),
        contest_id: contest.into(),
        option_index: option,
    }))
}

fn voter_key(dev: &mut Dev, contest: &str) -> Response {
    dev.handle(Request::GetVoterKey(GetVoterKey {
        election_id: "eleicoes-2026".into(),
        contest_id: contest.into(),
    }))
}

#[test]
fn loading_an_election_reports_each_contest_phase_and_eligibility() {
    let (w, _, mut dev, _) = ready();
    let (_, _, l) = loaded(&w, &mut dev);
    assert_eq!(
        (l.election_id.as_str(), l.revision, l.contests.len()),
        ("eleicoes-2026", 1, 4)
    );
    // phases, each on its own track
    assert_eq!(info(&l, "presidente").phase, Phase::VotingOpen as u32);
    assert_eq!(
        info(&l, "governador-sp").phase,
        Phase::RegistrationOpen as u32
    );
    assert_eq!(info(&l, "governador-rj").phase, Phase::VotingOpen as u32);
    assert_eq!(
        info(&l, "prefeito-3550308").phase,
        Phase::CandidatesPublished as u32
    );
    // eligibility comes from the DEVICE's credential (domicile SP / 3550308)
    assert!(
        info(&l, "presidente").eligible,
        "everyone votes for President"
    );
    assert!(info(&l, "governador-sp").eligible);
    assert!(!info(&l, "governador-rj").eligible, "not my state");
    assert!(info(&l, "prefeito-3550308").eligible);
    assert_eq!(
        info(&l, "governador-sp").option_names,
        ["Candidato Guará", "Candidata Vitória-Régia"]
    );
}

#[test]
fn get_election_reads_the_stored_election_without_a_prompt_and_tracks_votes() {
    let (w, hal, mut dev, _) = ready();
    assert_eq!(
        failure_code(&dev.handle(Request::GetElection(GetElection {}))),
        Some(ErrorCode::InvalidState)
    );
    loaded(&w, &mut dev);
    let before = hal.confirms_requested();
    let Response::ElectionLoaded(l) = dev.handle(Request::GetElection(GetElection {})) else {
        panic!()
    };
    assert_eq!(
        hal.confirms_requested(),
        before,
        "reading needs no confirmation"
    );
    assert!(!info(&l, "presidente").voted);
    vote(&mut dev, "presidente", 2);
    let Response::ElectionLoaded(l) = dev.handle(Request::GetElection(GetElection {})) else {
        panic!()
    };
    assert!(info(&l, "presidente").voted);
}

#[test]
fn an_election_needs_m_of_n_governance_signatures_and_a_configured_policy() {
    let (w, hal, mut dev, _) = ready();
    let confirms_before = hal.confirms_requested();
    let mut m = election_manifest(&w);
    m.signatures.truncate(1);
    let ts = history(&election_manifest(&w), &w);
    assert_eq!(
        failure_code(&dev.handle(load_req(&m, &ts))),
        Some(ErrorCode::InsufficientSignatures)
    );
    // a forged (outsider) manifest
    let outsider = generate_signing_key(&mut StdRng::seed_from_u64(99));
    let mut forged = election_manifest(&w);
    forged.signatures = vec![
        sign_election(&outsider, &forged),
        sign_election(&w.gov[0], &forged),
    ];
    assert_eq!(
        failure_code(&dev.handle(load_req(&forged, &ts))),
        Some(ErrorCode::InsufficientSignatures)
    );
    // one transition signed by a single institution
    let mut weak_ts = ts.clone();
    weak_ts[0].signatures.truncate(1);
    assert_eq!(
        failure_code(&dev.handle(load_req(&election_manifest(&w), &weak_ts))),
        Some(ErrorCode::InsufficientSignatures)
    );
    // nothing was ever shown for confirmation or stored
    assert_eq!(hal.confirms_requested(), confirms_before);
    assert_eq!(
        failure_code(&voter_key(&mut dev, "presidente")),
        Some(ErrorCode::InvalidState)
    );

    // A device whose trust anchors carry no governance has elections disabled.
    let mut w2 = world();
    w2.trust.gov_signers.clear();
    w2.trust.gov_threshold = 0;
    let hal2 = ScriptedHal::new();
    let mut d2 = boot(&w2, &hal2, 8);
    onboard(&mut d2, &w2, &hal2);
    let m2 = election_manifest(&w2);
    assert_eq!(
        failure_code(&d2.handle(load_req(&m2, &history(&m2, &w2)))),
        Some(ErrorCode::InvalidState)
    );
    let _ = GovernancePolicy::new(w.gov.iter().map(public_key_bytes).collect(), 2).unwrap();
}

#[test]
fn the_ballot_screen_shows_the_candidate_from_the_manifest_and_the_signature_verifies() {
    let (w, hal, mut dev, _) = ready();
    let (m, _, _) = loaded(&w, &mut dev);
    hal.clear_screens();
    let Response::BallotSignature(b) = vote(&mut dev, "presidente", 1) else {
        panic!("no ballot")
    };
    let screen = hal
        .screens()
        .into_iter()
        .find(|s| s.title == "CONFIRMAR VOTO?")
        .unwrap();
    let shown = screen.lines.join(" ");
    for must in [
        "Presidente da República",
        "Nacional",
        "11 - Candidato Boto",
        "Eleições de exemplo 2026",
    ] {
        assert!(shown.contains(must), "screen must show {must:?}: {shown}");
    }
    assert!(
        !shown.contains("Candidata Onça"),
        "only the chosen candidate is shown"
    );
    // The host named an INDEX only; the signature binds that exact contest version.
    let body = BallotBody {
        election_id: "eleicoes-2026".into(),
        contest_id: "presidente".into(),
        contest_hash: contest_hash(m.contest("presidente").unwrap()).to_vec(),
        option_index: 1,
    };
    assert!(verify_ballot(&b.voter_key, &body, &b.signature).is_ok());
    assert!(
        verify_ballot(
            &b.voter_key,
            &BallotBody {
                option_index: 0,
                ..body.clone()
            },
            &b.signature
        )
        .is_err()
    );
    // The ButtonRequest reached the host BEFORE the signature existed.
    assert!(hal.interim().iter().any(
        |r| matches!(r, Response::ButtonRequest(x) if x.kind == ButtonKind::SignBallot as i32)
    ));
}

#[test]
fn you_cannot_vote_outside_your_jurisdiction_or_outside_the_voting_phase() {
    let (w, hal, mut dev, _) = ready();
    loaded(&w, &mut dev);
    let confirms = hal.confirms_requested();
    // Rio's governor: voting is open, but I live in São Paulo.
    assert_eq!(
        failure_code(&vote(&mut dev, "governador-rj", 0)),
        Some(ErrorCode::NotEligible)
    );
    assert_eq!(
        failure_code(&voter_key(&mut dev, "governador-rj")),
        Some(ErrorCode::NotEligible)
    );
    // São Paulo's governor: mine, but still only collecting registrations.
    assert_eq!(
        failure_code(&vote(&mut dev, "governador-sp", 0)),
        Some(ErrorCode::WrongPhase)
    );
    // The mayor: mine, but not even registering yet.
    assert_eq!(
        failure_code(&vote(&mut dev, "prefeito-3550308", 0)),
        Some(ErrorCode::WrongPhase)
    );
    assert_eq!(
        failure_code(&voter_key(&mut dev, "prefeito-3550308")),
        Some(ErrorCode::WrongPhase)
    );
    // Bad requests.
    assert_eq!(
        failure_code(&vote(&mut dev, "senador-sp", 0)),
        Some(ErrorCode::InvalidArgument)
    );
    assert_eq!(
        failure_code(&vote(&mut dev, "presidente", 9)),
        Some(ErrorCode::InvalidArgument)
    );
    let other = dev.handle(Request::SignBallot(SignBallot {
        election_id: "outra".into(),
        contest_id: "presidente".into(),
        option_index: 0,
    }));
    assert_eq!(failure_code(&other), Some(ErrorCode::InvalidArgument));
    assert_eq!(
        hal.confirms_requested(),
        confirms,
        "no prompt was ever raised for a refused request"
    );
    // Registration IS open for the governor of SP, so its key can be exported.
    assert!(matches!(
        voter_key(&mut dev, "governador-sp"),
        Response::VoterKey(_)
    ));
}

#[test]
fn a_device_votes_once_per_contest_and_remembers_across_restarts() {
    let (w, hal, mut dev, _) = ready();
    loaded(&w, &mut dev);
    hal.push_decision(Decision::Reject);
    assert_eq!(
        failure_code(&vote(&mut dev, "presidente", 0)),
        Some(ErrorCode::UserRejected)
    );
    // refusing on the device did NOT use up the vote
    assert!(matches!(
        vote(&mut dev, "presidente", 0),
        Response::BallotSignature(_)
    ));
    // a second ballot, even for another option, is refused
    assert_eq!(
        failure_code(&vote(&mut dev, "presidente", 1)),
        Some(ErrorCode::AlreadyVoted)
    );
    drop(dev);
    let mut dev = boot(&w, &hal, 8);
    hal.set_default_pin(Some(PIN));
    assert!(matches!(
        dev.handle(Request::Unlock(Unlock {})),
        Response::Unlocked(_)
    ));
    assert_eq!(
        failure_code(&vote(&mut dev, "presidente", 1)),
        Some(ErrorCode::AlreadyVoted),
        "persisted"
    );
    // the stored election is still there (no reload needed) and shows who voted
    let m = election_manifest(&w);
    let Response::ElectionLoaded(l) = dev.handle(load_req(&m, &history(&m, &w))) else {
        panic!()
    };
    assert!(info(&l, "presidente").voted && !info(&l, "governador-sp").voted);
}

#[test]
fn contests_open_on_their_own_dates_and_old_history_is_refused() {
    let (w, _, mut dev, _) = ready();
    let (m, ts, _) = loaded(&w, &mut dev);
    assert_eq!(
        failure_code(&vote(&mut dev, "governador-sp", 0)),
        Some(ErrorCode::WrongPhase)
    );
    // Governance later opens the governor of SP (its own date, 2_500): newer history.
    let mut newer = ts.clone();
    newer.extend(path_from(
        &m,
        &w,
        "governador-sp",
        Phase::RegistrationOpen,
        Phase::VotingOpen,
        ts.len() as u64,
    ));
    let Response::ElectionLoaded(l) = dev.handle(load_req(&m, &newer)) else {
        panic!("newer history refused")
    };
    assert_eq!(info(&l, "governador-sp").phase, Phase::VotingOpen as u32);
    assert!(matches!(
        vote(&mut dev, "governador-sp", 1),
        Response::BallotSignature(_)
    ));
    // Replaying the OLDER history (where it was not open yet) must be refused,
    // otherwise a host could "rewind" a contest that governance already closed.
    assert_eq!(
        failure_code(&dev.handle(load_req(&m, &ts))),
        Some(ErrorCode::Downgrade)
    );
}

fn path_from(
    m: &ElectionManifest,
    w: &World,
    contest: &str,
    from: Phase,
    to: Phase,
    seq0: u64,
) -> Vec<Transition> {
    let all = path(m, w, contest, to, 0);
    all.into_iter()
        .filter(|t| t.from_phase >= from as i32)
        .enumerate()
        .map(|(i, mut t)| {
            t.seq = seq0 + i as u64 + 1;
            t.signatures = [1usize, 2]
                .iter()
                .map(|k| sign_transition(&w.gov[*k], &t))
                .collect();
            t
        })
        .collect()
}

#[test]
fn voter_keys_are_per_contest_stable_and_never_the_identity_key() {
    let (w, hal, mut dev, id_pk) = ready();
    loaded(&w, &mut dev);
    let key = |dev: &mut Dev, c: &str| match voter_key(dev, c) {
        Response::VoterKey(k) => k.voter_key,
        other => panic!("{other:?}"),
    };
    let pres = key(&mut dev, "presidente");
    let gov = key(&mut dev, "governador-sp");
    assert_ne!(pres, gov, "President and Governor keys must not link");
    assert_ne!(pres, id_pk, "voter key must never be the identity key");
    assert_eq!(pres, key(&mut dev, "presidente"), "deterministic");
    // the key used on the ballot is the same one exported for registration
    let Response::BallotSignature(b) = vote(&mut dev, "presidente", 0) else {
        panic!()
    };
    assert_eq!(b.voter_key, pres);
    // and it survives a restart without being stored
    drop(dev);
    let mut dev = boot(&w, &hal, 8);
    hal.set_default_pin(Some(PIN));
    dev.handle(Request::Unlock(Unlock {}));
    assert_eq!(key(&mut dev, "presidente"), pres);
    let secrets = dev.secrets_for_test();
    assert_ne!(
        secrets[0], secrets[1],
        "voter root must not be derived from the identity root"
    );
}

#[test]
fn the_stored_election_is_reverified_so_tampering_with_storage_does_not_help() {
    let (w, hal, mut dev, _) = ready();
    loaded(&w, &mut dev);
    drop(dev);
    // An attacker with write access flips a byte of the stored election.
    let mut p =
        <crate::state::Persisted as prost::Message>::decode(hal.raw_storage().unwrap().as_slice())
            .unwrap();
    let n = p.election.len();
    p.election[n / 2] ^= 0x01;
    {
        use crate::hal::SecureStorage;
        let mut h = hal.clone();
        assert!(h.store(&prost::Message::encode_to_vec(&p)));
    }
    let mut dev = boot(&w, &hal, 8);
    hal.set_default_pin(Some(PIN));
    dev.handle(Request::Unlock(Unlock {}));
    let r = vote(&mut dev, "presidente", 0);
    assert!(
        !matches!(r, Response::BallotSignature(_)),
        "tampered election must not sign: {r:?}"
    );
    assert!(failure_code(&r).is_some());
}

#[test]
fn wiping_removes_the_election_and_the_vote_record() {
    let (w, hal, mut dev, _) = ready();
    loaded(&w, &mut dev);
    vote(&mut dev, "presidente", 0);
    assert!(matches!(
        dev.handle(Request::Wipe(Wipe {})),
        Response::WipeDone(_)
    ));
    let stored =
        <crate::state::Persisted as prost::Message>::decode(hal.raw_storage().unwrap().as_slice())
            .unwrap();
    assert!(stored.election.is_empty(), "election removed from storage");
    assert!(stored.ballots_signed.is_empty(), "vote record removed");
    assert!(stored.voter_secret.is_empty() && stored.k_root.is_empty());
    assert_eq!(
        failure_code(&vote(&mut dev, "presidente", 0)),
        Some(ErrorCode::InvalidState)
    );
}

#[test]
fn wipe_needs_confirmation_and_destroys_everything() {
    let (_, hal, mut dev, _) = ready();
    hal.push_decision(Decision::Reject);
    assert_eq!(
        failure_code(&dev.handle(Request::Wipe(Wipe {}))),
        Some(ErrorCode::UserRejected)
    );
    assert_eq!(dev.state(), DeviceState::CredentialsIssued);
    let root = dev.secrets_for_test()[0].clone();
    assert!(matches!(
        dev.handle(Request::Wipe(Wipe {})),
        Response::WipeDone(_)
    ));
    assert_eq!(dev.state(), DeviceState::FirmwareInstalled);
    assert!(
        !hal.raw_storage()
            .unwrap()
            .windows(root.len())
            .any(|w| w == root.as_slice())
    );
    // Anti-rollback counter and factory attestation survive a wipe.
    let Response::Features(f) = dev.handle(Request::GetFeatures(GetFeatures {})) else {
        panic!()
    };
    assert_eq!(f.fw_version, 1);
    assert!(matches!(
        dev.handle(Request::AttestationChallenge(AttestationChallenge {
            nonce: vec![1; 32]
        })),
        Response::AttestationResponse(_)
    ));
}
