use std::fs;
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use std::time::Duration;

use issuer_cli::{ISSUER_NAME, Issuer, Subject};
use prost::Message;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use teko_attestation::TrustPolicy;
use teko_client::{
    AttributeResult, AuthenticityReport, CivicIdInfo, ClientError, DeviceClient, DeviceStatus,
    FirmwareInfo, SignedDocument, UnlockOutcome, VerifiedDocument, inspect_firmware,
    resolve_dev_dir, verify_signed_document,
};
use teko_crypto::hex;
use teko_protocol::{ButtonKind, Predicate, PredicateKind};
use teko_transport::{TcpTransport, TransportError, connect_tcp};

type Client = DeviceClient<TcpTransport>;

pub struct SuiteInner {
    pub client: Mutex<Option<Client>>,
    pub addr: String,
}

pub type Suite = Arc<SuiteInner>;

// ───────────────────────── errors ─────────────────────────

/// Error shape the UI receives (`invoke` rejects with this object).
#[derive(Debug, Serialize)]
pub struct UiError {
    code: String,
    message: String,
    retry_after_ms: u64,
    remaining: u32,
}

fn ui_err(code: &str, message: impl ToString) -> UiError {
    UiError {
        code: code.to_string(),
        message: message.to_string(),
        retry_after_ms: 0,
        remaining: 0,
    }
}

impl From<ClientError> for UiError {
    fn from(e: ClientError) -> Self {
        let mut u = ui_err(&e.code(), &e);
        if let ClientError::Device(f) = &e {
            u.retry_after_ms = f.retry_after_ms;
            u.remaining = f.remaining;
        }
        u
    }
}

fn prompt_name(kind: ButtonKind) -> &'static str {
    match kind {
        ButtonKind::PinEntry => "pin_entry",
        ButtonKind::FirmwareInstall => "firmware_install",
        ButtonKind::ConfirmCivicId => "confirm_civic_id",
        ButtonKind::PairwiseId => "pairwise_id",
        ButtonKind::SignDocument => "sign_document",
        ButtonKind::ProveAttribute => "prove_attribute",
        ButtonKind::ExportCommitment => "export_commitment",
        ButtonKind::SignBallot => "sign_ballot",
        ButtonKind::Wipe => "wipe",
        ButtonKind::IssueIdentity => "issue_identity",
        ButtonKind::LoadCredential => "load_credential",
        ButtonKind::Unspecified => "unknown",
    }
}

/// Hard cap on payloads coming from the (untrusted) UI over IPC.
const MAX_FILE_BYTES: usize = 32 * 1024 * 1024;

fn check_size(bytes: &[u8]) -> Result<(), UiError> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(ui_err(
            "InvalidArgument",
            "arquivo grande demais (limite: 32 MB)",
        ));
    }
    Ok(())
}

// ───────────────────────── plumbing ─────────────────────────

fn load_trust() -> Result<TrustPolicy, UiError> {
    let dir = resolve_dev_dir();
    let text = fs::read_to_string(dir.join("trust.txt")).map_err(|_| {
        ui_err(
            "NoTrust",
            "Não encontrei dev/trust.txt. Rode: cargo run -p issuer-cli -- dev-setup",
        )
    })?;
    TrustPolicy::parse(&text).map_err(|e| ui_err("NoTrust", e))
}

fn lock(inner: &SuiteInner) -> MutexGuard<'_, Option<Client>> {
    inner.client.lock().unwrap_or_else(|e| e.into_inner())
}

fn ensure_connected(
    inner: &SuiteInner,
    app: &AppHandle,
    slot: &mut Option<Client>,
) -> Result<(), ClientError> {
    if slot.is_none() {
        let transport = connect_tcp(&inner.addr, Duration::from_millis(700))?;
        let mut client = DeviceClient::new(transport);
        let app = app.clone();
        client.set_on_button(move |kind| {
            let _ = app.emit("device-prompt", prompt_name(kind));
        });
        client.initialize()?;
        *slot = Some(client);
    }
    Ok(())
}

