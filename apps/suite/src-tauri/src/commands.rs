use std::fs;
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use std::time::Duration;

use election_admin::{Admin, feed_new};
#[cfg(feature = "dev-simulation")]
use issuer_cli::{ISSUER_NAME, Issuer, Subject};
#[cfg(feature = "dev-simulation")]
use prost::Message;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use teko_attestation::TrustPolicy;
use teko_bulletin::BulletinBoard;
use teko_client::{
    AttributeResult, AuthenticityReport, CivicIdInfo, ClientError, DeviceClient, DeviceStatus,
    ElectionSummary, FirmwareInfo, SignedBallot, SignedDocument, UnlockOutcome, VerifiedDocument,
    inspect_firmware, resolve_dev_dir, verify_signed_document,
};
use teko_crypto::hex;
use teko_election::{Phase, ResultsVisibility, SubmittedBallot, SubmittedRegistration, Tallier};
use teko_protocol::{ButtonKind, Predicate, PredicateKind};
use teko_transport::{TcpTransport, TransportError, connect_tcp};

type Client = DeviceClient<TcpTransport>;

pub struct SuiteInner {
    pub client: Mutex<Option<Client>>,
    pub addr: String,
    /// Incremental tally for the live results view.
    pub results: Mutex<Option<Tallier>>,
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
        ButtonKind::LoadElection => "load_election",
        ButtonKind::VoterKey => "voter_key",
        ButtonKind::Register => "register",
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

/// The authority's signed revocation list, if it has published one. A list that
/// does not verify is ignored by the client (it only ever LOADS verified lists).
fn load_revocations(c: &mut Client, trust: &TrustPolicy) {
    let path = resolve_dev_dir().join("revocations.signed");
    if let Ok(bytes) = fs::read(path)
        && let Ok(list) =
            <teko_crypto::credential::RevocationList as prost::Message>::decode(bytes.as_slice())
    {
        let _ = c.use_revocations(trust, list);
    }
}

/// Returned when a command needs the TEST authority/operator, which a production
/// build does not contain (`dev-simulation` feature).
#[cfg(not(feature = "dev-simulation"))]
fn dev_only() -> UiError {
    ui_err(
        "DevOnly",
        "Disponível só no ambiente de desenvolvimento: numa versão de produção a autoridade e o operador do quadro são serviços externos.",
    )
}

/// Whether this build carries the test authority/operator (the UI hides those actions otherwise).
#[tauri::command]
pub fn dev_mode() -> bool {
    cfg!(feature = "dev-simulation")
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
/// What the person tells the authority (in person, in real life).
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(not(feature = "dev-simulation"), allow(dead_code))]
pub struct IssueForm {
    name: String,
    birth_date: String,
    nationality: String,
    state: String,
    municipality: String,
    /// Set = REISSUE (lost/replaced device): the previous credential is revoked.
    old_civic_id: Option<String>,
    reason: Option<String>,
}

#[tauri::command]
pub async fn issue_credential(
    app: AppHandle,
    suite: State<'_, Suite>,
    form: IssueForm,
) -> Result<(), UiError> {
    issue_credential_impl(app, suite, form).await
}

#[cfg(not(feature = "dev-simulation"))]
async fn issue_credential_impl(
    _app: AppHandle,
    _suite: State<'_, Suite>,
    _form: IssueForm,
) -> Result<(), UiError> {
    Err(dev_only())
}

#[cfg(feature = "dev-simulation")]
async fn issue_credential_impl(
    app: AppHandle,
    suite: State<'_, Suite>,
    form: IssueForm,
) -> Result<(), UiError> {
    let IssueForm {
        name,
        birth_date,
        nationality,
        state,
        municipality,
        old_civic_id,
        reason,
    } = form;
    let subject = Subject {
        name,
        birth_date,
        nationality,
        electoral_state: state,
        electoral_municipality: municipality,
    };
    with_device(app, suite.inner().clone(), move |c| {
        let issuer = Issuer::open(resolve_dev_dir());
        let nonce = Issuer::new_nonce();
        let request = c.identity_request(&nonce, ISSUER_NAME)?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let issued = match old_civic_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            // In-person REISSUE (no seed): revokes the previous credential.
            Some(old) => issuer.reissue_credential_at(
                &nonce,
                &request,
                &subject,
                old,
                reason.as_deref().unwrap_or("lost"),
                now,
            ),
            None => issuer.issue_credential(&nonce, &request, &subject),
        }
        .map_err(|e| ClientError::Verification(e.to_string()))?;
        c.load_credential(&issued.encode_to_vec())
    })
    .await
}

