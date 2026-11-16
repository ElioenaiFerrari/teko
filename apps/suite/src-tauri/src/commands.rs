use std::fs;
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use std::time::Duration;

use election_admin::{Admin, AnyBoard, feed_step};
#[cfg(feature = "dev-simulation")]
use issuer_cli::Issuer;
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
    /// The tally of the board, SHARED by the live results and the public explorer and
    /// advanced in steps (also by a background warm-up), so neither screen replays the
    /// board and a step never blocks the UI for long. Keyed by the FIRST entry of the
    /// board it was built from, so a different chain/election (reset, new network) never
    /// reuses stale state.
    pub board: Mutex<Option<BoardState>>,
    /// Credential keys this session exported (type -> hex), to list what was issued to them.
    pub cred_keys: Mutex<std::collections::BTreeMap<String, String>>,
}

pub struct BoardState {
    first: teko_bulletin::Hash,
    tallier: Tallier,
}

/// Entries verified per step (a step takes well under a second on several cores).
const BOARD_STEP: u64 = 400;

/// What a screen needs after one step over the shared tally.
struct Walk {
    board: AnyBoard,
    snap: teko_election::ElectionTally,
    manifest: Option<teko_election::ElectionManifest>,
    /// Entries verified so far, of `board.len()`.
    verified: u64,
}

/// Prints a timing line in the terminal that runs the Suite when something is slow, so
/// it is visible where the time goes.
fn log_slow(what: &str, started: std::time::Instant, detail: impl FnOnce() -> String) {
    let took = started.elapsed();
    if took >= Duration::from_millis(300) {
        eprintln!("[suite] {what}: {took:.1?} ({})", detail());
    }
}

/// Reads the board (a verified replica of the network, incremental) and moves the shared
/// tally forward by at most `step` entries.
fn walk_board(suite: &Suite, admin: &Admin, step: u64) -> Result<Walk, String> {
    let board = admin
        .read_board()
        .map_err(|e| format!("quadro ilegível: {e}"))?;
    if board.is_empty() {
        return Err("O quadro de avisos ainda está vazio: publique a eleição.".into());
    }
    let mut guard = suite.board.lock().unwrap_or_else(|e| e.into_inner());
    let first = board.entry(0).map_or([0; 32], |e| e.leaf_hash());
    if guard
        .as_ref()
        .is_some_and(|s| s.first != first || s.tallier.consumed() > board.len())
    {
        *guard = None;
    }
    let st = guard.get_or_insert_with(|| BoardState {
        first,
        tallier: admin.new_tallier(),
    });
    let (started, before) = (std::time::Instant::now(), st.tallier.consumed());
    feed_step(&mut st.tallier, &board, step);
    let verified = st.tallier.consumed();
    log_slow("quadro", started, || {
        format!(
            "+{} entradas verificadas, {verified} de {}",
            verified - before,
            board.len()
        )
    });
    Ok(Walk {
        snap: st.tallier.snapshot(),
        manifest: st.tallier.manifest().cloned(),
        verified,
        board,
    })
}