/// Runs `f` against the device on a blocking thread (it may wait for the user
/// to act on the device). A transport failure drops the connection.
async fn with_device<T, F>(app: AppHandle, suite: Suite, f: F) -> Result<T, UiError>
where
    T: Send + 'static,
    F: FnOnce(&mut Client) -> Result<T, ClientError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || {
        let mut guard = lock(&suite);
        ensure_connected(&suite, &app, &mut guard)?;
        let client = guard
            .as_mut()
            .ok_or_else(|| ui_err("Internal", "sem conexão"))?;
        let result = f(client);
        if matches!(result, Err(ClientError::Transport(_))) {
            *guard = None;
        }
        result.map_err(UiError::from)
    })
    .await
    .map_err(|e| ui_err("Internal", e))?
}

// ───────────────────────── status ─────────────────────────

#[derive(Serialize)]
pub struct StatusReply {
    connected: bool,
    busy: bool,
    status: Option<DeviceStatus>,
    error: Option<String>,
    addr: String,
    trust_ok: bool,
}

#[tauri::command]
pub async fn device_status(
    app: AppHandle,
    suite: State<'_, Suite>,
) -> Result<StatusReply, UiError> {
    let suite = suite.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut reply = StatusReply {
            connected: false,
            busy: false,
            status: None,
            error: None,
            addr: suite.addr.clone(),
            trust_ok: load_trust().is_ok(),
        };
        // If an operation is in flight (waiting for the user on the device),
        // do not queue behind it: report "busy".
        let mut guard = match suite.client.try_lock() {
            Ok(g) => g,
            Err(TryLockError::WouldBlock) => {
                reply.busy = true;
                return reply;
            }
            Err(TryLockError::Poisoned(p)) => p.into_inner(),
        };
        match ensure_connected(&suite, &app, &mut guard) {
            Ok(()) => match guard.as_mut().map(Client::features) {
                Some(Ok(s)) => {
                    reply.connected = true;
                    reply.status = Some(s);
                }
                Some(Err(e)) => {
                    *guard = None;
                    reply.error = Some(e.to_string());
                }
                None => {}
            },
            // "No device" is a normal state, not an error to display.
            Err(ClientError::Transport(TransportError::Io(_))) => {}
            Err(e) => reply.error = Some(e.to_string()),
        }
        reply
    })
    .await
    .map_err(|e| ui_err("Internal", e))
}

#[derive(Serialize)]
pub struct DevInfo {
    dev_dir: String,
    addr: String,
    authority_root: Option<String>,
}

#[tauri::command]
pub fn dev_info(suite: State<'_, Suite>) -> DevInfo {
    DevInfo {
        dev_dir: resolve_dev_dir().display().to_string(),
        addr: suite.addr.clone(),
        authority_root: load_trust().ok().map(|t| hex::encode(&t.authority_root)),
    }
}

// ───────────────────────── authenticity & firmware ─────────────────────────

#[tauri::command]
pub async fn verify_authenticity(
    app: AppHandle,
    suite: State<'_, Suite>,
) -> Result<AuthenticityReport, UiError> {
    let trust = load_trust()?;
    with_device(app, suite.inner().clone(), move |c| {
        c.verify_authenticity(&trust)
    })
    .await
}

#[derive(Serialize)]
pub struct FirmwareOffer {
    #[serde(flatten)]
    info: FirmwareInfo,
    verdict: String,
    installable: bool,
}

#[tauri::command]
pub async fn firmware_catalog(
    app: AppHandle,
    suite: State<'_, Suite>,
) -> Result<Vec<FirmwareOffer>, UiError> {
    let trust = load_trust()?;
    let installed = with_device(app, suite.inner().clone(), |c| c.features())
        .await?
        .fw_version;
    let dir = resolve_dev_dir();
    let mut offers = Vec::new();
    for v in 1u32..=20 {
        let (Ok(manifest), Ok(blob)) = (
            fs::read(dir.join(format!("firmware-v{v}.manifest"))),
            fs::read(dir.join(format!("firmware-v{v}.blob"))),
        ) else {
            continue;
        };
        let offer = match inspect_firmware(&trust, &manifest, &blob, installed) {
            Ok(info) => {
                let ok = info.valid_signers >= info.threshold;
                let (verdict, installable) = if !ok {
                    ("Assinaturas insuficientes: recusado", false)
                } else if info.is_downgrade {
                    (
                        "Versão anterior à instalada: recusado (anti-downgrade)",
                        false,
                    )
                } else if info.version == installed {
                    ("Já instalado", false)
                } else if installed == 0 {
                    ("Pronto para instalar", true)
                } else {
                    ("Atualização disponível", true)
                };
                FirmwareOffer {
                    info,
                    verdict: verdict.to_string(),
                    installable,
                }
            }
            Err(e) => {
                return Err(ui_err("Verification", format!("firmware v{v}: {e}")));
            }
        };
        offers.push(offer);
    }
    Ok(offers)
}

