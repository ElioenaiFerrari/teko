use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use std::time::Duration;

use rand::Rng;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use teko_assets::devnet::World;
use teko_client::{
    ClientError, CredentialOpArgs, CredentialOpKind, DeviceClient, DeviceStatus, resolve_dev_dir,
};
use teko_credentials::{CredRec, Status, TypeRec};
use teko_crypto::hex;
use teko_ledger::{Tx, address_of};
use teko_protocol::ButtonKind;
use teko_transport::{TcpTransport, connect_tcp};

type Client = DeviceClient<TcpTransport>;

pub struct IssuerInner {
    pub client: Mutex<Option<Client>>,
    pub addr: String,
}

pub type Issuer = Arc<IssuerInner>;

/// Error shape the UI receives (`invoke` rejects with this object).
#[derive(Debug, Serialize)]
pub struct UiError {
    code: String,
    message: String,
}

fn ui_err(code: &str, message: impl ToString) -> UiError {
    UiError {
        code: code.to_string(),
        message: message.to_string(),
    }
}

impl From<ClientError> for UiError {
    fn from(e: ClientError) -> Self {
        ui_err(&e.code(), &e)
    }
}

fn prompt_name(kind: ButtonKind) -> &'static str {
    match kind {
        ButtonKind::CredentialKey => "credential_key",
        ButtonKind::SignCredentialOp => "sign_credential_op",
        ButtonKind::PinEntry => "pin_entry",
        _ => "other",
    }
}

fn lock(inner: &IssuerInner) -> MutexGuard<'_, Option<Client>> {
    inner.client.lock().unwrap_or_else(|e| e.into_inner())
}

fn ensure_connected(
    inner: &IssuerInner,
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

/// Runs `f` against the device on a blocking thread (it may wait for the user to act on the
/// device). A transport failure drops the connection.
async fn with_device<T, F>(app: AppHandle, issuer: Issuer, f: F) -> Result<T, UiError>
where
    T: Send + 'static,
    F: FnOnce(&mut Client) -> Result<T, UiError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || {
        let mut guard = lock(&issuer);
        ensure_connected(&issuer, &app, &mut guard)?;
        let client = guard
            .as_mut()
            .ok_or_else(|| ui_err("Internal", "sem conexão"))?;
        let result = f(client);
        if matches!(&result, Err(e) if e.code == "Transport") {
            *guard = None;
        }
        result
    })
    .await
    .map_err(|e| ui_err("Internal", e))?
}

fn net_dir() -> PathBuf {
    resolve_dev_dir().join("net")
}

fn issued_dir() -> PathBuf {
    resolve_dev_dir().join("issued")
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn open_world() -> Result<teko_node::NodeWorld, UiError> {
    let net = net_dir();
    if !net.join(teko_node::ECONOMY_FILE).exists() {
        return Err(ui_err(
            "NoNetwork",
            "Não encontrei a rede de teste (dev/net). Rode: make devnet FOUNDER=<sua carteira>",
        ));
    }
    teko_node::NodeWorld::open(&net)
        .map_err(|e| ui_err("NoNetwork", format!("rede indisponível: {e}")))
}

// ───────────────────────── status ─────────────────────────

#[derive(Serialize)]
pub struct StatusReply {
    connected: bool,
    busy: bool,
    status: Option<DeviceStatus>,
    error: Option<String>,
    addr: String,
}

#[tauri::command]
pub async fn device_status(
    app: AppHandle,
    issuer: State<'_, Issuer>,
) -> Result<StatusReply, UiError> {
    let issuer = issuer.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut reply = StatusReply {
            connected: false,
            busy: false,
            status: None,
            error: None,
            addr: issuer.addr.clone(),
        };
        // an operation in flight (waiting for the user on the device): do not queue behind it
        let mut guard = match issuer.client.try_lock() {
            Ok(g) => g,
            Err(TryLockError::WouldBlock) => {
                reply.busy = true;
                return reply;
            }
            Err(TryLockError::Poisoned(p)) => p.into_inner(),
        };
        match ensure_connected(&issuer, &app, &mut guard) {
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
            Err(e) => reply.error = Some(e.to_string()),
        }
        reply
    })
    .await
    .map_err(|e| ui_err("Internal", e))
}