/// Keeps the shared tally up to date in the background from the moment the Suite opens,
/// so by the time Results or the Explorer is opened the board is usually verified
/// already. Idle (and cheap) when there is nothing new.
pub fn spawn_board_warmup(suite: Suite) {
    std::thread::spawn(move || {
        loop {
            let behind = match Admin::open(resolve_dev_dir()) {
                Ok(admin) => walk_board(&suite, &admin, BOARD_STEP)
                    .map(|w| w.verified < w.board.len())
                    .unwrap_or(false),
                Err(_) => false,
            };
            std::thread::sleep(Duration::from_millis(if behind { 10 } else { 2_000 }));
        }
    });
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
        ButtonKind::WalletCreate => "wallet_create",
        ButtonKind::SeedWords => "seed_words",
        ButtonKind::WalletConfirm => "wallet_confirm",
        ButtonKind::WalletRestore => "wallet_restore",
        ButtonKind::WalletCheck => "wallet_check",
        ButtonKind::WalletPassphrase => "wallet_passphrase",
        ButtonKind::SignTransfer => "sign_transfer",
        ButtonKind::SignAssetOffer => "sign_asset_offer",
        ButtonKind::SignAssetAccept => "sign_asset_accept",
        ButtonKind::SignStake => "sign_stake",
        ButtonKind::CredentialKey => "credential_key",
        ButtonKind::SignCredentialOp => "sign_credential_op",
        ButtonKind::OpenCredential => "open_credential",
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

/// The device creates the wallet and shows the 24 words ON ITS SCREEN; the Suite only
/// waits (it never receives the words).
#[tauri::command]
pub async fn wallet_setup(
    app: AppHandle,
    suite: State<'_, Suite>,
) -> Result<teko_client::WalletInfo, UiError> {
    with_device(app, suite.inner().clone(), |c| c.wallet_setup()).await
}

/// The user types the 24 words ON THE DEVICE.
#[tauri::command]
pub async fn wallet_restore(
    app: AppHandle,
    suite: State<'_, Suite>,
) -> Result<teko_client::WalletInfo, UiError> {
    with_device(app, suite.inner().clone(), |c| c.wallet_restore()).await
}

#[tauri::command]
pub async fn wallet_check_backup(app: AppHandle, suite: State<'_, Suite>) -> Result<(), UiError> {
    with_device(app, suite.inner().clone(), |c| c.wallet_check_backup()).await
}

/// The economy the devnet screens work with: the VERIFIED chain of a running node network
/// when there is one, else the local test file. DEVELOPMENT ONLY.
#[cfg(feature = "dev-simulation")]
struct EconCtx {
    world: Box<dyn teko_assets::devnet::World>,
    cast: teko_assets::devnet::Cast,
    /// "node" or "local".
    mode: &'static str,
}

#[cfg(feature = "dev-simulation")]
fn economy_open(dev: &std::path::Path) -> Result<EconCtx, String> {
    use teko_assets::devnet::{self, Cast};
    let net = dev.join("net");
    let opened = if net.join(teko_node::ECONOMY_FILE).exists() {
        let started = std::time::Instant::now();
        let before = teko_node::NodeWorld::full_syncs();
        let r = teko_node::NodeWorld::open(&net);
        log_slow("economia", started, || {
            if teko_node::NodeWorld::full_syncs() > before {
                "cadeia verificada do zero".to_string()
            } else {
                "só os blocos novos".to_string()
            }
        });
        match r {
            Ok(w) => Some(w),
            // say WHY the network world is not used (an old chain, nodes down...) instead
            // of silently showing the local test file
            Err(e) => {
                eprintln!("[suite] economia da rede indisponível: {e}");
                None
            }
        }
    } else {
        None
    };
    if let Some(w) = opened {
        let mut cast = Cast::load(dev)?.with_store(net.join(devnet::METADATA_DIR));
        for class in devnet::CLASSES {
            cast = cast.with_registrars(class, teko_node::load_class_registrar_keys(&net, class));
        }
        return Ok(EconCtx {
            world: Box::new(w),
            cast,
            mode: "node",
        });
    }
    let (econ, cast) = devnet::open(dev)?;
    Ok(EconCtx {
        world: Box::new(econ),
        cast,
        mode: "local",
    })
}

/// Result of a sent transfer.
#[derive(serde::Serialize)]
pub struct SendResult {
    amount: u64,
    fee: u64,
    to: String,
    balance_after: u64,
}

/// One line of the wallet history.
#[derive(serde::Serialize)]
pub struct HistoryItem {
    height: u64,
    /// "sent" or "received".
    direction: String,
    counterparty: String,
    amount: u64,
    fee: u64,
}

/// Sends coins. The DEVICE shows destination, amount and fee and signs only after the
/// physical confirmation; the Suite prepares the fields and submits the signed result.
/// DEVELOPMENT ONLY for now: it goes to the local devnet economy (`dev/economy.*`).
#[tauri::command]
pub async fn wallet_send(
    app: AppHandle,
    suite: State<'_, Suite>,
    to: String,
    amount: u64,
) -> Result<SendResult, UiError> {
    send_impl(app, suite, to, amount).await
}

#[cfg(not(feature = "dev-simulation"))]
async fn send_impl(
    _app: AppHandle,
    _suite: State<'_, Suite>,
    _to: String,
    _amount: u64,
) -> Result<SendResult, UiError> {
    Err(ui_err(
        "NotAvailable",
        "O livro de economia ainda não está na rede: enviar moedas ficará disponível com ele.",
    ))
}

#[cfg(feature = "dev-simulation")]
async fn send_impl(
    app: AppHandle,
    suite: State<'_, Suite>,
    to: String,
    amount: u64,
) -> Result<SendResult, UiError> {
    let dir = resolve_dev_dir();
    with_device(app, suite.inner().clone(), move |c| {
        let v = |m: String| ClientError::Verification(m);
        let dest = teko_crypto::parse_wallet_address(&to)
            .map_err(|_| v("endereço de destino inválido (confira o checksum)".into()))?;
        if amount == 0 {
            return Err(v("informe um valor maior que zero".into()));
        }
        let me = c.wallet_address()?;
        let pk = teko_crypto::hex::decode(&me.public_key)
            .map_err(|_| v("chave pública inválida".into()))?;
        let mine = teko_crypto::ledger_address(&pk);
        if mine == dest {
            return Err(v("o destino é a sua própria carteira".into()));
        }
        let mut ctx = economy_open(&dir).map_err(v)?;
        let econ = &mut ctx.world;
        let nonce = econ.nonce(&mine);
        let fee = econ.estimate_fee(&pk, nonce, &dest, amount);
        let needed = amount.saturating_add(fee);
        if econ.balance(&mine) < needed {
            return Err(v(format!(
                "saldo insuficiente: precisa de {} (valor {} + taxa {}), tem {}",
                teko_crypto::amount::format_amount(needed),
                teko_crypto::amount::format_amount(amount),
                teko_crypto::amount::format_amount(fee),
                teko_crypto::amount::format_amount(econ.balance(&mine))
            )));
        }
        let chain = econ.chain_id().to_string();
        let tx = c.wallet_sign_transfer(&chain, &to, amount, fee, nonce)?;
        econ.submit(tx).map_err(v)?;
        Ok(SendResult {
            amount,
            fee,
            to,
            balance_after: econ.balance(&mine),
        })
    })
    .await
}

/// Wallet history (DEVELOPMENT ONLY: local devnet economy).
#[tauri::command]
pub async fn wallet_history(address: String) -> Result<Vec<HistoryItem>, UiError> {
    Ok(history_impl(&address))
}

#[cfg(not(feature = "dev-simulation"))]
fn history_impl(_address: &str) -> Vec<HistoryItem> {
    Vec::new()
}

#[cfg(feature = "dev-simulation")]
fn history_impl(address: &str) -> Vec<HistoryItem> {
    let Ok(who) = teko_crypto::parse_wallet_address(address) else {
        return Vec::new();
    };
    let Ok(ctx) = economy_open(&resolve_dev_dir()) else {
        return Vec::new();
    };
    ctx.world
        .history(&who)
        .into_iter()
        .rev()
        .map(|r| {
            let sent = r.from == who;
            let other = if sent { r.to } else { r.from };
            HistoryItem {
                height: r.height,
                direction: if sent { "sent" } else { "received" }.into(),
                counterparty: teko_crypto::wallet_address(&other).unwrap_or_default(),
                amount: r.amount,
                fee: if sent { r.fee } else { 0 },
            }
        })
        .collect()
}

/// One asset of the assets screen.
#[derive(serde::Serialize)]
pub struct AssetItem {
    id: String,
    class: String,
    version: u64,
    liens: usize,
    /// What the metadata record says (plate, model...), when this computer holds a record
    /// that matches the hash on the chain; empty otherwise.
    summary: Vec<String>,
    /// True when the record is here AND matches the chain (a sale needs it).
    verified: bool,
}

/// One field of an asset class form.
#[derive(serde::Serialize)]
pub struct FormField {
    key: &'static str,
    label: &'static str,
    help: &'static str,
}

/// What the register form of a class needs: its fields and a VALID example. The rules
/// themselves stay in Rust (`teko_crypto::asset_meta`): the UI only shows the error.
#[derive(serde::Serialize)]
pub struct AssetForm {
    class: String,
    label: &'static str,
    fields: Vec<FormField>,
    sample: std::collections::BTreeMap<String, String>,
}

#[tauri::command]
pub async fn asset_form(class: String, n: Option<u32>) -> Result<AssetForm, UiError> {
    let spec = teko_crypto::asset_meta::schema(&class)
        .ok_or_else(|| ui_err("InvalidArgument", "classe desconhecida"))?;
    let sample = teko_crypto::asset_meta::sample(&class, n.unwrap_or(1))
        .ok_or_else(|| ui_err("InvalidArgument", "classe desconhecida"))?;
    Ok(AssetForm {
        label: if class == "veiculo" {
            "Veículo"
        } else {
            "Imóvel"
        },
        class,
        fields: spec
            .iter()
            .map(|s| FormField {
                key: s.key,
                label: s.label,
                help: s.help,
            })
            .collect(),
        sample: sample.into_iter().collect(),
    })
}

#[derive(serde::Serialize)]
pub struct AssetsView {
    available: bool,
    reason: Option<String>,
    mine: Vec<AssetItem>,
    counterparty_address: String,
    counterparty_balance: u64,
    counterparty_assets: Vec<AssetItem>,
    balance: u64,
    /// Trades in progress that involve my wallet (offered, or pending in the veto window).
    pending: Vec<PendingItem>,
    /// This machine's clock (unix seconds), to count down the veto window.
    now: u64,
}

/// A trade in progress (ADR-0036).
#[derive(serde::Serialize)]
pub struct PendingItem {
    class: String,
    asset: String,
    /// `seller` | `buyer`: which side is MY wallet.
    role: &'static str,
    price: u64,
    accepted: bool,
    /// Unix seconds from which anyone may finalize (0 until the buyer accepts).
    release_at: u64,
}

#[cfg_attr(feature = "dev-simulation", allow(dead_code))]
fn assets_unavailable() -> UiError {
    ui_err(
        "NotAvailable",
        "O livro de bens ainda não está na rede: esta área funcionará com ele.",
    )
}

/// Assets of the wallet and of the SIMULATED counterparty. DEVELOPMENT ONLY.
#[tauri::command]
pub async fn assets_view(address: String) -> Result<AssetsView, UiError> {
    assets_view_impl(&address)
}

#[cfg(not(feature = "dev-simulation"))]
fn assets_view_impl(_address: &str) -> Result<AssetsView, UiError> {
    Ok(AssetsView {
        available: false,
        reason: Some(assets_unavailable().message),
        mine: Vec::new(),
        counterparty_address: String::new(),
        counterparty_balance: 0,
        counterparty_assets: Vec::new(),
        balance: 0,
        pending: Vec::new(),
        now: 0,
    })
}

#[cfg(feature = "dev-simulation")]
fn assets_view_impl(address: &str) -> Result<AssetsView, UiError> {
    use teko_assets::devnet;
    let me = teko_crypto::parse_wallet_address(address)
        .map_err(|_| ui_err("InvalidArgument", "endereço inválido"))?;
    let ctx = economy_open(&resolve_dev_dir()).map_err(|e| ui_err("Io", e))?;
    let (econ, cast) = (&ctx.world, &ctx.cast);
    let cp = cast.counterparty_address();
    let items = |who: &teko_ledger::Address| {
        devnet::owned(&**econ, who)
            .into_iter()
            .map(|o| {
                let hash = teko_assets::asset(econ.ledger(), o.class.as_bytes(), o.id.as_bytes())
                    .map(|a| a.metadata_hash)
                    .unwrap_or_default();
                let record = cast.store.get_verified(&o.class, &o.id, &hash);
                let summary = record
                    .as_deref()
                    .and_then(|r| teko_crypto::asset_meta::summary_lines(&o.class, r).ok())
                    .unwrap_or_default();
                AssetItem {
                    id: o.id,
                    class: o.class,
                    version: o.version,
                    liens: o.liens,
                    summary,
                    verified: record.is_some(),
                }
            })
            .collect::<Vec<_>>()
    };
    Ok(AssetsView {
        available: true,
        reason: None,
        mine: items(&me),
        counterparty_address: teko_crypto::wallet_address(&cp).unwrap_or_default(),
        counterparty_balance: econ.balance(&cp),
        counterparty_assets: items(&cp),
        balance: econ.balance(&me),
        pending: devnet::pending_trades(&**econ, &me)
            .into_iter()
            .map(|t| PendingItem {
                role: if t.seller == me { "seller" } else { "buyer" },
                class: t.class,
                asset: t.asset,
                price: t.price,
                accepted: t.accepted,
                release_at: t.release_at,
            })
            .collect(),
        now: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
    })
}

/// The SIMULATED registrar registers a new test asset to me (or to the counterparty).
#[tauri::command]
pub async fn asset_register_test(
    address: String,
    class: String,
    fields: std::collections::HashMap<String, String>,
    for_counterparty: bool,
) -> Result<String, UiError> {
    register_impl(&address, &class, fields, for_counterparty)
}

#[cfg(not(feature = "dev-simulation"))]
fn register_impl(
    _a: &str,
    _class: &str,
    _fields: std::collections::HashMap<String, String>,
    _c: bool,
) -> Result<String, UiError> {
    Err(assets_unavailable())
}

#[cfg(feature = "dev-simulation")]
fn register_impl(
    address: &str,
    class: &str,
    fields: std::collections::HashMap<String, String>,
    for_counterparty: bool,
) -> Result<String, UiError> {
    use teko_assets::devnet;
    let me = teko_crypto::parse_wallet_address(address)
        .map_err(|_| ui_err("InvalidArgument", "endereço inválido"))?;
    let mut ctx = economy_open(&resolve_dev_dir()).map_err(|e| ui_err("Io", e))?;
    let owner = if for_counterparty {
        ctx.cast.counterparty_address()
    } else {
        me
    };
    // the rules are Rust's (teko_crypto::asset_meta): a bad field comes back as a message
    let pairs: Vec<(&str, &str)> = fields
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let record = teko_crypto::asset_meta::canonical(class, &pairs)
        .map_err(|e| ui_err("InvalidArgument", e.to_string()))?;
    devnet::register(&mut *ctx.world, &ctx.cast, &owner, class, &record)
        .map_err(|e| ui_err("Rejected", e))
}

/// The SIMULATED registrar places or releases a lien (penhora) on an asset.
#[tauri::command]
pub async fn asset_lien(class: String, asset: String, on: bool) -> Result<(), UiError> {
    lien_impl(&class, &asset, on)
}

#[cfg(not(feature = "dev-simulation"))]
fn lien_impl(_c: &str, _a: &str, _o: bool) -> Result<(), UiError> {
    Err(assets_unavailable())
}

#[cfg(feature = "dev-simulation")]
fn lien_impl(class: &str, asset: &str, on: bool) -> Result<(), UiError> {
    use teko_assets::devnet;
    let mut ctx = economy_open(&resolve_dev_dir()).map_err(|e| ui_err("Io", e))?;
    devnet::set_lien(&mut *ctx.world, &ctx.cast, class, asset, on)
        .map_err(|e| ui_err("Rejected", e))
}

/// Sells my asset to (or buys it from) the simulated counterparty (ADR-0036): MY device builds, shows
/// and signs MY OWN transaction (the offer, or my acceptance); the trade then waits in the class's veto
/// window. Returns my coin balance.
#[tauri::command]
pub async fn asset_trade(
    app: AppHandle,
    suite: State<'_, Suite>,
    class: String,
    asset: String,
    price: u64,
    selling: bool,
) -> Result<u64, UiError> {
    trade_impl(app, suite, class, asset, price, selling).await
}

#[cfg(not(feature = "dev-simulation"))]
async fn trade_impl(
    _app: AppHandle,
    _suite: State<'_, Suite>,
    _class: String,
    _asset: String,
    _price: u64,
    _selling: bool,
) -> Result<u64, UiError> {
    Err(assets_unavailable())
}

#[cfg(feature = "dev-simulation")]
async fn trade_impl(
    app: AppHandle,
    suite: State<'_, Suite>,
    class: String,
    asset: String,
    price: u64,
    selling: bool,
) -> Result<u64, UiError> {
    use teko_assets::devnet::{self, TradeAction, TradeRequest};
    let dir = resolve_dev_dir();
    with_device(app, suite.inner().clone(), move |c| {
        let v = |m: String| ClientError::Verification(m);
        let me_info = c.wallet_address()?;
        let pk = teko_crypto::hex::decode(&me_info.public_key)
            .map_err(|_| v("chave pública inválida".into()))?;
        let me = teko_crypto::ledger_address(&pk);
        let mut ctx = economy_open(&dir).map_err(v)?;
        let chain = ctx.world.chain_id().to_string();
        let mut device_error: Option<ClientError> = None;
        let result = {
            // my device is handed the metadata record: it shows plate/model and signs the record's
            // hash, and it builds and signs MY OWN transaction (offer or acceptance)
            let mut device = |r: &TradeRequest<'_>| {
                let seller =
                    teko_crypto::wallet_address(r.seller).map_err(|_| "endereço".to_string())?;
                let buyer =
                    teko_crypto::wallet_address(r.buyer).map_err(|_| "endereço".to_string())?;
                match r.action {
                    TradeAction::Offer => c.wallet_sign_asset_offer(
                        &chain,
                        r.class,
                        r.asset,
                        &buyer,
                        r.price,
                        r.version,
                        r.metadata,
                        r.window_secs,
                        r.fee,
                        r.nonce,
                    ),
                    TradeAction::Accept => c.wallet_sign_asset_accept(
                        &chain,
                        r.class,
                        r.asset,
                        &seller,
                        r.price,
                        r.version,
                        r.metadata,
                        r.window_secs,
                        r.fee,
                        r.nonce,
                    ),
                }
                .map_err(|e| {
                    let msg = e.to_string();
                    device_error = Some(e);
                    msg
                })
            };
            if selling {
                devnet::sell_to_counterparty(
                    &mut *ctx.world,
                    &ctx.cast,
                    &me,
                    &class,
                    &asset,
                    price,
                    &mut device,
                )
            } else {
                devnet::buy_from_counterparty(
                    &mut *ctx.world,
                    &ctx.cast,
                    &me,
                    &class,
                    &asset,
                    price,
                    &mut device,
                )
            }
        };
        if let Some(e) = device_error {
            return Err(e);
        }
        result.map_err(v)?;
        Ok(ctx.world.balance(&me))
    })
    .await
}