#[tauri::command]
pub async fn install_firmware(
    app: AppHandle,
    suite: State<'_, Suite>,
    version: u32,
) -> Result<u32, UiError> {
    let trust = load_trust()?;
    let dir = resolve_dev_dir();
    let manifest = fs::read(dir.join(format!("firmware-v{version}.manifest"))).map_err(|_| {
        ui_err(
            "NoFirmware",
            format!("firmware v{version} não encontrado em {}", dir.display()),
        )
    })?;
    let blob = fs::read(dir.join(format!("firmware-v{version}.blob")))
        .map_err(|_| ui_err("NoFirmware", format!("firmware v{version} não encontrado")))?;
    let progress_app = app.clone();
    with_device(app, suite.inner().clone(), move |c| {
        c.install_firmware(&trust, &manifest, &blob, &mut |done, total| {
            let _ = progress_app.emit("fw-progress", (done, total));
        })
    })
    .await
}

// ───────────────────────── PIN, identity, issuance ─────────────────────────

/// The PIN is typed ON THE DEVICE; this only waits for the outcome.
#[tauri::command]
pub async fn setup_pin(app: AppHandle, suite: State<'_, Suite>) -> Result<(), UiError> {
    with_device(app, suite.inner().clone(), |c| c.setup_pin()).await
}

#[tauri::command]
pub async fn unlock(app: AppHandle, suite: State<'_, Suite>) -> Result<UnlockOutcome, UiError> {
    with_device(app, suite.inner().clone(), |c| c.unlock()).await
}

#[tauri::command]
pub async fn civic_id(app: AppHandle, suite: State<'_, Suite>) -> Result<CivicIdInfo, UiError> {
    with_device(app, suite.inner().clone(), |c| c.civic_id()).await
}

/// Simulated authority: the person was verified out of band; the device
/// proves possession of its key and genuineness; the issuer binds the
/// credential to the device's identity key.
#[tauri::command]
pub async fn issue_credential(
    app: AppHandle,
    suite: State<'_, Suite>,
    name: String,
    birth_date: String,
    nationality: String,
) -> Result<(), UiError> {
    let subject = Subject {
        name,
        birth_date,
        nationality,
    };
    with_device(app, suite.inner().clone(), move |c| {
        let issuer = Issuer::open(resolve_dev_dir());
        let nonce = Issuer::new_nonce();
        let request = c.identity_request(&nonce, ISSUER_NAME)?;
        let issued = issuer
            .issue_credential(&nonce, &request, &subject)
            .map_err(|e| ClientError::Verification(e.to_string()))?;
        c.load_credential(&issued.encode_to_vec())
    })
    .await
}

fn reveal(name: &str) -> Predicate {
    Predicate {
        kind: PredicateKind::Reveal as i32,
        arg: 0,
        name: name.to_string(),
    }
}

/// Reveals the identification data, one on-device confirmation per field.
#[tauri::command]
pub async fn reveal_identity(
    app: AppHandle,
    suite: State<'_, Suite>,
) -> Result<Vec<(String, String)>, UiError> {
    let trust = load_trust()?;
    with_device(app, suite.inner().clone(), move |c| {
        let mut out = Vec::new();
        for field in ["name", "birth_date", "nationality", "registry_id"] {
            let r = c.prove_attribute(&trust, reveal(field), "Teko Suite (ver meus dados)")?;
            out.extend(r.disclosed);
        }
        Ok(out)
    })
    .await
}

#[tauri::command]
pub async fn pairwise_id(
    app: AppHandle,
    suite: State<'_, Suite>,
    service: String,
) -> Result<String, UiError> {
    with_device(app, suite.inner().clone(), move |c| c.pairwise_id(&service)).await
}