/// The PIN is typed ON THE DEVICE; this only asks it to prompt.
#[tauri::command]
pub async fn unlock(
    app: AppHandle,
    issuer: State<'_, Issuer>,
) -> Result<teko_client::UnlockOutcome, UiError> {
    with_device(app, issuer.inner().clone(), |c| {
        c.unlock().map_err(UiError::from)
    })
    .await
}

// ───────────────────────── the chain's view ─────────────────────────

#[derive(Serialize)]
pub struct CredView {
    id: String,
    ctype: String,
    subject: String,
    status: &'static str,
    approvals: usize,
    threshold: u32,
    approved_by_me: bool,
    valid_until: u64,
    active_at: u64,
    reason: String,
    /// The claims in clear (public types, or private ones this app issued).
    claims: Option<String>,
    can_approve: bool,
    can_veto: bool,
    can_revoke: bool,
}

#[derive(Serialize)]
pub struct OrgView {
    ctype: String,
    public: bool,
    threshold: u32,
    members: usize,
    contest_secs: u64,
    validity_secs: u64,
    is_member: bool,
    credentials: Vec<CredView>,
}

#[derive(Serialize)]
pub struct Overview {
    chain_id: String,
    now: u64,
    /// The `tekow1…` of the device wallet (empty while the device is locked or has no wallet).
    wallet: String,
    wallet_note: String,
    orgs: Vec<OrgView>,
}

fn status_name(s: Status) -> &'static str {
    match s {
        Status::Proposed => "proposed",
        Status::Pending => "pending",
        Status::Active => "active",
        Status::Expired => "expired",
        Status::Vetoed => "vetoed",
        Status::Revoked => "revoked",
    }
}

/// The off-chain bundle the issuer keeps and hands to the holder: the claims and the salt of a
/// PRIVATE credential (the chain only has the commitment).
#[derive(Serialize, Deserialize)]
struct Bundle {
    chain_id: String,
    ctype: String,
    id: String,
    subject: String,
    claims: String,
    salt: String,
    valid_until: u64,
}

fn load_bundle(id: &[u8], rec: &CredRec) -> Option<(Vec<u8>, [u8; 16])> {
    let text = fs::read_to_string(issued_dir().join(format!("{}.json", hex::encode(id)))).ok()?;
    let b: Bundle = serde_json::from_str(&text).ok()?;
    let salt: [u8; 16] = hex::decode(&b.salt).ok()?.try_into().ok()?;
    let claims = b.claims.into_bytes();
    // only a bundle that matches the commitment on the chain counts
    (teko_crypto::ledger::cred_private_commitment(&salt, &claims).as_slice() == rec.commitment)
        .then_some((claims, salt))
}

#[tauri::command]
pub async fn overview(app: AppHandle, issuer: State<'_, Issuer>) -> Result<Overview, UiError> {
    let issuer = issuer.inner().clone();
    // the wallet address needs the unlocked device; without it the view is read-only
    let (wallet, wallet_note) =
        match with_device(app, issuer, |c| c.wallet_address().map_err(UiError::from)).await {
            Ok(a) => (a.address, String::new()),
            Err(e) => (String::new(), e.message),
        };
    tauri::async_runtime::spawn_blocking(move || {
        let world = open_world()?;
        let me = teko_crypto::parse_wallet_address(&wallet).ok();
        let now = now_secs();
        let ledger = World::ledger(&world);
        let mut orgs = Vec::new();
        for (name, t) in teko_credentials::types(ledger) {
            let ctype = String::from_utf8_lossy(&name).to_string();
            let is_member = me.is_some_and(|a| t.members.iter().any(|m| m.as_slice() == a));
            let credentials = teko_credentials::credentials_of_type(ledger, &name)
                .into_iter()
                .map(|(id, rec)| cred_view(&ctype, &t, &id, &rec, me, now))
                .collect();
            orgs.push(OrgView {
                ctype,
                public: t.public,
                threshold: t.threshold,
                members: t.members.len(),
                contest_secs: t.contest_secs,
                validity_secs: t.validity_secs,
                is_member,
                credentials,
            });
        }
        Ok(Overview {
            chain_id: world.chain_id().to_string(),
            now,
            wallet,
            wallet_note,
            orgs,
        })
    })
    .await
    .map_err(|e| ui_err("Internal", e))?
}