/// "I lost my device": the authority blocks the credential at once. Needs no device.
#[tauri::command]
pub async fn revoke_credential(civic_id: String, reason: String) -> Result<usize, UiError> {
    #[cfg(not(feature = "dev-simulation"))]
    {
        let _ = (civic_id, reason);
        Err(dev_only())
    }
    #[cfg(feature = "dev-simulation")]
    {
        revoke_impl(civic_id, reason)
    }
}

#[cfg(feature = "dev-simulation")]
fn revoke_impl(civic_id: String, reason: String) -> Result<usize, UiError> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    Issuer::open(resolve_dev_dir())
        .revoke_civic_id(civic_id.trim(), &reason, now)
        .map_err(|e| ui_err("Revoke", e))
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
        load_revocations(c, &trust);
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
        load_revocations(c, &trust);
        c.prove_attribute(&trust, predicate, &verifier)
    })
    .await
}

// ───────────────────────── elections & wipe ─────────────────────────

/// Gives the device the demo election (`dev/election.bundle`). The Suite checks
/// governance first; the device checks again and asks the user to confirm.
#[tauri::command]
pub async fn election_load(
    app: AppHandle,
    suite: State<'_, Suite>,
) -> Result<ElectionSummary, UiError> {
    let trust = load_trust()?;
    let bytes = read_bundle()?;
    with_device(app, suite.inner().clone(), move |c| {
        c.load_election(&trust, &bytes)
    })
    .await
}

fn read_bundle() -> Result<Vec<u8>, UiError> {
    fs::read(resolve_dev_dir().join("election.bundle")).map_err(|_| {
        ui_err(
            "NoElection",
            "Não encontrei dev/election.bundle. Rode: make setup",
        )
    })
}

/// Reads the election the device already holds (no confirmation).
#[tauri::command]
pub async fn election_get(
    app: AppHandle,
    suite: State<'_, Suite>,
) -> Result<ElectionSummary, UiError> {
    let trust = load_trust()?;
    let bytes = read_bundle()?;
    with_device(app, suite.inner().clone(), move |c| {
        if !c.has_election() {
            // After a reconnection: re-verify the bundle on the host (no device traffic).
            c.use_election_bundle(&trust, &bytes)?;
        }
        c.election()
    })
    .await
}

#[derive(Serialize)]
pub struct VoteReceipt {
    ballot: SignedBallot,
    /// PROTOTYPE: the Suite plays the board operator and appends the ballot.
    submitted: bool,
    board_index: Option<u64>,
    note: Option<String>,
}

/// PROTOTYPE: the device signs the ballot after the user confirms on ITS screen
/// (candidate name from the verified manifest); the Suite then appends it to the
/// local bulletin board so it shows up in the live results.
#[tauri::command]
pub async fn election_vote(
    app: AppHandle,
    suite: State<'_, Suite>,
    election_id: String,
    contest_id: String,
    option_index: u32,
) -> Result<VoteReceipt, UiError> {
    let trust = load_trust()?;
    let bytes = read_bundle()?;
    let (ballot, reg_note) = with_device(app, suite.inner().clone(), move |c| {
        if !c.has_election() {
            c.use_election_bundle(&trust, &bytes)?;
        }
        let reg_note = register_anonymously(c, &election_id, &contest_id)?;
        Ok((
            c.sign_ballot(&election_id, &contest_id, option_index)?,
            reg_note,
        ))
    })
    .await?;
    let submitted = submit_to_board(&ballot);
    Ok(VoteReceipt {
        ballot,
        submitted: submitted.is_ok(),
        board_index: submitted.as_ref().ok().copied(),
        note: submitted.err().or(reg_note),
    })
}