/// Completes a trade whose veto window is over (ANYONE may: here the relayer sends it).
#[tauri::command]
pub async fn asset_finalize(class: String, asset: String) -> Result<(), UiError> {
    finalize_impl(&class, &asset)
}

#[cfg(not(feature = "dev-simulation"))]
fn finalize_impl(_c: &str, _a: &str) -> Result<(), UiError> {
    Err(assets_unavailable())
}

#[cfg(feature = "dev-simulation")]
fn finalize_impl(class: &str, asset: &str) -> Result<(), UiError> {
    use teko_assets::devnet;
    let mut ctx = economy_open(&resolve_dev_dir()).map_err(|e| ui_err("Io", e))?;
    devnet::finalize_pending(&mut *ctx.world, &ctx.cast, class, asset)
        .map_err(|e| ui_err("Rejected", e))
}

/// TEST: the SIMULATED órgão (registrar quorum) vetoes a pending trade, with a public reason.
#[tauri::command]
pub async fn asset_veto(class: String, asset: String) -> Result<(), UiError> {
    veto_impl(&class, &asset)
}

#[cfg(not(feature = "dev-simulation"))]
fn veto_impl(_c: &str, _a: &str) -> Result<(), UiError> {
    Err(assets_unavailable())
}

#[cfg(feature = "dev-simulation")]
fn veto_impl(class: &str, asset: &str) -> Result<(), UiError> {
    use teko_assets::devnet;
    let mut ctx = economy_open(&resolve_dev_dir()).map_err(|e| ui_err("Io", e))?;
    devnet::veto_pending(
        &mut *ctx.world,
        &ctx.cast,
        class,
        asset,
        "veto de teste do órgão simulado",
    )
    .map_err(|e| ui_err("Rejected", e))
}