fn cred_view(
    ctype: &str,
    t: &TypeRec,
    id: &[u8],
    rec: &CredRec,
    me: Option<[u8; 32]>,
    now: u64,
) -> CredView {
    let status = rec.status_at(now);
    let is_member = me.is_some_and(|a| t.members.iter().any(|m| m.as_slice() == a));
    let mine = |votes: &[Vec<u8>]| me.is_some_and(|a| votes.iter().any(|v| v.as_slice() == a));
    let claims = if t.public {
        String::from_utf8(rec.claims.clone()).ok()
    } else {
        load_bundle(id, rec).and_then(|(c, _)| String::from_utf8(c).ok())
    };
    CredView {
        id: hex::encode(id),
        ctype: ctype.to_string(),
        subject: hex::encode(&rec.subject),
        status: status_name(status),
        approvals: rec.approvals.len(),
        threshold: t.threshold,
        approved_by_me: mine(&rec.approvals),
        valid_until: rec.valid_until,
        active_at: rec.active_at,
        reason: rec.reason.clone(),
        claims,
        can_approve: is_member && status == Status::Proposed && !mine(&rec.approvals),
        can_veto: is_member && status == Status::Pending && !mine(&rec.veto_votes),
        can_revoke: is_member && status == Status::Active && !mine(&rec.revoke_votes),
    }
}

// ───────────────────────── actions ─────────────────────────

/// The credential key of THIS device for a type (for testing: issue to yourself). The holder
/// normally shows it from their own device.
#[tauri::command]
pub async fn holder_key(
    app: AppHandle,
    issuer: State<'_, Issuer>,
    ctype: String,
) -> Result<String, UiError> {
    let issuer = issuer.inner().clone();
    with_device(app, issuer, move |c| {
        c.credential_key(&ctype)
            .map(|k| hex::encode(&k))
            .map_err(UiError::from)
    })
    .await
}

#[derive(Deserialize)]
pub struct IssueForm {
    ctype: String,
    /// Hex of the holder's credential key.
    holder_key: String,
    /// `name=value;name=value`.
    claims: String,
    valid_days: u64,
}

#[derive(Serialize)]
pub struct IssueReply {
    id: String,
    /// What the issuer hands to the holder (private types); empty for public ones.
    bundle: String,
    bundle_path: String,
}

/// A fee that covers the per-byte price of a credentials transaction.
fn fee_for(world: &dyn World, sender_key: &[u8], nonce: u64, payload_len: usize) -> u64 {
    let price = teko_coin::config(world.ledger()).map_or(0, |c| c.fee_per_byte);
    let probe = Tx {
        chain_id: world.chain_id().into(),
        nonce,
        module: teko_credentials::CREDENTIALS_MODULE,
        payload: vec![0u8; payload_len],
        sender: sender_key.to_vec(),
        signature: vec![0u8; 64],
        fee: u64::MAX,
    };
    price.saturating_mul(probe.to_bytes().len() as u64)
}

fn clean_claims(s: &str) -> Result<String, UiError> {
    let t = s.trim();
    if t.is_empty() || t.len() > 2_000 {
        return Err(ui_err(
            "InvalidArgument",
            "informe os atributos (nome=valor;nome=valor)",
        ));
    }
    for part in t.split(';').filter(|p| !p.trim().is_empty()) {
        if !part.contains('=') {
            return Err(ui_err(
                "InvalidArgument",
                format!(
                    "atributo sem '=': «{}» (use nome=valor;nome=valor)",
                    part.trim()
                ),
            ));
        }
    }
    Ok(t.to_string())
}

#[tauri::command]
pub async fn issue(
    app: AppHandle,
    issuer: State<'_, Issuer>,
    form: IssueForm,
) -> Result<IssueReply, UiError> {
    let holder = hex::decode(form.holder_key.trim())
        .ok()
        .filter(|k| k.len() == 33 || k.len() == 65)
        .ok_or_else(|| {
            ui_err(
                "InvalidArgument",
                "chave do titular inválida (hex, 33 ou 65 bytes)",
            )
        })?;
    issue_inner(
        app,
        issuer.inner().clone(),
        form.ctype,
        holder,
        form.claims,
        form.valid_days,
    )
    .await
}