/// ANONYMOUS REGISTRATION (ADR-0008) for contests that require it: the DEVICE
/// builds the blinded message (and asks the user on its own screen), the
/// authority checks who the citizen is and blind-signs, the device unblinds, and
/// `(voter_key, token)` goes to the board on its own. The authority never sees
/// the voter key and the board never sees the identity.
///
/// Returns a warning when the token could not be published (the vote would not
/// count); a refusal by the device or the authority is an error.
fn register_anonymously(
    c: &mut Client,
    election_id: &str,
    contest_id: &str,
) -> Result<Option<String>, ClientError> {
    let required = c
        .election()?
        .contests
        .iter()
        .find(|x| x.contest_id == contest_id)
        .is_some_and(|x| x.registration_required);
    if !required {
        return Ok(None);
    }
    let req = c.begin_registration(election_id, contest_id)?;
    let sig = authority_sign_registration(election_id, contest_id, &req)?;
    let token = c.finish_registration(election_id, contest_id, &sig)?;
    let published = Admin::open(resolve_dev_dir())
        .map_err(|e| e.to_string())
        .and_then(|admin| {
            if !admin.has_board() {
                return Err("o quadro ainda não foi publicado".to_string());
            }
            admin
                .submit_registration(&SubmittedRegistration {
                    election_id: election_id.to_string(),
                    contest_id: contest_id.to_string(),
                    voter_key: token.voter_key,
                    token: token.token,
                })
                .map(|_| ())
                .map_err(|e| e.to_string())
        });
    Ok(published
        .err()
        .map(|e| format!("Registro anônimo não publicado ({e}): a cédula não será contada.")))
}

/// The registration AUTHORITY: in development it is the simulated `issuer-cli`; a
/// production Suite would call the real authority's service instead.
#[cfg(feature = "dev-simulation")]
fn authority_sign_registration(
    election_id: &str,
    contest_id: &str,
    req: &teko_protocol::RegistrationRequest,
) -> Result<Vec<u8>, ClientError> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    Issuer::open(resolve_dev_dir())
        .sign_registration(election_id, contest_id, req, now)
        .map_err(|e| ClientError::Unexpected(e.to_string()))
}

#[cfg(not(feature = "dev-simulation"))]
fn authority_sign_registration(
    _election_id: &str,
    _contest_id: &str,
    _req: &teko_protocol::RegistrationRequest,
) -> Result<Vec<u8>, ClientError> {
    Err(ClientError::Unexpected(
        "o registro anônimo precisa da autoridade (serviço externo, ainda não integrado): disponível só em desenvolvimento".into(),
    ))
}

fn submit_to_board(b: &SignedBallot) -> Result<u64, String> {
    let admin = Admin::open(resolve_dev_dir()).map_err(|e| e.to_string())?;
    if !admin.has_board() {
        return Err(
            "O quadro ainda não foi publicado: a cédula não entrou na contagem ao vivo.".into(),
        );
    }
    let h = |s: &str| hex::decode(s).map_err(|e| e.to_string());
    let ballot = SubmittedBallot {
        election_id: b.election_id.clone(),
        contest_id: b.contest_id.clone(),
        contest_hash: h(&b.contest_hash)?,
        // Encrypted mode: the vote lives only in the ciphertexts; the signed body has option 0.
        option_index: if b.encrypted.is_empty() {
            b.option_index
        } else {
            0
        },
        voter_key: h(&b.voter_key)?,
        signature: h(&b.signature)?,
        encrypted: h(&b.encrypted)?,
    };
    admin
        .submit_ballot(&ballot)
        .map(|r| r.index)
        .map_err(|e| e.to_string())
}

// ───────────────────────── live results (public; no device needed) ─────────────────────────

#[derive(Serialize)]
pub struct OptionView {
    index: u32,
    name: String,
    number: String,
    votes: u64,
}

#[derive(Serialize)]
pub struct ContestView {
    contest_id: String,
    title: String,
    office: &'static str,
    scope: &'static str,
    scope_code: String,
    phase: &'static str,
    /// `None` = hidden by the results policy.
    accepted: Option<u64>,
    rejected: u64,
    /// Whether the contest requires anonymous registration, and how many keys did.
    registration_required: bool,
    registered: u64,
    /// Ballots are encrypted: the distribution exists only after the validators decrypt.
    encrypted: bool,
    /// The totals were decrypted by enough validators and verified.
    revealed: bool,
    distribution_visible: bool,
    top: Vec<OptionView>,
    others_votes: u64,
    others_count: usize,
}

#[derive(Serialize)]
pub struct ResultsReply {
    available: bool,
    reason: Option<String>,
    election_id: String,
    title: String,
    revision: u32,
    visibility: &'static str,
    simulated: bool,
    head_size: u64,
    head_root: String,
    ignored_governance: u64,
    contests: Vec<ContestView>,
}

