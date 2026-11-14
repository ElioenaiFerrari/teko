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
use rand::rngs::OsRng;
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
            OsRng,
            cfg,
            trust2,
            Some(factory),
        )
        .unwrap();
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
    let id = c.civic_id().unwrap();
    assert!(id.id.starts_with("teko1"));
    let nonce = Issuer::new_nonce();
    let req = c.identity_request(&nonce, "Autoridade Cívica").unwrap();
    let subject = Subject {
        name: "Maria da Silva".into(),
        birth_date: "1990-05-17".into(),
        nationality: "BR".into(),
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

    // pairwise ids + voting stubs
    let a = c.pairwise_id("banco.exemplo").unwrap();
    assert_ne!(a, c.pairwise_id("loja.exemplo").unwrap());
    assert_eq!(c.voter_commitment().unwrap().len(), 64);
    assert_eq!(
        device_code(c.sign_ballot("eleicao-2026", "Candidato 13").unwrap_err()),
        ErrorCode::NotImplemented
    );

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