/// Proposes a credential: the member's device shows everything, signs, and the transaction goes
/// to the chain. For a PRIVATE type the claims and the salt go to the holder, not to the chain.
async fn issue_inner(
    app: AppHandle,
    issuer: Issuer,
    ctype_name: String,
    holder: Vec<u8>,
    claims: String,
    valid_days: u64,
) -> Result<IssueReply, UiError> {
    let claims = clean_claims(&claims)?;
    if valid_days == 0 || valid_days > 36_500 {
        return Err(ui_err("InvalidArgument", "validade em dias (1 a 36500)"));
    }
    let form = IssueForm {
        ctype: ctype_name,
        holder_key: hex::encode(&holder),
        claims: claims.clone(),
        valid_days,
    };
    let mut world = open_world()?;
    let t = teko_credentials::type_info(World::ledger(&world), form.ctype.as_bytes())
        .ok_or_else(|| ui_err("InvalidArgument", "tipo de credencial desconhecido"))?;
    let valid_until = if t.validity_secs == 0 {
        0
    } else {
        now_secs().saturating_add(form.valid_days.saturating_mul(86_400))
    };
    let mut salt = [0u8; 16];
    rand::rng().fill_bytes(&mut salt);
    let chain_id = world.chain_id().to_string();
    let (ctype, public, contest) = (form.ctype.clone(), t.public, t.contest_secs);
    let ctype2 = ctype.clone();
    let claims2 = claims.clone();
    let holder2 = holder.clone();
    with_device(app, issuer, move |c| {
        let me = teko_crypto::hex::decode(&c.wallet_address()?.public_key)
            .map_err(|_| ui_err("Internal", "chave da carteira"))?;
        let addr = address_of(&me);
        let nonce = world.nonce(&addr);
        let payload_len = 320 + claims2.len();
        let fee = fee_for(&world, &me, nonce, payload_len);
        let tx = c.wallet_sign_credential_op(&CredentialOpArgs {
            chain_id: &chain_id,
            kind: CredentialOpKind::Propose,
            ctype: &ctype2,
            subject: &holder2,
            claims: claims2.as_bytes(),
            public,
            salt: Some(&salt),
            commitment: None,
            valid_until,
            reason: "",
            contest_secs: contest,
            fee,
            nonce,
        })?;
        world.submit(tx).map_err(|e| ui_err("Refused", e))?;
        Ok(())
    })
    .await?;
    let id = teko_credentials::credential_id(
        ctype.as_bytes(),
        &holder,
        &if public {
            teko_crypto::ledger::cred_public_commitment(claims.as_bytes()).to_vec()
        } else {
            teko_crypto::ledger::cred_private_commitment(&salt, claims.as_bytes()).to_vec()
        },
        if public { claims.as_bytes() } else { &[] },
        valid_until,
    );
    let id_hex = hex::encode(&id);
    if public {
        return Ok(IssueReply {
            id: id_hex,
            bundle: String::new(),
            bundle_path: String::new(),
        });
    }
    // PRIVATE: the claims and the salt go to the holder, not to the chain
    let bundle = Bundle {
        chain_id: net_chain_id(),
        ctype: ctype.clone(),
        id: id_hex.clone(),
        subject: hex::encode(&holder),
        claims,
        salt: hex::encode(&salt),
        valid_until,
    };
    let json = serde_json::to_string_pretty(&bundle).map_err(|e| ui_err("Internal", e))?;
    let dir = issued_dir();
    fs::create_dir_all(&dir).map_err(|e| ui_err("Internal", e))?;
    let path = dir.join(format!("{id_hex}.json"));
    fs::write(&path, &json).map_err(|e| ui_err("Internal", e))?;
    Ok(IssueReply {
        id: id_hex,
        bundle: json,
        bundle_path: path.display().to_string(),
    })
}