fn unavailable(reason: &str) -> ResultsReply {
    ResultsReply {
        available: false,
        reason: Some(reason.to_string()),
        election_id: String::new(),
        title: String::new(),
        revision: 0,
        visibility: "unspecified",
        simulated: false,
        head_size: 0,
        head_root: String::new(),
        ignored_governance: 0,
        contests: Vec::new(),
    }
}

fn office_name(o: teko_election::Office) -> &'static str {
    use teko_election::Office::*;
    match o {
        President => "president",
        Governor => "governor",
        Senator => "senator",
        FederalDeputy => "federal_deputy",
        StateDeputy => "state_deputy",
        Mayor => "mayor",
        CityCouncillor => "city_councillor",
        Other => "other",
        Unspecified => "unspecified",
    }
}

fn phase_name(p: Phase) -> &'static str {
    match p {
        Phase::Draft => "draft",
        Phase::CandidatesPublished => "candidates_published",
        Phase::RegistrationOpen => "registration_open",
        Phase::VotingOpen => "voting_open",
        Phase::VotingClosed => "voting_closed",
        Phase::Tallied => "tallied",
        Phase::Audited => "audited",
        Phase::Cancelled => "cancelled",
    }
}

/// Reads the bulletin board and returns the CURRENT tally (incremental: only new
/// entries are verified). Public information: needs no device.
#[tauri::command]
pub async fn results_snapshot(suite: State<'_, Suite>) -> Result<ResultsReply, UiError> {
    let suite = suite.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let admin = match Admin::open(resolve_dev_dir()) {
            Ok(a) => a,
            Err(e) => return unavailable(&e.to_string()),
        };
        // SIMULATED governance: contests past their deadline are closed in the log
        // (at most every 5 s), so the public count locks without anyone clicking.
        #[cfg(feature = "dev-simulation")]
        {
            static LAST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            if now.saturating_sub(LAST.load(std::sync::atomic::Ordering::Relaxed)) >= 5 {
                LAST.store(now, std::sync::atomic::Ordering::Relaxed);
                let _ = admin.close_due(now);
            }
        }
        let board = match admin.read_board() {
            Ok(b) => b,
            Err(e) => return unavailable(&format!("quadro ilegível: {e}")),
        };
        if board.is_empty() {
            return unavailable("O quadro de avisos ainda está vazio: publique a eleição.");
        }
        let mut guard = suite.results.lock().unwrap_or_else(|e| e.into_inner());
        // A shrunk board (reset/new election) invalidates the incremental state.
        if guard.as_ref().is_some_and(|t| t.consumed() > board.len()) {
            *guard = None;
        }
        let tallier = guard.get_or_insert_with(|| admin.new_tallier());
        feed_new(tallier, &board);
        let t = tallier.snapshot();
        let manifest = tallier.manifest().cloned();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let contests = t
            .contests
            .iter()
            .map(|c| {
                // After the last allowed moment the vote is locked and the result
                // is shown without waiting for anyone to publish the closing step.
                let mut closed = c.clone();
                let due = manifest
                    .as_ref()
                    .and_then(|m| m.contest(&c.contest_id))
                    .and_then(|x| x.schedule.as_ref())
                    .is_some_and(|s| now >= s.voting_closes);
                if due && closed.phase == Phase::VotingOpen {
                    closed.phase = Phase::VotingClosed;
                }
                let c = &closed;
                let view = c.public_view(t.visibility);
                let ranked = c.ranked(10);
                let show = view.distribution;
                ContestView {
                    contest_id: c.contest_id.clone(),
                    title: c.title.clone(),
                    office: office_name(c.office),
                    scope: match c.scope.level() {
                        teko_election::ScopeLevel::National => "national",
                        teko_election::ScopeLevel::State => "state",
                        teko_election::ScopeLevel::Municipality => "municipality",
                        teko_election::ScopeLevel::Unspecified => "unknown",
                    },
                    scope_code: c.scope.code.clone(),
                    phase: phase_name(c.phase),
                    accepted: view.accepted,
                    rejected: c.rejected.total(),
                    registration_required: c.registration_required,
                    registered: c.registered,
                    encrypted: c.encrypted,
                    revealed: c.revealed,
                    distribution_visible: show,
                    top: if show {
                        ranked
                            .top
                            .iter()
                            .map(|o| OptionView {
                                index: o.index,
                                name: o.name.clone(),
                                number: o.number.clone(),
                                votes: o.votes,
                            })
                            .collect()
                    } else {
                        Vec::new()
                    },
                    others_votes: if show { ranked.others_votes } else { 0 },
                    others_count: if show { ranked.others_count } else { 0 },
                }
            })
            .collect();
        ResultsReply {
            available: true,
            reason: None,
            election_id: t.election_id.clone(),
            title: t.title.clone(),
            revision: t.revision,
            visibility: match t.visibility {
                ResultsVisibility::Live => "live",
                ResultsVisibility::TurnoutOnly => "turnout_only",
                ResultsVisibility::AfterClose => "after_close",
                ResultsVisibility::Unspecified => "unspecified",
            },
            simulated: admin.is_simulated(),
            head_size: board.len(),
            head_root: hex::encode(&board.root()),
            ignored_governance: t.ignored_governance,
            contests,
        }
    })
    .await
    .map_err(|e| ui_err("Internal", e))
}