// ───────────────────────── documents & attributes ─────────────────────────

/// SHA3-256 of a file, shown BEFORE signing so the user can compare it with
/// the hash the device displays.
#[tauri::command]
pub fn hash_file(bytes: Vec<u8>) -> Result<String, UiError> {
    check_size(&bytes)?;
    Ok(hex::encode(&teko_client::file_hash(&bytes)))
}

#[tauri::command]
pub async fn sign_document(
    app: AppHandle,
    suite: State<'_, Suite>,
    bytes: Vec<u8>,
    summary: String,
) -> Result<SignedDocument, UiError> {
    check_size(&bytes)?;
    with_device(app, suite.inner().clone(), move |c| {
        c.sign_document(&bytes, &summary)
    })
    .await
}

fn safe_file_stem(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let s = s.trim_matches('-');
    if s.is_empty() {
        "documento".to_string()
    } else {
        s.chars().take(60).collect()
    }
}

/// Saves the signature bundle into the user's Downloads folder.
#[tauri::command]
pub fn save_signed_document(
    app: AppHandle,
    doc: SignedDocument,
    file_name: String,
) -> Result<String, UiError> {
    let dir = app
        .path()
        .download_dir()
        .map_err(|e| ui_err("Io", format!("pasta Downloads indisponível: {e}")))?;
    let stem = safe_file_stem(
        file_name
            .rsplit_once('.')
            .map_or(file_name.as_str(), |(s, _)| s),
    );
    let mut path = dir.join(format!("{stem}.teko-signature.json"));
    let mut n = 1;
    while path.exists() {
        path = dir.join(format!("{stem}-{n}.teko-signature.json"));
        n += 1;
    }
    let json = serde_json::to_string_pretty(&doc).map_err(|e| ui_err("Io", e))?;
    fs::write(&path, json).map_err(|e| ui_err("Io", e))?;
    Ok(path.display().to_string())
}

#[tauri::command]
pub fn verify_document(
    bundle_json: String,
    file: Option<Vec<u8>>,
) -> Result<VerifiedDocument, UiError> {
    if bundle_json.len() > 64 * 1024 {
        return Err(ui_err(
            "Verification",
            "arquivo de assinatura grande demais",
        ));
    }
    if let Some(f) = &file {
        check_size(f)?;
    }
    let doc: SignedDocument = serde_json::from_str(&bundle_json).map_err(|_| {
        ui_err(
            "Verification",
            "arquivo de assinatura inválido (esperado JSON do Teko)",
        )
    })?;
    verify_signed_document(&doc, file.as_deref()).map_err(UiError::from)
}

#[tauri::command]
pub async fn prove_attribute(
    app: AppHandle,
    suite: State<'_, Suite>,
    kind: String,
    arg: u32,
    name: String,
    verifier: String,
) -> Result<AttributeResult, UiError> {
    let trust = load_trust()?;
    let kind = match kind.as_str() {
        "age_at_least" => PredicateKind::AgeAtLeast,
        "has" => PredicateKind::Has,
        "reveal" => PredicateKind::Reveal,
        _ => return Err(ui_err("InvalidArgument", "predicado desconhecido")),
    };
    let predicate = Predicate {
        kind: kind as i32,
        arg,
        name,
    };
    with_device(app, suite.inner().clone(), move |c| {
        c.prove_attribute(&trust, predicate, &verifier)
    })
    .await
}

// ───────────────────────── voting (stub) & wipe ─────────────────────────

/// Wired to the device-side `SignBallot` stub. Expected result today:
/// the device asks for confirmation, then answers `NotImplemented`.
#[tauri::command]
pub async fn cast_vote(
    app: AppHandle,
    suite: State<'_, Suite>,
    election: String,
    choice: String,
) -> Result<(), UiError> {
    with_device(app, suite.inner().clone(), move |c| {
        c.sign_ballot(&election, &choice)
    })
    .await
}

#[tauri::command]
pub async fn wipe(app: AppHandle, suite: State<'_, Suite>) -> Result<(), UiError> {
    with_device(app, suite.inner().clone(), |c| c.wipe()).await
}