fn net_chain_id() -> String {
    teko_node::load_economy(&net_dir())
        .ok()
        .flatten()
        .map(|e| e.genesis.chain_id)
        .unwrap_or_default()
}

#[derive(Deserialize)]
pub struct VoteForm {
    /// `approve`, `veto` or `revoke`.
    op: String,
    ctype: String,
    id: String,
    reason: String,
}

#[tauri::command]
pub async fn vote(
    app: AppHandle,
    issuer: State<'_, Issuer>,
    form: VoteForm,
) -> Result<String, UiError> {
    let issuer = issuer.inner().clone();
    let kind = match form.op.as_str() {
        "approve" => CredentialOpKind::Approve,
        "veto" => CredentialOpKind::Veto,
        "revoke" => CredentialOpKind::Revoke,
        _ => return Err(ui_err("InvalidArgument", "operação inválida")),
    };
    let id = hex::decode(&form.id)
        .ok()
        .filter(|i| i.len() == 32)
        .ok_or_else(|| ui_err("InvalidArgument", "identificador inválido"))?;
    let mut world = open_world()?;
    let t = teko_credentials::type_info(World::ledger(&world), form.ctype.as_bytes())
        .ok_or_else(|| ui_err("InvalidArgument", "tipo de credencial desconhecido"))?;
    let rec = teko_credentials::credential(World::ledger(&world), form.ctype.as_bytes(), &id)
        .ok_or_else(|| ui_err("InvalidArgument", "credencial desconhecida"))?;
    let chain_id = world.chain_id().to_string();
    let contest = t.contest_secs;
    let reason = form.reason.trim().to_string();
    let ctype = form.ctype.clone();
    with_device(app, issuer, move |c| {
        let me = teko_crypto::hex::decode(&c.wallet_address()?.public_key)
            .map_err(|_| ui_err("Internal", "chave da carteira"))?;
        let addr = address_of(&me);
        let nonce = world.nonce(&addr);
        let fee = fee_for(&world, &me, nonce, 160);
        // public: claims on the chain; private: the claims and salt only if this app issued it
        let bundle = if t.public {
            None
        } else {
            load_bundle(&id, &rec)
        };
        let (claims, salt): (&[u8], Option<&[u8]>) = match (&bundle, t.public) {
            (_, true) => (&rec.claims, None),
            (Some((cl, s)), false) => (cl, Some(s.as_slice())),
            (None, false) => (&[], None),
        };
        let tx = c.wallet_sign_credential_op(&CredentialOpArgs {
            chain_id: &chain_id,
            kind,
            ctype: &ctype,
            subject: &rec.subject,
            claims,
            public: t.public,
            salt,
            commitment: if claims.is_empty() {
                Some(&rec.commitment)
            } else {
                None
            },
            valid_until: rec.valid_until,
            reason: &reason,
            contest_secs: contest,
            fee,
            nonce,
        })?;
        world.submit(tx).map_err(|e| ui_err("Refused", e))?;
        Ok(String::from("ok"))
    })
    .await
}

// ───────────────────────── civic identity credential (ADR-0038) ─────────────────────────

fn trust_policy() -> Result<teko_attestation::TrustPolicy, UiError> {
    let text = fs::read_to_string(resolve_dev_dir().join("trust.txt")).map_err(|_| {
        ui_err(
            "NoTrust",
            "Não encontrei dev/trust.txt. Rode: cargo run -p issuer-cli -- dev-setup",
        )
    })?;
    teko_attestation::TrustPolicy::parse(&text).map_err(|e| ui_err("NoTrust", e))
}

#[derive(Deserialize)]
struct RequestFile {
    civic_id: String,
    nonce: String,
    request: String,
}