// ───────────────────────── public explorer (no device needed) ─────────────────────────

#[derive(Serialize)]
pub struct ExplorerOption {
    number: String,
    name: String,
}

#[derive(Serialize)]
pub struct ExplorerContest {
    contest_id: String,
    title: String,
    office: &'static str,
    scope: &'static str,
    scope_code: String,
    phase: &'static str,
    registration_opens: u64,
    voting_opens: u64,
    voting_closes: u64,
    registration_required: bool,
    options: Vec<ExplorerOption>,
}

#[derive(Serialize)]
pub struct GovernanceStep {
    seq: u64,
    contest_id: String,
    from: &'static str,
    to: &'static str,
    at: u64,
    signatures: usize,
    /// Position of this decision in the public log.
    board_index: u64,
}

#[derive(Serialize)]
pub struct ExplorerReply {
    available: bool,
    reason: Option<String>,
    election_id: String,
    title: String,
    revision: u32,
    head_size: u64,
    head_root: String,
    contests: Vec<ExplorerContest>,
    governance: Vec<GovernanceStep>,
    /// The validators' key ceremony, re-verified here (None if the election has no encrypted ballots).
    ceremony: Option<CeremonyView>,
}

#[derive(Serialize)]
pub struct CeremonyView {
    threshold: usize,
    validators: usize,
    /// The election public key every ballot is encrypted to.
    election_key: String,
    /// Does it match the key in the signed manifest?
    matches_manifest: bool,
}

fn explorer_unavailable(reason: &str) -> ExplorerReply {
    ExplorerReply {
        available: false,
        reason: Some(reason.to_string()),
        election_id: String::new(),
        title: String::new(),
        revision: 0,
        head_size: 0,
        head_root: String::new(),
        contests: Vec::new(),
        governance: Vec::new(),
        ceremony: None,
    }
}