/// Staking screen data. DEVELOPMENT ONLY (local devnet economy).
#[derive(serde::Serialize)]
pub struct StakeView {
    available: bool,
    reason: Option<String>,
    balance: u64,
    my_stake: u64,
    /// (release height, amount)
    unbonding: Vec<(u64, u64)>,
    height: u64,
    unbonding_period: u64,
    slash_bps: u32,
    /// (short key, stake, is me)
    validators: Vec<(String, u64, bool)>,
    /// "node" (verified network chain) or "local" (test file).
    mode: String,
    founder_phase_active: bool,
    founder_target: u64,
    total_stake: u64,
}

#[tauri::command]
pub async fn stake_view(address: String) -> Result<StakeView, UiError> {
    stake_view_impl(&address)
}

#[cfg(not(feature = "dev-simulation"))]
fn stake_view_impl(_a: &str) -> Result<StakeView, UiError> {
    Ok(StakeView {
        available: false,
        reason: Some(
            "O livro de economia ainda não está na rede: o stake funcionará com ele.".into(),
        ),
        balance: 0,
        my_stake: 0,
        unbonding: Vec::new(),
        height: 0,
        unbonding_period: 0,
        slash_bps: 0,
        validators: Vec::new(),
        mode: String::new(),
        founder_phase_active: false,
        founder_target: 0,
        total_stake: 0,
    })
}

#[cfg(feature = "dev-simulation")]
fn stake_view_impl(address: &str) -> Result<StakeView, UiError> {
    let me = teko_crypto::parse_wallet_address(address)
        .map_err(|_| ui_err("InvalidArgument", "endereço inválido"))?;
    let ctx = economy_open(&resolve_dev_dir()).map_err(|e| ui_err("Io", e))?;
    let econ = &ctx.world;
    let l = econ.ledger();
    let cfg = teko_coin::config(l);
    let hex_short = |pk: &[u8]| {
        let h = teko_crypto::hex::encode(pk);
        format!(
            "{}…{}",
            &h[..8.min(h.len())],
            &h[h.len().saturating_sub(6)..]
        )
    };
    Ok(StakeView {
        available: true,
        reason: None,
        balance: econ.balance(&me),
        my_stake: teko_coin::stake_of(l, &me),
        unbonding: teko_coin::unbonding_of(l, &me),
        height: l.height(),
        unbonding_period: cfg.as_ref().map_or(0, |c| c.unbonding_period),
        slash_bps: cfg.as_ref().map_or(0, |c| c.slash_bps),
        mode: ctx.mode.to_string(),
        founder_phase_active: teko_coin::founder_phase_active(l),
        founder_target: cfg.as_ref().map_or(0, |c| c.founder_target),
        total_stake: teko_coin::total_stake(l),
        validators: teko_coin::validator_set(l)
            .into_iter()
            .map(|e| {
                let mine = e.owner == me;
                (hex_short(&e.key), e.stake, mine)
            })
            .collect(),
    })
}

/// Bond, unbond or claim stake. The DEVICE builds, shows and signs the transaction.
#[tauri::command]
pub async fn stake_action(
    app: AppHandle,
    suite: State<'_, Suite>,
    action: u8,
    amount: u64,
) -> Result<u64, UiError> {
    stake_action_impl(app, suite, action, amount).await
}

#[cfg(not(feature = "dev-simulation"))]
async fn stake_action_impl(
    _app: AppHandle,
    _suite: State<'_, Suite>,
    _action: u8,
    _amount: u64,
) -> Result<u64, UiError> {
    Err(ui_err(
        "NotAvailable",
        "O livro de economia ainda não está na rede: o stake funcionará com ele.",
    ))
}

#[cfg(feature = "dev-simulation")]
async fn stake_action_impl(
    app: AppHandle,
    suite: State<'_, Suite>,
    action: u8,
    amount: u64,
) -> Result<u64, UiError> {
    let dir = resolve_dev_dir();
    with_device(app, suite.inner().clone(), move |c| {
        let v = |m: String| ClientError::Verification(m);
        let info = c.wallet_address()?;
        let pk = teko_crypto::hex::decode(&info.public_key)
            .map_err(|_| v("chave pública inválida".into()))?;
        let me = teko_crypto::ledger_address(&pk);
        let mut ctx = economy_open(&dir).map_err(v)?;
        let econ = &mut ctx.world;
        let nonce = econ.nonce(&me);
        let payload = teko_crypto::ledger::coin_stake_payload(action, amount);
        // same fee rule as any transaction: price per byte over a generous envelope
        let price = teko_coin::config(econ.ledger()).map_or(0, |c| c.fee_per_byte);
        let fee = price.saturating_mul(payload.len() as u64 + pk.len() as u64 + 160);
        let have = econ.balance(&me);
        if action == 1 && have < amount.saturating_add(fee) {
            return Err(v(format!(
                "saldo insuficiente: precisa de {} (valor + taxa), tem {}",
                teko_crypto::amount::format_amount(amount.saturating_add(fee)),
                teko_crypto::amount::format_amount(have)
            )));
        }
        if action != 1 && have < fee {
            return Err(v(format!(
                "saldo insuficiente para a taxa ({})",
                teko_crypto::amount::format_amount(fee)
            )));
        }
        let chain = econ.chain_id().to_string();
        let tx = c.wallet_sign_stake(&chain, action, amount, fee, nonce)?;
        econ.submit(tx).map_err(v)?;
        Ok(econ.balance(&me))
    })
    .await
}

/// A node of the economy network, as shown by the overview.
#[derive(serde::Serialize)]
pub struct EconNodeView {
    index: usize,
    up: bool,
    height: u64,
    is_validator: bool,
    mempool: u32,
}

/// The state of the economy network. DEVELOPMENT ONLY.
#[derive(serde::Serialize)]
pub struct EconomyOverview {
    available: bool,
    reason: Option<String>,
    /// "node" or "local".
    mode: String,
    chain_id: String,
    height: u64,
    founder_phase_active: bool,
    founder_target: u64,
    total_stake: u64,
    validators: usize,
    nodes: Vec<EconNodeView>,
}

#[tauri::command]
pub async fn economy_overview() -> Result<EconomyOverview, UiError> {
    tauri::async_runtime::spawn_blocking(overview_impl)
        .await
        .map_err(|e| ui_err("Internal", e))
}

#[cfg(not(feature = "dev-simulation"))]
fn overview_impl() -> EconomyOverview {
    EconomyOverview {
        available: false,
        reason: Some("O livro de economia ainda não está na rede.".into()),
        mode: String::new(),
        chain_id: String::new(),
        height: 0,
        founder_phase_active: false,
        founder_target: 0,
        total_stake: 0,
        validators: 0,
        nodes: Vec::new(),
    }
}