fn read_request(
    name: &str,
) -> Result<(RequestFile, teko_protocol::IdentityRequest, Vec<u8>), UiError> {
    // a file NAME inside dev/requests: never a path
    if name.is_empty() || name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(ui_err("InvalidArgument", "nome de arquivo inválido"));
    }
    let text = fs::read_to_string(resolve_dev_dir().join("requests").join(name))
        .map_err(|_| ui_err("InvalidArgument", "pedido não encontrado"))?;
    let f: RequestFile =
        serde_json::from_str(&text).map_err(|_| ui_err("InvalidArgument", "pedido malformado"))?;
    let nonce =
        hex::decode(&f.nonce).map_err(|_| ui_err("InvalidArgument", "pedido malformado"))?;
    let bytes =
        hex::decode(&f.request).map_err(|_| ui_err("InvalidArgument", "pedido malformado"))?;
    let req = <teko_protocol::IdentityRequest as prost::Message>::decode(bytes.as_slice())
        .map_err(|_| ui_err("InvalidArgument", "pedido malformado"))?;
    Ok((f, req, nonce))
}

#[derive(Serialize)]
pub struct CivicRequestView {
    file: String,
    civic_id: String,
    /// The device proved it holds the identity key and is genuine.
    verified: bool,
    error: String,
}

/// The citizens' requests waiting in `dev/requests`, each checked: the proof of possession of the
/// identity key and the batch attestation of a genuine device.
#[tauri::command]
pub async fn civic_requests() -> Result<Vec<CivicRequestView>, UiError> {
    tauri::async_runtime::spawn_blocking(|| {
        let trust = trust_policy()?;
        let mut out = Vec::new();
        let mut names: Vec<String> = fs::read_dir(resolve_dev_dir().join("requests"))
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| n.starts_with("civic-request-") && n.ends_with(".json"))
            .collect();
        names.sort();
        for file in names {
            out.push(match read_request(&file) {
                Ok((f, req, nonce)) => {
                    match teko_client::verify_identity_request(&trust, &nonce, &req) {
                        Ok(()) => CivicRequestView {
                            file,
                            civic_id: f.civic_id,
                            verified: true,
                            error: String::new(),
                        },
                        Err(e) => CivicRequestView {
                            file,
                            civic_id: f.civic_id,
                            verified: false,
                            error: e.to_string(),
                        },
                    }
                }
                Err(e) => CivicRequestView {
                    file,
                    civic_id: String::new(),
                    verified: false,
                    error: e.message,
                },
            });
        }
        Ok(out)
    })
    .await
    .map_err(|e| ui_err("Internal", e))?
}

#[derive(Deserialize)]
pub struct CivicForm {
    request_file: String,
    name: String,
    birth_date: String,
    nationality: String,
    state: String,
    municipality: String,
    /// Set = a REISSUE: the same person keeps the same registry id.
    registry_id: Option<String>,
    valid_days: u64,
}

/// The organ issues the civic credential to the identity key of a verified request: the claims are
/// built from what the organ verified in person, and the member's device signs the proposal.
#[tauri::command]
pub async fn civic_issue(
    app: AppHandle,
    issuer: State<'_, Issuer>,
    form: CivicForm,
) -> Result<IssueReply, UiError> {
    let trust = trust_policy()?;
    let (_, req, nonce) = read_request(&form.request_file)?;
    teko_client::verify_identity_request(&trust, &nonce, &req)
        .map_err(|e| ui_err("InvalidRequest", format!("pedido recusado: {e}")))?;
    let registry_id = match form
        .registry_id
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty())
    {
        Some(r) => r.to_string(),
        None => {
            let mut r = [0u8; 4];
            rand::rng().fill_bytes(&mut r);
            format!("TK-{}", hex::encode(&r).to_uppercase())
        }
    };
    let claims = teko_credentials::civic::civic_claims(
        &teko_credentials::civic::CivicSubject {
            name: form.name,
            birth_date: form.birth_date,
            nationality: form.nationality,
            electoral_state: form.state,
            electoral_municipality: form.municipality,
        },
        &registry_id,
        now_secs(),
    )
    .map_err(|m| ui_err("InvalidArgument", m))?;
    issue_inner(
        app,
        issuer.inner().clone(),
        teko_crypto::civic::CIVIC_TYPE.to_string(),
        req.pk_identity,
        claims,
        form.valid_days,
    )
    .await
}