/// Everything PUBLIC about the election, straight from the bulletin board: rules,
/// candidates, calendar and the signed governance decisions in log order. It
/// replays the log itself (M-of-N checked), so nothing here is taken on trust.
#[tauri::command]
pub async fn explorer_snapshot() -> Result<ExplorerReply, UiError> {
    tauri::async_runtime::spawn_blocking(move || {
        let admin = match Admin::open(resolve_dev_dir()) {
            Ok(a) => a,
            Err(e) => return explorer_unavailable(&e.to_string()),
        };
        let board = match admin.read_board() {
            Ok(b) => b,
            Err(e) => return explorer_unavailable(&format!("quadro ilegível: {e}")),
        };
        if board.is_empty() {
            return explorer_unavailable(
                "O quadro de avisos ainda está vazio: publique a eleição.",
            );
        }
        let mut t = admin.new_tallier();
        feed_new(&mut t, &board);
        let (Some(m), snap) = (t.manifest().cloned(), t.snapshot()) else {
            return explorer_unavailable("Nenhuma eleição válida no quadro.");
        };
        let m_tally_key = m.tally_key.clone();
        let contests = m
            .contests
            .iter()
            .map(|c| {
                let sch = c.schedule.clone().unwrap_or_default();
                let scope = c.scope.clone().unwrap_or_default();
                ExplorerContest {
                    contest_id: c.contest_id.clone(),
                    title: c.title.clone(),
                    office: office_name(c.office()),
                    scope: match scope.level() {
                        teko_election::ScopeLevel::National => "national",
                        teko_election::ScopeLevel::State => "state",
                        teko_election::ScopeLevel::Municipality => "municipality",
                        teko_election::ScopeLevel::Unspecified => "unknown",
                    },
                    scope_code: scope.code.clone(),
                    phase: snap
                        .contest(&c.contest_id)
                        .map_or("draft", |x| phase_name(x.phase)),
                    registration_opens: sch.registration_opens,
                    voting_opens: sch.voting_opens,
                    voting_closes: sch.voting_closes,
                    registration_required: !c.registration_key.is_empty(),
                    options: c
                        .options
                        .iter()
                        .map(|o| ExplorerOption {
                            number: o.number.clone(),
                            name: o.name.clone(),
                        })
                        .collect(),
                }
            })
            .collect();
        let phase_of = |p: i32| Phase::try_from(p).map_or("draft", phase_name);
        let governance = (0..board.len())
            .filter_map(|i| {
                let e = board.entry(i)?;
                if e.kind != teko_bulletin::EntryKind::TRANSITION {
                    return None;
                }
                let tr =
                    <teko_election::Transition as prost::Message>::decode(e.payload.as_slice())
                        .ok()?;
                Some(GovernanceStep {
                    seq: tr.seq,
                    contest_id: tr.contest_id.clone(),
                    from: phase_of(tr.from_phase),
                    to: phase_of(tr.to_phase),
                    at: tr.at,
                    signatures: tr.signatures.len(),
                    board_index: i,
                })
            })
            .collect();
        ExplorerReply {
            available: true,
            reason: None,
            election_id: snap.election_id,
            title: snap.title,
            revision: snap.revision,
            head_size: board.len(),
            head_root: hex::encode(&board.root()),
            contests,
            governance,
            ceremony: (0..board.len())
                .filter_map(|i| board.entry(i))
                .filter(|e| e.kind == teko_bulletin::EntryKind::DKG)
                .find_map(|e| {
                    let rec = <teko_ballot_crypto::dkg_wire::DkgRecord as prost::Message>::decode(
                        e.payload.as_slice(),
                    )
                    .ok()?;
                    // Re-verified HERE: every proof of possession and the key recomputed.
                    let v = rec.verify().ok()?;
                    Some(CeremonyView {
                        threshold: v.threshold,
                        validators: v.verification_keys.len(),
                        matches_manifest: m_tally_key == rec.group_key,
                        election_key: hex::encode(&rec.group_key),
                    })
                }),
        }
    })
    .await
    .map_err(|e| ui_err("Internal", e))
}

#[derive(Serialize)]
pub struct NodeView {
    index: usize,
    addr: String,
    up: bool,
    height: u64,
    mempool: u32,
    peers_online: u32,
    equivocations: u32,
}

#[derive(Serialize)]
pub struct BlockView {
    height: u64,
    entries: usize,
    signatures: usize,
    round: u32,
    timestamp: u64,
    /// Validator (by index) that proposed it.
    proposer: usize,
    hash: String,
}

#[derive(Serialize)]
pub struct NetworkReply {
    /// "file" = local single-operator log; "net" = federated PoA test network.
    mode: &'static str,
    name: String,
    chain_id: String,
    threshold: usize,
    validators: usize,
    height: u64,
    nodes: Vec<NodeView>,
    /// Most recent blocks, newest first (every one verified here).
    blocks: Vec<BlockView>,
    error: Option<String>,
}