#[cfg(feature = "dev-simulation")]
fn overview_impl() -> EconomyOverview {
    let dev = resolve_dev_dir();
    let net = dev.join("net");
    if net.join(teko_node::ECONOMY_FILE).exists()
        && let Ok(w) = teko_node::NodeWorld::open(&net)
    {
        let l = w.ledger_ref();
        let nodes = w
            .statuses()
            .into_iter()
            .enumerate()
            .map(|(i, s)| EconNodeView {
                index: i,
                up: s.is_some(),
                height: s.as_ref().map_or(0, |s| s.height),
                is_validator: s.as_ref().is_some_and(|s| s.is_validator),
                mempool: s.as_ref().map_or(0, |s| s.mempool),
            })
            .collect();
        return EconomyOverview {
            available: true,
            reason: None,
            mode: "node".into(),
            chain_id: w.genesis().chain_id.clone(),
            height: l.height(),
            founder_phase_active: teko_coin::founder_phase_active(l),
            founder_target: teko_coin::config(l).map_or(0, |c| c.founder_target),
            total_stake: teko_coin::total_stake(l),
            validators: teko_coin::validator_set(l).len(),
            nodes,
        };
    }
    EconomyOverview {
        available: true,
        reason: Some(
            "Sem rede de economia: usando o livro local de teste (make devnet para a rede).".into(),
        ),
        mode: "local".into(),
        chain_id: teko_coin::devnet::CHAIN_ID.into(),
        height: 0,
        founder_phase_active: false,
        founder_target: 0,
        total_stake: 0,
        validators: 0,
        nodes: Vec::new(),
    }
}

/// What the wallet screen shows as balance.
#[derive(serde::Serialize)]
pub struct BalanceInfo {
    available: bool,
    amount: u64,
    note: Option<String>,
    reason: Option<String>,
}

/// Balance of a wallet address. DEVELOPMENT ONLY: it comes from the devnet allocations
/// file (`dev/economy.txt`, made by `issuer-cli economy-fund`); the economy ledger is
/// not on the network yet (M6), so production builds report it as unavailable.
#[tauri::command]
pub async fn wallet_balance(address: String) -> Result<BalanceInfo, UiError> {
    Ok(balance_impl(&address))
}

#[cfg(feature = "dev-simulation")]
fn balance_impl(address: &str) -> BalanceInfo {
    let who =
        teko_crypto::parse_wallet_address(address).map_err(|_| "endereço inválido".to_string());
    // A running node network wins: the balance comes from its VERIFIED chain.
    if let Ok(w) = &who
        && let Ok(ctx) = economy_open(&resolve_dev_dir())
        && ctx.mode == "node"
    {
        return BalanceInfo {
            available: true,
            amount: ctx.world.balance(w),
            note: Some(
                "Moedas de TESTE da rede local (consenso BFT, verificado aqui). Fundador: as recompensas de bloco chegam à carteira do fundador.".into(),
            ),
            reason: None,
        };
    }
    match who.and_then(|w| teko_assets::devnet::balance_of(&resolve_dev_dir(), &w)) {
        Ok(Some(amount)) => BalanceInfo {
            available: true,
            amount,
            note: Some(
                "Alocação de TESTE da devnet local (dev/economy.txt). Para ganhar moedas de teste: make fund ADDR=<seu endereço> AMOUNT=1000.".into(),
            ),
            reason: None,
        },
        Ok(None) => BalanceInfo {
            available: false,
            amount: 0,
            note: None,
            reason: Some(
                "Nenhuma moeda de teste ainda. Rode: make fund ADDR=<seu endereço> AMOUNT=1000".into(),
            ),
        },
        Err(e) => BalanceInfo {
            available: false,
            amount: 0,
            note: None,
            reason: Some(e),
        },
    }
}

#[cfg(not(feature = "dev-simulation"))]
fn balance_impl(_address: &str) -> BalanceInfo {
    BalanceInfo {
        available: false,
        amount: 0,
        note: None,
        reason: Some(
            "O livro de economia ainda não está na rede: o saldo aparecerá quando ele estiver."
                .into(),
        ),
    }
}

#[tauri::command]
pub async fn wallet_address(
    app: AppHandle,
    suite: State<'_, Suite>,
) -> Result<teko_client::AddressInfo, UiError> {
    with_device(app, suite.inner().clone(), |c| c.wallet_address()).await
}

#[tauri::command]
pub async fn civic_id(app: AppHandle, suite: State<'_, Suite>) -> Result<CivicIdInfo, UiError> {
    with_device(app, suite.inner().clone(), |c| c.civic_id()).await
}

// ───────────────────────── civic credential (ADR-0038) ─────────────────────────

/// The civic credentials of the (verified) economy chain, by id: subject, commitment, expiry and
/// whether each is ACTIVE now (past the contest window, not vetoed, revoked or expired).
/// Credentials are issued on the chain by the organ's app, so this is where the Suite learns
/// whether the citizen's credential is worth anything.
fn civic_snapshot() -> Result<std::collections::HashMap<Vec<u8>, teko_client::CivicRecord>, UiError>
{
    let net = resolve_dev_dir().join("net");
    if !net.join(teko_node::ECONOMY_FILE).exists() {
        return Err(ui_err(
            "NoNetwork",
            "Sem rede: a credencial cívica vive na chain. Rode make devnet.",
        ));
    }
    let world = teko_node::NodeWorld::open(&net)
        .map_err(|e| ui_err("NoNetwork", format!("rede indisponível: {e}")))?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    Ok(
        teko_credentials::credentials_of_type(world.ledger_ref(), b"civic")
            .into_iter()
            .map(|(id, rec)| {
                (
                    id,
                    teko_client::CivicRecord {
                        subject: rec.subject.clone(),
                        commitment: rec.commitment.clone(),
                        valid_until: rec.valid_until,
                        active: rec.status_at(now) == teko_credentials::Status::Active,
                    },
                )
            })
            .collect(),
    )
}

#[derive(Serialize)]
pub struct CivicRequestInfo {
    civic_id: String,
    /// Where the request file was written (hand it to the organ).
    path: String,
    /// The same content, to copy.
    json: String,
}

/// The citizen asks the organ for the civic credential: the device (after the confirmation)
/// proves it holds the identity key and is genuine. The Suite only writes the request down.
#[tauri::command]
pub async fn civic_request(
    app: AppHandle,
    suite: State<'_, Suite>,
) -> Result<CivicRequestInfo, UiError> {
    use rand::Rng;
    let mut nonce = [0u8; 32];
    rand::rand_core::UnwrapErr(rand::rngs::SysRng).fill_bytes(&mut nonce);
    let (id, req) = with_device(app, suite.inner().clone(), move |c| {
        let id = c.civic_id()?;
        let req = c.identity_request(&nonce, "Órgão emissor")?;
        Ok((id, req))
    })
    .await?;
    let json = serde_json::json!({
        "civic_id": id.id,
        "nonce": hex::encode(&nonce),
        "request": hex::encode(&prost::Message::encode_to_vec(&req)),
    })
    .to_string();
    let dir = resolve_dev_dir().join("requests");
    fs::create_dir_all(&dir).map_err(|e| ui_err("Io", e))?;
    let path = dir.join(format!("civic-request-{}.json", &hex::encode(&nonce)[..8]));
    fs::write(&path, &json).map_err(|e| ui_err("Io", e))?;
    Ok(CivicRequestInfo {
        civic_id: id.id,
        path: path.display().to_string(),
        json,
    })
}

#[derive(Serialize)]
pub struct CivicStatusInfo {
    /// `none`, `installed`.
    state: &'static str,
    /// What the chain says: `missing`, `proposed`, `pending`, `active`, `expired`, `vetoed`, `revoked`.
    chain: &'static str,
    active_at: u64,
    valid_until: u64,
    now: u64,
}