/// Hands the citizen the package for an approved credential: the claims, the salt and the
/// transactions the quorum signed, found ON THE CHAIN. The citizen's device checks it alone.
#[tauri::command]
pub async fn civic_deliver(id: String) -> Result<String, UiError> {
    tauri::async_runtime::spawn_blocking(move || {
        use teko_crypto::civic::{BundleTx, CIVIC_TYPE, CivicBundle, CivicPolicy, verify_civic_bundle};
        use teko_crypto::ledger::{CRED_OP_APPROVE, cred_propose_payload, cred_vote_payload};
        let id = hex::decode(&id)
            .ok()
            .filter(|i| i.len() == 32)
            .ok_or_else(|| ui_err("InvalidArgument", "identificador inválido"))?;
        let world = open_world()?;
        let rec = teko_credentials::credential(World::ledger(&world), CIVIC_TYPE.as_bytes(), &id)
            .ok_or_else(|| ui_err("InvalidArgument", "credencial desconhecida"))?;
        match rec.status_at(now_secs()) {
            Status::Pending | Status::Active => {}
            _ => {
                return Err(ui_err(
                    "NotReady",
                    "a credencial ainda não tem o quórum, ou foi vetada, revogada ou expirou",
                ));
            }
        }
        let (claims, salt) = load_bundle(&id, &rec).ok_or_else(|| {
            ui_err("NoBundle", "este app não guardou os atributos desta credencial (só quem a propôs os tem)")
        })?;
        let commitment: [u8; 32] = rec
            .commitment
            .as_slice()
            .try_into()
            .map_err(|_| ui_err("Internal", "compromisso"))?;
        let id32: [u8; 32] = id.as_slice().try_into().map_err(|_| ui_err("Internal", "id"))?;
        let propose = cred_propose_payload(CIVIC_TYPE.as_bytes(), &rec.subject, &commitment, &[], rec.valid_until);
        let approve = cred_vote_payload(CRED_OP_APPROVE, CIVIC_TYPE.as_bytes(), &id32, "");
        // the signed transactions of the issuance, from the chain's blocks
        let mut proposal: Option<BundleTx> = None;
        let mut approvals: Vec<BundleTx> = Vec::new();
        for d in world.chain().blocks() {
            let Ok(block) = d.block() else { continue };
            for raw in &block.txs {
                let Ok(tx) = Tx::from_bytes(raw) else { continue };
                if tx.module != teko_credentials::CREDENTIALS_MODULE {
                    continue;
                }
                let bt = BundleTx {
                    chain_id: tx.chain_id.clone(),
                    nonce: tx.nonce,
                    fee: tx.fee,
                    payload: tx.payload.clone(),
                    sender: tx.sender.clone(),
                    signature: tx.signature.clone(),
                };
                if tx.payload == propose {
                    proposal.get_or_insert(bt);
                } else if tx.payload == approve && !approvals.iter().any(|a| a.sender == bt.sender) {
                    approvals.push(bt);
                }
            }
        }
        let proposal = proposal.ok_or_else(|| ui_err("NotReady", "a proposta não está na chain"))?;
        let mut txs = vec![proposal];
        txs.extend(approvals);
        let bundle = CivicBundle {
            chain_id: world.chain_id().to_string(),
            ctype: CIVIC_TYPE.to_string(),
            claims,
            salt: salt.to_vec(),
            valid_until: rec.valid_until,
            txs,
        };
        // what the citizen's device will check, checked here first
        let trust = trust_policy()?;
        verify_civic_bundle(
            &bundle,
            &CivicPolicy {
                chain_id: trust.civic_chain_id.clone(),
                threshold: trust.civic_threshold,
                members: trust.civic_members.clone(),
            },
            &rec.subject,
        )
        .map_err(|e| {
            ui_err(
                "BadBundle",
                format!("o pacote não passaria no dispositivo ({e}): confira se o trust.txt é o desta rede (reinicie o emulador depois de criar a rede)"),
            )
        })?;
        let dir = resolve_dev_dir().join("inbox");
        fs::create_dir_all(&dir).map_err(|e| ui_err("Io", e))?;
        let path = dir.join(format!("civic-{}.bundle", &hex::encode(&id)[..8]));
        fs::write(&path, prost::Message::encode_to_vec(&bundle)).map_err(|e| ui_err("Io", e))?;
        Ok(path.display().to_string())
    })
    .await
    .map_err(|e| ui_err("Internal", e))?
}