/// Which "blockchain" the Suite is reading, with the state of each node and the
/// latest blocks. The blocks shown were verified in THIS process (M-of-N
/// signatures, hash chain, Merkle state), not taken from a node's word.
#[tauri::command]
pub async fn network_status() -> Result<NetworkReply, UiError> {
    tauri::async_runtime::spawn_blocking(move || {
        let file_mode = NetworkReply {
            mode: "file",
            name: "arquivo local".into(),
            chain_id: String::new(),
            threshold: 1,
            validators: 1,
            height: 0,
            nodes: Vec::new(),
            blocks: Vec::new(),
            error: None,
        };
        let Ok(admin) = Admin::open(resolve_dev_dir()) else {
            return file_mode;
        };
        let Some(net) = admin.network() else {
            return NetworkReply {
                height: admin.read_board().map_or(0, |b| b.len()),
                ..file_mode
            };
        };
        let nodes = net
            .client()
            .statuses()
            .into_iter()
            .enumerate()
            .map(|(i, (addr, s))| NodeView {
                index: i,
                addr: addr.to_string(),
                up: s.is_some(),
                height: s.as_ref().map_or(0, |s| s.height),
                mempool: s.as_ref().map_or(0, |s| s.mempool),
                peers_online: s.as_ref().map_or(0, |s| s.peers_online),
                equivocations: s.as_ref().map_or(0, |s| s.equivocations),
            })
            .collect();
        let (blocks, height, error) = match admin.read_board() {
            Ok(election_admin::AnyBoard::Net(nb)) => {
                let chain = nb.chain();
                let validators = &net.profile.validators;
                let v = chain
                    .map(|c| {
                        c.blocks()
                            .iter()
                            .rev()
                            .take(20)
                            .map(|b| BlockView {
                                height: b.height,
                                entries: b.entries.len(),
                                signatures: b.signatures.len(),
                                round: b.round,
                                timestamp: b.timestamp,
                                proposer: validators
                                    .iter()
                                    .position(|k| *k == b.proposer)
                                    .unwrap_or(0),
                                hash: hex::encode(&b.header_hash()),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                (v, nb.height(), None)
            }
            Ok(_) => (Vec::new(), 0, None),
            Err(e) => (Vec::new(), 0, Some(e.to_string())),
        };
        NetworkReply {
            mode: "net",
            name: net.profile.kind.name().to_string(),
            chain_id: net.profile.chain_id.clone(),
            threshold: net.profile.threshold,
            validators: net.profile.validators.len(),
            height,
            nodes,
            blocks,
            error,
        }
    })
    .await
    .map_err(|e| ui_err("Internal", e))
}

#[derive(Serialize)]
pub struct InclusionReply {
    valid: bool,
    kind: &'static str,
    index: u64,
    size: u64,
    root: String,
    proof_len: usize,
}

/// "Is entry #index really in the log?": recomputes the Merkle inclusion proof
/// against the CURRENT root (RFC 9162). The check runs here, not on trust of the
/// board file's own bookkeeping.
#[tauri::command]
pub async fn verify_inclusion(index: u64) -> Result<InclusionReply, UiError> {
    tauri::async_runtime::spawn_blocking(move || {
        let admin = Admin::open(resolve_dev_dir()).map_err(|e| ui_err("NoElection", e))?;
        let board = admin
            .read_board()
            .map_err(|e| ui_err("NoElection", format!("quadro ilegível: {e}")))?;
        let entry = board
            .entry(index)
            .ok_or_else(|| ui_err("InvalidArgument", "não existe essa posição no quadro"))?;
        let proof = board
            .inclusion_proof(index)
            .map_err(|e| ui_err("Internal", e))?;
        let head = teko_bulletin::SignedHead {
            size: board.len(),
            root: board.root(),
            operator_key: Vec::new(),
            signature: Vec::new(),
        };
        Ok(InclusionReply {
            valid: teko_bulletin::verify_entry_inclusion(entry, index, &head, &proof),
            kind: match entry.kind {
                teko_bulletin::EntryKind::MANIFEST => "manifesto",
                teko_bulletin::EntryKind::TRANSITION => "decisão da governança",
                teko_bulletin::EntryKind::REGISTRATION => "registro anônimo",
                teko_bulletin::EntryKind::BALLOT => "cédula",
                _ => "outro",
            },
            index,
            size: board.len(),
            root: hex::encode(&board.root()),
            proof_len: proof.len(),
        })
    })
    .await
    .map_err(|e| ui_err("Internal", e))?
}

/// Publishes the demo election on the local board (idempotent).
#[tauri::command]
pub fn board_publish() -> Result<u64, UiError> {
    #[cfg(not(feature = "dev-simulation"))]
    {
        Err(dev_only())
    }
    #[cfg(feature = "dev-simulation")]
    {
        let admin = Admin::open(resolve_dev_dir()).map_err(|e| ui_err("NoElection", e))?;
        admin
            .publish()
            .map(|r| r.appended)
            .map_err(|e| ui_err("NoElection", e))
    }
}

#[tauri::command]
pub async fn wipe(app: AppHandle, suite: State<'_, Suite>) -> Result<(), UiError> {
    with_device(app, suite.inner().clone(), |c| c.wipe()).await
}