/// Installs the credential the organ handed over: the first bundle waiting in `dev/inbox/` that is
/// for THIS device (the device checks the organ's signatures and the holder itself).
#[tauri::command]
pub async fn civic_import(app: AppHandle, suite: State<'_, Suite>) -> Result<String, UiError> {
    let inbox = resolve_dev_dir().join("inbox");
    let mut files: Vec<_> = fs::read_dir(&inbox)
        .map_err(|_| {
            ui_err(
                "NoBundle",
                "Nenhum pacote do órgão em dev/inbox. Peça a emissão no app Teko Emissor.",
            )
        })?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "bundle"))
        .collect();
    files.sort();
    if files.is_empty() {
        return Err(ui_err(
            "NoBundle",
            "Nenhum pacote do órgão em dev/inbox. Peça a emissão no app Teko Emissor.",
        ));
    }
    let bundles: Vec<(String, Vec<u8>)> = files
        .iter()
        .filter_map(|p| Some((p.display().to_string(), fs::read(p).ok()?)))
        .collect();
    with_device(app, suite.inner().clone(), move |c| {
        let mut last: Option<ClientError> = None;
        for (name, bytes) in bundles {
            check_size_client(&bytes)?;
            match c.load_credential(&bytes) {
                Ok(()) => return Ok(name),
                // not for this device or not trusted: try the next one; the user refusing stops here
                Err(ClientError::Device(f)) if f.code == teko_protocol::ErrorCode::UserRejected => {
                    return Err(ClientError::Device(f));
                }
                Err(e) => last = Some(e),
            }
        }
        Err(last.unwrap_or_else(|| {
            ClientError::Unexpected("nenhum pacote serve a este dispositivo".into())
        }))
    })
    .await
}

fn check_size_client(bytes: &[u8]) -> Result<(), ClientError> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(ClientError::Unexpected("pacote grande demais".into()));
    }
    Ok(())
}

/// What the chain says about the credential installed on the device (read-only).
#[tauri::command]
pub async fn civic_status() -> Result<CivicStatusInfo, UiError> {
    tauri::async_runtime::spawn_blocking(|| {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let mut info = CivicStatusInfo {
            state: "installed",
            chain: "missing",
            active_at: 0,
            valid_until: 0,
            now,
        };
        let net = resolve_dev_dir().join("net");
        let world = teko_node::NodeWorld::open(&net).map_err(|e| ui_err("NoNetwork", e))?;
        // the credential installed on the device is not exposed to the Suite: the chain is searched
        // for the bundle the organ handed over (the one in the inbox that matches)
        let inbox = resolve_dev_dir().join("inbox");
        let ledger = world.ledger_ref();
        let mut best: Option<teko_credentials::CredRec> = None;
        for entry in fs::read_dir(&inbox)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
        {
            let Ok(bytes) = fs::read(entry.path()) else {
                continue;
            };
            let Ok(bundle) =
                <teko_crypto::civic::CivicBundle as prost::Message>::decode(bytes.as_slice())
            else {
                continue;
            };
            for (_, rec) in teko_credentials::credentials_of_type(ledger, b"civic") {
                let salt: [u8; 16] = bundle.salt.as_slice().try_into().unwrap_or([0; 16]);
                let c = teko_crypto::civic::commitment_of(&teko_crypto::civic::claim_list(
                    &salt,
                    &bundle.claims,
                ));
                if rec.commitment == c && rec.valid_until == bundle.valid_until {
                    best = Some(rec);
                }
            }
        }
        if let Some(rec) = best {
            info.chain = match rec.status_at(now) {
                teko_credentials::Status::Proposed => "proposed",
                teko_credentials::Status::Pending => "pending",
                teko_credentials::Status::Active => "active",
                teko_credentials::Status::Expired => "expired",
                teko_credentials::Status::Vetoed => "vetoed",
                teko_credentials::Status::Revoked => "revoked",
                teko_credentials::Status::Superseded => "superseded",
            };
            info.active_at = rec.active_at;
            info.valid_until = rec.valid_until;
        }
        Ok(info)
    })
    .await
    .map_err(|e| ui_err("Internal", e))?
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
    let snap = civic_snapshot()?;
    with_device(app, suite.inner().clone(), move |c| {
        let lookup = |id: &[u8]| snap.get(id).cloned();
        let mut out = Vec::new();
        for field in ["name", "birth_date", "nationality", "registry_id"] {
            let r = c.prove_attribute(reveal(field), "Teko Suite (ver meus dados)", &lookup)?;
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
    let snap = civic_snapshot()?;
    with_device(app, suite.inner().clone(), move |c| {
        c.prove_attribute(predicate, &verifier, &|id: &[u8]| snap.get(id).cloned())
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
    let snap = civic_snapshot().map_err(|e| ClientError::Unexpected(e.message))?;
    Issuer::open(resolve_dev_dir())
        .sign_registration(election_id, contest_id, req, &|id: &[u8]| {
            snap.get(id).cloned()
        })
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
    /// Board entries verified so far and in total: while `verified < total` the numbers
    /// are those of the verified part.
    verified: u64,
    total: u64,
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
        verified: 0,
        total: 0,
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
        let walk = match walk_board(&suite, &admin, BOARD_STEP) {
            Ok(w) => w,
            Err(e) => return unavailable(&e),
        };
        let (t, manifest, board) = (walk.snap, walk.manifest, walk.board);
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
            verified: walk.verified,
            total: board.len(),
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
    /// Board entries verified so far and in total (the screen shows a progress bar
    /// while `verified < total`).
    verified: u64,
    total: u64,
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
        verified: 0,
        total: 0,
    }
}

/// Everything PUBLIC about the election, straight from the bulletin board: rules,
/// candidates, calendar and the signed governance decisions in log order. It
/// replays the log itself (M-of-N checked), so nothing here is taken on trust.
#[tauri::command]
pub async fn explorer_snapshot(suite: State<'_, Suite>) -> Result<ExplorerReply, UiError> {
    let suite = suite.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let admin = match Admin::open(resolve_dev_dir()) {
            Ok(a) => a,
            Err(e) => return explorer_unavailable(&e.to_string()),
        };
        // the SAME verified tally as the live results (not a replay of the whole board)
        let walk = match walk_board(&suite, &admin, BOARD_STEP) {
            Ok(w) => w,
            Err(e) => return explorer_unavailable(&e),
        };
        let (board, snap, verified) = (walk.board, walk.snap, walk.verified);
        let Some(m) = walk.manifest else {
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
            ceremony: {
                // Every ceremony on the chain is re-verified HERE (proofs of possession,
                // key recomputed). Prefer the one whose key is the manifest's; if none
                // matches, show the first one flagged as different (results cannot be
                // decrypted in that case).
                let mut views: Vec<CeremonyView> = (0..board.len())
                    .filter_map(|i| board.entry(i))
                    .filter(|e| e.kind == teko_bulletin::EntryKind::DKG)
                    .filter_map(|e| {
                        let rec =
                            <teko_ballot_crypto::dkg_wire::DkgRecord as prost::Message>::decode(
                                e.payload.as_slice(),
                            )
                            .ok()?;
                        let v = rec.verify().ok()?;
                        Some(CeremonyView {
                            threshold: v.threshold,
                            validators: v.verification_keys.len(),
                            matches_manifest: m_tally_key == rec.group_key,
                            election_key: hex::encode(&rec.group_key),
                        })
                    })
                    .collect();
                match views.iter().position(|v| v.matches_manifest) {
                    Some(i) => Some(views.swap_remove(i)),
                    None => views.into_iter().next(),
                }
            },
            verified,
            total: board.len(),
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
                            .filter_map(|d| {
                                let b = d.block().ok()?;
                                let (precommits, round) = d
                                    .commit
                                    .as_ref()
                                    .map_or((0, 0), |c| (c.precommits.len(), c.round));
                                Some(BlockView {
                                    height: b.height,
                                    entries: b.entries.len(),
                                    // BFT: precommits in the commit certificate (ADR-0034)
                                    signatures: precommits,
                                    round,
                                    timestamp: b.timestamp,
                                    proposer: validators
                                        .iter()
                                        .position(|k| *k == b.proposer)
                                        .unwrap_or(0),
                                    hash: hex::encode(&b.id()),
                                })
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

// ───────────────────────── credentials on the chain (ADR-0037) ─────────────────────────

#[derive(Serialize)]
pub struct CredTypeView {
    ctype: String,
    public: bool,
    contest_secs: u64,
    validity_secs: u64,
    /// The credential key exported on this session (hex), if any.
    key: Option<String>,
}

#[derive(Serialize)]
pub struct MyCredView {
    ctype: String,
    id: String,
    status: &'static str,
    claims: Option<String>,
    valid_until: u64,
    active_at: u64,
    reason: String,
}

#[derive(Serialize)]
pub struct CredsView {
    available: bool,
    reason: String,
    chain_id: String,
    now: u64,
    types: Vec<CredTypeView>,
    mine: Vec<MyCredView>,
}

fn status_name(s: teko_credentials::Status) -> &'static str {
    use teko_credentials::Status;
    match s {
        Status::Proposed => "proposed",
        Status::Pending => "pending",
        Status::Active => "active",
        Status::Expired => "expired",
        Status::Vetoed => "vetoed",
        Status::Revoked => "revoked",
        Status::Superseded => "superseded",
    }
}

/// What the chain says about the credentials issued to the keys this session exported. READ-ONLY:
/// credentials are issued by the organs in the separate issuer app, never by this Suite.
#[tauri::command]
pub async fn credentials_view(suite: State<'_, Suite>) -> Result<CredsView, UiError> {
    let suite = suite.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut view = CredsView {
            available: false,
            reason: String::new(),
            chain_id: String::new(),
            now: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
            types: Vec::new(),
            mine: Vec::new(),
        };
        let net = resolve_dev_dir().join("net");
        if !net.join(teko_node::ECONOMY_FILE).exists() {
            view.reason = "Sem rede: rode make devnet (as credenciais ficam na chain).".into();
            return Ok(view);
        }
        let world = match teko_node::NodeWorld::open(&net) {
            Ok(w) => w,
            Err(e) => {
                view.reason = format!("rede indisponível: {e}");
                return Ok(view);
            }
        };
        let mut keys = load_cred_keys();
        keys.extend(
            suite
                .cred_keys
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone(),
        );
        let ledger = world.ledger_ref();
        view.available = true;
        view.chain_id = world.genesis().chain_id.clone();
        for (name, t) in teko_credentials::types(ledger) {
            let ctype = String::from_utf8_lossy(&name).to_string();
            // the elected credential is issued to the winner's CANDIDACY key (ADR-0040)
            let key = keys
                .get(if ctype == "eleito" {
                    "candidato"
                } else {
                    &ctype
                })
                .cloned();
            if let Some(k) = key.as_deref().and_then(|k| hex::decode(k).ok()) {
                for (id, rec) in teko_credentials::credentials_of_type(ledger, &name) {
                    if rec.subject != k {
                        continue;
                    }
                    view.mine.push(MyCredView {
                        ctype: ctype.clone(),
                        id: hex::encode(&id),
                        status: status_name(rec.status_at(view.now)),
                        claims: if t.public {
                            String::from_utf8(rec.claims.clone()).ok()
                        } else {
                            recovered_claims(&id)
                        },
                        valid_until: rec.valid_until,
                        active_at: rec.active_at,
                        reason: rec.reason.clone(),
                    });
                }
            }
            view.types.push(CredTypeView {
                ctype,
                public: t.public,
                contest_secs: t.contest_secs,
                validity_secs: t.validity_secs,
                key,
            });
        }
        Ok(view)
    })
    .await
    .map_err(|e| ui_err("Internal", e))?
}

/// Credential keys are PUBLIC, so the Suite remembers them on disk (`dev/cred-keys.json`): the
/// device re-derives them from the wallet at any time, this only saves a confirmation.
fn load_cred_keys() -> std::collections::BTreeMap<String, String> {
    fs::read_to_string(resolve_dev_dir().join("cred-keys.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_cred_keys(keys: &std::collections::BTreeMap<String, String>) {
    if let Ok(json) = serde_json::to_string_pretty(keys) {
        let _ = fs::write(resolve_dev_dir().join("cred-keys.json"), json);
    }
}

/// The holder file of a recovered private credential (`dev/recovered/<id>.json`).
fn recovered_claims(id: &[u8]) -> Option<String> {
    let t = fs::read_to_string(
        resolve_dev_dir()
            .join("recovered")
            .join(format!("{}.json", hex::encode(id))),
    )
    .ok()?;
    let v: serde_json::Value = serde_json::from_str(&t).ok()?;
    v.get("claims")?.as_str().map(str::to_string)
}

/// The PUBLIC credential key of this holder for one type, confirmed on the device. The holder
/// hands it to the issuing organ; it is unlinkable to the identity, wallet and voter keys.
#[tauri::command]
pub async fn credential_key(
    app: AppHandle,
    suite: State<'_, Suite>,
    ctype: String,
) -> Result<String, UiError> {
    let inner = suite.inner().clone();
    let t2 = ctype.clone();
    let key = with_device(app, inner.clone(), move |c| c.credential_key(&t2)).await?;
    let key = hex::encode(&key);
    let mut keys = inner.cred_keys.lock().unwrap_or_else(|e| e.into_inner());
    keys.insert(ctype, key.clone());
    save_cred_keys(&keys);
    Ok(key)
}

/// A private credential to open on the device: type, id, holder key, sealed blob, commitment.
type ToOpen = (String, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>);

#[derive(Serialize)]
pub struct RecoveredCred {
    ctype: String,
    id: String,
    status: &'static str,
    claims: Option<String>,
    valid_until: u64,
}

#[derive(Serialize)]
pub struct Recovery {
    address: String,
    balance: u64,
    stake: u64,
    assets: usize,
    credentials: Vec<RecoveredCred>,
    /// Private credentials whose attributes could not be opened (no sealed copy on the chain).
    unopened: usize,
}

/// "Recover everything" after a wallet restore (ADR-0039): everything that belongs to the wallet is
/// on the chain, so the Suite reads it back, verified. The credential keys come from the WALLET
/// (one confirmation), the chain is searched for what was issued to them, and the attributes
/// sealed to those keys are opened ON THE DEVICE (one confirmation each). The identity (civic ID)
/// and the voter registration are NOT recovered here: they never come from the seed.
#[tauri::command]
pub async fn recover_all(app: AppHandle, suite: State<'_, Suite>) -> Result<Recovery, UiError> {
    let inner = suite.inner().clone();
    let net = resolve_dev_dir().join("net");
    if !net.join(teko_node::ECONOMY_FILE).exists() {
        return Err(ui_err("NoNetwork", "Sem rede: rode make devnet."));
    }
    let types: Vec<(String, bool)> = {
        let world = teko_node::NodeWorld::open(&net)
            .map_err(|e| ui_err("NoNetwork", format!("rede indisponível: {e}")))?;
        teko_credentials::types(world.ledger_ref())
            .into_iter()
            .map(|(n, t)| (String::from_utf8_lossy(&n).to_string(), t.public))
            .filter(|(n, _)| n != "civic")
            .collect()
    };
    // 1) the wallet and its credential keys: the device derives them from the seed
    let ctypes: Vec<String> = types
        .iter()
        .filter(|(n, _)| n != "eleito")
        .map(|(n, _)| n.clone())
        .collect();
    let (addr, keys) = with_device(app.clone(), inner.clone(), move |c| {
        let addr = c.wallet_address()?;
        let keys = if ctypes.is_empty() {
            Vec::new()
        } else {
            c.credential_keys(&ctypes)?
        };
        Ok((addr, keys))
    })
    .await?;
    {
        let mut mem = inner.cred_keys.lock().unwrap_or_else(|e| e.into_inner());
        for (t, k) in &keys {
            mem.insert(t.clone(), hex::encode(k));
        }
        save_cred_keys(&mem);
    }
    // 2) the chain (verified): what belongs to the wallet, and what was issued to its keys
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let me = teko_crypto::parse_wallet_address(&addr.address)
        .map_err(|_| ui_err("Internal", "endereço da carteira"))?;
    let mut rec = Recovery {
        address: addr.address.clone(),
        balance: 0,
        stake: 0,
        assets: 0,
        credentials: Vec::new(),
        unopened: 0,
    };
    // (type, id, subject, sealed, commitment) of the private ones still to open
    let mut to_open: Vec<ToOpen> = Vec::new();
    {
        let world = teko_node::NodeWorld::open(&net)
            .map_err(|e| ui_err("NoNetwork", format!("rede indisponível: {e}")))?;
        let ledger = world.ledger_ref();
        rec.balance = teko_coin::balance(ledger, &me);
        rec.stake = teko_coin::stake_of(ledger, &me);
        rec.assets = teko_assets::assets_of(ledger, &me).len();
        for (ctype, key) in &keys {
            for (t, id) in teko_credentials::credentials_of(ledger, key) {
                let Some(cr) = teko_credentials::credential(ledger, &t, &id) else {
                    continue;
                };
                // what the wallet's key holds: its own type, and the `eleito` credential a candidate
                // key receives when it wins (ADR-0040)
                let held = String::from_utf8_lossy(&t).to_string();
                if held != *ctype && !(ctype == "candidato" && held == "eleito") {
                    continue;
                }
                let public = types.iter().any(|(n, p)| *n == held && *p);
                if !public && recovered_claims(&id).is_none() {
                    if cr.sealed.is_empty() {
                        rec.unopened += 1;
                    } else {
                        to_open.push((
                            held.clone(),
                            id.clone(),
                            key.clone(),
                            cr.sealed.clone(),
                            cr.commitment.clone(),
                        ));
                    }
                }
                rec.credentials.push(RecoveredCred {
                    ctype: held.clone(),
                    id: hex::encode(&id),
                    status: status_name(cr.status_at(now)),
                    claims: if public {
                        String::from_utf8(cr.claims.clone()).ok()
                    } else {
                        None
                    },
                    valid_until: cr.valid_until,
                });
            }
        }
    }
    // 3) the private attributes: sealed to the wallet's keys, opened ON THE DEVICE
    if !to_open.is_empty() {
        let dir = resolve_dev_dir().join("recovered");
        let opened = with_device(app, inner, move |c| {
            let mut out = Vec::new();
            for (ct, id, key, sealed, commitment) in to_open {
                // a refusal on the device skips just this one (the user may decline)
                if let Ok((claims, salt)) =
                    c.open_credential_bundle(&ct, &key, &sealed, &commitment)
                {
                    out.push((ct, id, claims, salt));
                }
            }
            Ok(out)
        })
        .await?;
        fs::create_dir_all(&dir).map_err(|e| ui_err("Io", e))?;
        for (ct, id, claims, salt) in opened {
            let json = serde_json::json!({
                "ctype": ct,
                "id": hex::encode(&id),
                "claims": String::from_utf8_lossy(&claims),
                "salt": hex::encode(&salt),
            });
            let _ = fs::write(
                dir.join(format!("{}.json", hex::encode(&id))),
                json.to_string(),
            );
        }
    }
    for c in &mut rec.credentials {
        if c.claims.is_none()
            && let Ok(id) = hex::decode(&c.id)
        {
            c.claims = recovered_claims(&id);
            if c.claims.is_none() {
                rec.unopened += 1;
            }
        }
    }
    Ok(rec)
}

// ───────────────────────── candidacies on the chain (ADR-0040) ─────────────────────────

#[derive(Serialize)]
pub struct OptionCheck {
    index: u32,
    /// `none` (no credential bound), `verified`, `mismatch`, `pending`, `expired`, `revoked`, `unknown`.
    state: &'static str,
}

#[derive(Serialize)]
pub struct ContestChecks {
    contest_id: String,
    options: Vec<OptionCheck>,
}

/// Checks each candidate of the election against the chain: its candidacy credential exists, is
/// ACTIVE and its claims (name, number, office, jurisdiction, election) are what the signed
/// manifest says. The manifest is verified (governance M-of-N) here; a contest already signed
/// cannot change, so a candidacy revoked later shows up here as a warning.
#[tauri::command]
pub async fn candidate_checks() -> Result<Vec<ContestChecks>, UiError> {
    tauri::async_runtime::spawn_blocking(|| {
        use prost::Message as _;
        use teko_credentials::candidate::{Cargo, parse_candidate};
        let trust = load_trust()?;
        let policy =
            teko_election::GovernancePolicy::new(trust.gov_signers.clone(), trust.gov_threshold)
                .map_err(|_| ui_err("NoElection", "sem governança no trust.txt"))?;
        let bundle = teko_election::ElectionBundle::decode(read_bundle()?.as_slice())
            .map_err(|_| ui_err("NoElection", "pacote de eleição malformado"))?;
        let manifest = teko_election::ElectionManifest::decode(bundle.manifest.as_slice())
            .map_err(|_| ui_err("NoElection", "manifesto malformado"))?;
        teko_election::verify_manifest(&policy, &manifest)
            .map_err(|e| ui_err("NoElection", format!("manifesto recusado: {e}")))?;
        let net = resolve_dev_dir().join("net");
        if !net.join(teko_node::ECONOMY_FILE).exists() {
            return Ok(Vec::new());
        }
        let world = teko_node::NodeWorld::open(&net)
            .map_err(|e| ui_err("NoNetwork", format!("rede indisponível: {e}")))?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let ledger = world.ledger_ref();
        let mut out = Vec::new();
        for c in &manifest.contests {
            let scope = c.scope.clone().unwrap_or_default();
            let cargo = match c.office() {
                teko_election::Office::President => Some(Cargo::Presidente),
                teko_election::Office::Governor => Some(Cargo::Governador),
                teko_election::Office::Mayor => Some(Cargo::Prefeito),
                _ => None,
            };
            let options = c
                .options
                .iter()
                .map(|o| {
                    let state = if o.credential_id.is_empty() {
                        "none"
                    } else if let Some(rec) =
                        teko_credentials::credential(ledger, b"candidato", &o.credential_id)
                    {
                        match rec.status_at(now) {
                            teko_credentials::Status::Active => {
                                let ok = parse_candidate(&rec.claims).is_some_and(|k| {
                                    Some(k.cargo) == cargo
                                        && k.scope_code() == scope.code
                                        && k.eleicao == manifest.election_id
                                        && k.nome == o.name
                                        && k.numero == o.number
                                });
                                if ok { "verified" } else { "mismatch" }
                            }
                            teko_credentials::Status::Proposed
                            | teko_credentials::Status::Pending => "pending",
                            teko_credentials::Status::Expired => "expired",
                            _ => "revoked",
                        }
                    } else {
                        "unknown"
                    };
                    OptionCheck {
                        index: o.index,
                        state,
                    }
                })
                .collect();
            out.push(ContestChecks {
                contest_id: c.contest_id.clone(),
                options,
            });
        }
        Ok(out)
    })
    .await
    .map_err(|e| ui_err("Internal", e))?
}
