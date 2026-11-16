use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

use prost::Message;
use rand_core::{CryptoRng, Rng};
use teko_attestation::{
    BlobHasher, FactoryData, FirmwareError, FirmwareManifest, TrustPolicy, issue_request_message,
    manifest_fingerprint, sign_attestation, verify_blob, verify_manifest,
};
use teko_crypto::civic::{
    CIVIC_TYPE, CivicBundle, CivicPolicy, CivicPresentation, claim_list, parse_claims,
    presentation_signing_bytes, verify_civic_bundle,
};
use teko_crypto::credential::{Claim, claim_digest};
use teko_crypto::{
    SigningKey, canonical_service_id, civic_id, derive_key, derive_signing_key, pairwise_id,
    public_key_bytes, sign_tagged, tagged_hash, tags, voter,
};
use teko_election::ballot_ctx;
use teko_election::{
    BallotBody, Domicile, ElectionBundle, ElectionError, ElectionManifest, ElectionStatus,
    GovernancePolicy, OptionKind, Phase, RegistrationBlinder, ScopeLevel, Transition, contest_hash,
    is_eligible, manifest_hash, registration_message, registration_request_message, replay,
    sign_ballot, validate_revision,
};
use teko_protocol::{
    AttestationResponse, AttributeProof, BallotSignature, ButtonKind, ButtonRequest, CivicId,
    ContestInfo, CredentialLoaded, DeviceState, DocumentSignature, ElectionLoaded, ErrorCode,
    Failure, Features, FirmwareStatus, IdentityRequest, PROTO_VERSION, PairwiseId, PinSet,
    PinWrong, Predicate, PredicateKind, RegistrationRequest, RegistrationToken, Request, Response,
    Success, TransferSigned, Unlocked, VoterKey, WalletBackupOk, WalletReady, WipeDone,
};
use teko_wallet::{MasterKey, Phrase, WORD_COUNT};
use zeroize::{Zeroize, Zeroizing};

use crate::config::DeviceConfig;
use crate::hal::{Decision, Hal, Screen};
use crate::pin::{PinPolicyError, derive_pin_hash, pin_matches, validate_new_pin};
use crate::state::Persisted;
use crate::text::{hex_lines, sanitize, wrap};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceError {
    CorruptState,
}

impl fmt::Display for DeviceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeviceError::CorruptState => f.write_str("persisted device state is corrupt"),
        }
    }
}

fn fail(code: ErrorCode, msg: &str) -> Failure {
    Failure::new(code, msg)
}

struct FwUpload {
    manifest: FirmwareManifest,
    hasher: BlobHasher,
    received: u64,
}

pub struct Device<H: Hal, R: Rng + CryptoRng> {
    hal: H,
    rng: R,
    cfg: DeviceConfig,
    trust: TrustPolicy,
    p: Persisted,
    unlocked: bool,
    last_activity_ms: u64,
    upload: Option<FwUpload>,
    /// What the home screen last showed (state, unlocked, firmware): avoids
    /// redrawing on every passive host poll.
    shown_home: (i32, bool, u32),
    /// Voter-side blind-signature math (std crate behind a trait; ADR-0008).
    blinder: Option<Box<dyn RegistrationBlinder>>,
}

impl<H: Hal, R: Rng + CryptoRng> Drop for Device<H, R> {
    fn drop(&mut self) {
        self.p.k_root.zeroize();
        self.p.voter_secret.zeroize();
        self.p.pin_hash.zeroize();
        self.p.batch_key.zeroize();
        self.p.wallet_master.zeroize();
    }
}

impl<H: Hal, R: Rng + CryptoRng> Device<H, R> {
    /// Installs the registration math. Without it the device refuses to register.
    pub fn with_registration_blinder(mut self, b: Box<dyn RegistrationBlinder>) -> Self {
        self.blinder = Some(b);
        self
    }

    /// Boots the device. Loads persisted state or starts `Virgin`
    /// (with the factory attestation material, if provided).
    pub fn new(
        mut hal: H,
        rng: R,
        cfg: DeviceConfig,
        trust: TrustPolicy,
        factory: Option<FactoryData>,
    ) -> Result<Self, DeviceError> {
        let loaded = hal.load().map(Zeroizing::new);
        let mut p = match loaded {
            Some(bytes) => {
                Persisted::decode(bytes.as_slice()).map_err(|_| DeviceError::CorruptState)?
            }
            None => Persisted::default(),
        };
        if let (true, Some(f)) = (p.batch_key.is_empty(), factory) {
            p.batch_key = f.batch_key;
            p.batch_cert = f.batch_cert;
        }
        let now = hal.now_ms();
        let mut dev = Self {
            hal,
            rng,
            cfg,
            trust,
            p,
            unlocked: false,
            last_activity_ms: now,
            upload: None,
            shown_home: (-1, false, 0),
            blinder: None,
        };
        let _ = dev.save();
        dev.show_home();
        Ok(dev)
    }

    pub fn hal(&self) -> &H {
        &self.hal
    }

    pub fn hal_mut(&mut self) -> &mut H {
        &mut self.hal
    }

    #[cfg(test)]
    pub(crate) fn secrets_for_test(&self) -> Vec<Vec<u8>> {
        let mut v = Vec::from([
            self.p.k_root.clone(),
            self.p.voter_secret.clone(),
            self.p.batch_key.clone(),
            self.p.pin_hash.clone(),
        ]);
        if let Ok(k) = self.identity_key() {
            v.push(k.to_bytes().to_vec());
        }
        // Appended LAST: tests index [0] as `k_root`.
        v.push(self.p.wallet_master.clone());
        v
    }

    pub fn state(&self) -> DeviceState {
        self.p.state()
    }

    /// Locks the device (e.g. on USB disconnect). PIN is required again.
    pub fn lock(&mut self) {
        self.unlocked = false;
        self.show_home();
    }

    pub fn is_unlocked(&self) -> bool {
        self.unlocked
    }

    /// Handles one host request. Never panics; errors become `Failure`.
    pub fn handle(&mut self, req: Request) -> Response {
        // Passive host polling must NOT extend the unlock window: only real
        // user-driven activity counts as activity.
        let passive = matches!(
            &req,
            Request::GetFeatures(_)
                | Request::Ping(_)
                | Request::Initialize(_)
                | Request::AttestationChallenge(_)
        );
        self.expire_unlock();
        let resp = match self.dispatch(req) {
            Ok(r) => r,
            Err(f) => Response::Failure(f),
        };
        if !passive {
            self.last_activity_ms = self.hal.now_ms();
        }
        // Passive polling must not spam the screen: redraw only on change.
        if !passive || self.home_key() != self.shown_home {
            self.show_home();
        }
        resp
    }

    // ───────────────────────── helpers ─────────────────────────

    fn expire_unlock(&mut self) {
        if self.unlocked {
            let idle = self.hal.now_ms().saturating_sub(self.last_activity_ms);
            if idle > self.cfg.unlock_timeout_ms {
                self.unlocked = false;
            }
        }
    }

    fn save(&mut self) -> Result<(), Failure> {
        let bytes = Zeroizing::new(self.p.encode_to_vec());
        if self.hal.store(bytes.as_slice()) {
            Ok(())
        } else {
            Err(fail(ErrorCode::Internal, "falha ao gravar armazenamento"))
        }
    }

    fn home_key(&self) -> (i32, bool, u32) {
        (self.p.state, self.unlocked, self.p.fw_version)
    }

    fn show_home(&mut self) {
        self.shown_home = self.home_key();
        let (status, hint) = match (self.p.state(), self.unlocked) {
            (DeviceState::Virgin, _) => ("Sem firmware", "Conecte a Teko Suite"),
            (DeviceState::FirmwareInstalled, _) => {
                ("Firmware instalado", "Defina o PIN pela Suite")
            }
            (DeviceState::Initialized, true) => ("Chaves geradas", "Aguardando credencial"),
            (DeviceState::CredentialsIssued, true) => ("DESBLOQUEADO", "Pronto para uso"),
            (_, false) => ("BLOQUEADO", "Desbloqueie pela Suite"),
        };
        let lines = Vec::from([
            String::from(status),
            String::from(hint),
            format!("Firmware v{}", self.p.fw_version),
        ]);
        let screen = Screen::new("TEKO", lines);
        self.hal.show(&screen);
    }

    fn is_pin_set(&self) -> bool {
        matches!(
            self.p.state(),
            DeviceState::Initialized | DeviceState::CredentialsIssued
        )
    }

    fn require_unlocked_initialized(&self) -> Result<(), Failure> {
        if !self.is_pin_set() {
            return Err(fail(
                ErrorCode::InvalidState,
                "dispositivo não inicializado",
            ));
        }
        if !self.unlocked {
            return Err(fail(ErrorCode::InvalidState, "dispositivo bloqueado"));
        }
        Ok(())
    }

    fn require_ready(&self) -> Result<(), Failure> {
        self.require_unlocked_initialized()?;
        if self.p.state() != DeviceState::CredentialsIssued {
            return Err(fail(
                ErrorCode::InvalidState,
                "credenciais ainda não emitidas",
            ));
        }
        Ok(())
    }

    fn root(&self) -> Result<Zeroizing<[u8; 32]>, Failure> {
        let a: [u8; 32] = self
            .p
            .k_root
            .as_slice()
            .try_into()
            .map_err(|_| fail(ErrorCode::Internal, "chave raiz ausente"))?;
        Ok(Zeroizing::new(a))
    }

    fn identity_key(&self) -> Result<SigningKey, Failure> {
        let root = self.root()?;
        derive_signing_key(&root, tags::IDENTITY)
            .map_err(|_| fail(ErrorCode::Internal, "derivação falhou"))
    }

    fn batch_key(&self) -> Result<SigningKey, Failure> {
        SigningKey::from_slice(&self.p.batch_key)
            .map_err(|_| fail(ErrorCode::Internal, "sem chave de atestação de fábrica"))
    }

    fn voter_secret(&self) -> Result<Zeroizing<[u8; 32]>, Failure> {
        let a: [u8; 32] = self
            .p
            .voter_secret
            .as_slice()
            .try_into()
            .map_err(|_| fail(ErrorCode::Internal, "segredo de eleitor ausente"))?;
        Ok(Zeroizing::new(a))
    }

    /// Shows `screen`, tells the host we wait for the user, and blocks on the
    /// PHYSICAL decision. This is the only way a signature can be authorized.
    fn confirm(&mut self, kind: ButtonKind, screen: Screen) -> Result<(), Failure> {
        self.hal.show(&screen);
        self.hal.notify(&Response::ButtonRequest(ButtonRequest {
            kind: kind as i32,
        }));
        match self.hal.confirm(kind) {
            Decision::Confirm => Ok(()),
            Decision::Reject => Err(fail(ErrorCode::UserRejected, "recusado no dispositivo")),
            Decision::Timeout => Err(fail(ErrorCode::Timeout, "tempo esgotado no dispositivo")),
        }
    }

    fn wipe_internal(&mut self) -> Result<(), Failure> {
        self.p.k_root.zeroize();
        self.p.voter_secret.zeroize();
        self.p.pin_hash.zeroize();
        self.p.credential.zeroize();
        self.p.wallet_master.zeroize();
        let fw_version = self.p.fw_version;
        let batch_key = core::mem::take(&mut self.p.batch_key);
        let batch_cert = core::mem::take(&mut self.p.batch_cert);
        let state = if fw_version > 0 {
            DeviceState::FirmwareInstalled
        } else {
            DeviceState::Virgin
        };
        self.p = Persisted {
            state: state as i32,
            fw_version,
            batch_key,
            batch_cert,
            ..Persisted::default()
        };
        self.unlocked = false;
        self.upload = None;
        self.save()
    }

    fn features(&self) -> Features {
        Features {
            proto_ver: u32::from(PROTO_VERSION),
            fw_version: self.p.fw_version,
            state: self.p.state,
            locked: self.is_pin_set() && !self.unlocked,
            retries_remaining: self
                .cfg
                .max_pin_attempts
                .saturating_sub(self.p.pin_fail_count),
            capabilities: [
                "attestation",
                "firmware-update",
                "pin-on-device",
                "sign-document",
                "attribute-proof",
                "pairwise-id",
                "voter-commitment",
                "ballot-stub",
                "wallet-seed",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
            label: self.cfg.label.clone(),
            has_credential: !self.p.credential.is_empty(),
            wallet_ready: self.wallet_ready(),
        }
    }

    // ───────────────────────── dispatch ─────────────────────────

    fn dispatch(&mut self, req: Request) -> Result<Response, Failure> {
        match req {
            Request::Initialize(m) => {
                if m.host_proto_max == 0 {
                    return Err(fail(ErrorCode::UnsupportedVersion, "versão de protocolo"));
                }
                Ok(Response::Features(self.features()))
            }
            Request::GetFeatures(_) => Ok(Response::Features(self.features())),
            Request::Ping(_) | Request::ButtonAck(_) | Request::Cancel(_) => {
                Ok(Response::Success(Success {}))
            }
            Request::AttestationChallenge(m) => self.attestation(&m.nonce),
            Request::FirmwareBegin(m) => self.firmware_begin(&m.manifest),
            Request::FirmwareChunk(m) => self.firmware_chunk(m.offset, &m.data),
            Request::FirmwareFinish(_) => self.firmware_finish(),
            Request::SetupPin(_) => self.setup_pin(),
            Request::Unlock(_) => self.unlock(),
            Request::GetCivicId(_) => self.get_civic_id(),
            Request::SetupWallet(_) => self.setup_wallet(),
            Request::RestoreWallet(_) => self.restore_wallet(),
            Request::CheckWalletBackup(_) => self.check_wallet_backup(),
            Request::GetAddress(_) => self.get_address(),
            Request::SignAssetOffer(m) => self.sign_asset_trade(&AssetTrade {
                accept: false,
                chain_id: &m.chain_id,
                class: &m.class,
                asset: &m.asset,
                other_address: &m.buyer_address,
                price: m.price,
                version: m.version,
                metadata: &m.metadata,
                window_secs: m.window_secs,
                fee: m.fee,
                nonce: m.nonce,
            }),
            Request::SignAssetAccept(m) => self.sign_asset_trade(&AssetTrade {
                accept: true,
                chain_id: &m.chain_id,
                class: &m.class,
                asset: &m.asset,
                other_address: &m.seller_address,
                price: m.price,
                version: m.version,
                metadata: &m.metadata,
                window_secs: m.window_secs,
                fee: m.fee,
                nonce: m.nonce,
            }),
            Request::GetCredentialKey(m) => self.get_credential_key(&m.ctype),
            Request::GetCredentialKeys(m) => self.get_credential_keys(&m.ctypes),
            Request::OpenCredentialBundle(m) => {
                self.open_credential_bundle(&m.ctype, &m.subject, &m.sealed)
            }
            Request::CheckMyBallot(m) => self.check_my_ballot(&m),
            Request::SignCredentialOp(m) => self.sign_credential_op(&m),
            Request::SignStake(m) => {
                self.sign_stake(&m.chain_id, m.action, m.amount, m.fee, m.nonce)
            }
            Request::SignTransfer(m) => {
                self.sign_transfer(&m.chain_id, &m.to_address, m.amount, m.fee, m.nonce)
            }
            Request::GetPairwiseId(m) => self.get_pairwise_id(&m.service_id),
            Request::SignDocument(m) => self.sign_document(&m.hash, &m.summary),
            Request::ProveAttribute(m) => {
                self.prove_attribute(m.predicate, &m.verifier_nonce, &m.verifier_name)
            }
            Request::LoadElection(m) => self.load_election(&m.manifest, &m.transitions),
            Request::GetElection(_) => self.get_election(),
            Request::GetVoterKey(m) => self.get_voter_key(&m.election_id, &m.contest_id),
            Request::BeginRegistration(m) => self.begin_registration(&m.election_id, &m.contest_id),
            Request::FinishRegistration(m) => {
                self.finish_registration(&m.election_id, &m.contest_id, &m.blind_signature)
            }
            Request::SignBallot(m) => self.sign_ballot(
                &m.election_id,
                &m.contest_id,
                m.option_index,
                &m.option_indices,
            ),
            Request::Wipe(_) => self.wipe(),
            Request::GetIdentityRequest(m) => {
                self.get_identity_request(&m.authority_nonce, &m.authority_name)
            }
            Request::LoadCredential(m) => self.load_credential(&m.credential),
        }
    }

    // ───────────────────────── attestation ─────────────────────────

    fn attestation(&mut self, nonce: &[u8]) -> Result<Response, Failure> {
        if !(16..=64).contains(&nonce.len()) {
            return Err(fail(
                ErrorCode::InvalidArgument,
                "nonce deve ter 16 a 64 bytes",
            ));
        }
        let key = self.batch_key()?;
        let signature = sign_attestation(&key, nonce, self.p.fw_version, self.p.state);
        Ok(Response::AttestationResponse(AttestationResponse {
            chain: Vec::from([self.p.batch_cert.clone()]),
            signature,
            fw_version: self.p.fw_version,
            state: self.p.state,
        }))
    }

    // ───────────────────────── firmware (bootloader) ─────────────────────────

    fn firmware_begin(&mut self, manifest_bytes: &[u8]) -> Result<Response, Failure> {
        // Without a PIN anyone may install; once set, the user must be unlocked.
        if self.is_pin_set() && !self.unlocked {
            return Err(fail(ErrorCode::InvalidState, "dispositivo bloqueado"));
        }
        let manifest = FirmwareManifest::decode(manifest_bytes)
            .map_err(|_| fail(ErrorCode::InvalidArgument, "manifesto malformado"))?;
        verify_manifest(&self.trust, &manifest, self.p.fw_version).map_err(|e| match e {
            FirmwareError::Malformed => fail(ErrorCode::InvalidArgument, "manifesto malformado"),
            FirmwareError::InsufficientSignatures { valid, required } => fail(
                ErrorCode::InsufficientSignatures,
                &format!("assinaturas insuficientes: {valid} de {required}"),
            ),
            FirmwareError::Downgrade { offered, installed } => fail(
                ErrorCode::Downgrade,
                &format!("versão {offered} é anterior à instalada ({installed})"),
            ),
            FirmwareError::HashMismatch => fail(ErrorCode::HashMismatch, "hash"),
        })?;
        let mut lines = Vec::from([
            format!("Versão atual: v{}", self.p.fw_version),
            format!("Nova versão:  v{}", manifest.version),
            String::from("Impressão digital:"),
            manifest_fingerprint(&manifest),
            format!(
                "Assinado por {} de {}",
                self.trust.fw_threshold,
                self.trust.fw_signers.len()
            ),
        ]);
        lines.push(String::from("Compare com a Suite"));
        self.confirm(
            ButtonKind::FirmwareInstall,
            Screen::new("INSTALAR FIRMWARE?", lines),
        )?;
        self.upload = Some(FwUpload {
            manifest,
            hasher: BlobHasher::new(),
            received: 0,
        });
        Ok(Response::FirmwareStatus(FirmwareStatus {
            received: 0,
            installed_version: self.p.fw_version,
        }))
    }

    fn firmware_chunk(&mut self, offset: u64, data: &[u8]) -> Result<Response, Failure> {
        let up = self
            .upload
            .as_mut()
            .ok_or_else(|| fail(ErrorCode::InvalidState, "nenhuma atualização em andamento"))?;
        let end = up.received.saturating_add(data.len() as u64);
        if offset != up.received || end > up.manifest.size {
            self.upload = None;
            return Err(fail(
                ErrorCode::InvalidArgument,
                "deslocamento/tamanho inválido",
            ));
        }
        up.hasher.update(data);
        up.received = end;
        Ok(Response::FirmwareStatus(FirmwareStatus {
            received: end,
            installed_version: self.p.fw_version,
        }))
    }

    fn firmware_finish(&mut self) -> Result<Response, Failure> {
        let up = self
            .upload
            .take()
            .ok_or_else(|| fail(ErrorCode::InvalidState, "nenhuma atualização em andamento"))?;
        if up.received != up.manifest.size {
            return Err(fail(ErrorCode::InvalidArgument, "firmware incompleto"));
        }
        let digest = up.hasher.finalize();
        verify_blob(&up.manifest, &digest)
            .map_err(|_| fail(ErrorCode::HashMismatch, "hash do firmware não confere"))?;
        // Re-check anti-downgrade at commit time (monotonic counter).
        if up.manifest.version < self.p.fw_version {
            return Err(fail(ErrorCode::Downgrade, "downgrade recusado"));
        }
        self.p.fw_version = up.manifest.version;
        if self.p.state() == DeviceState::Virgin {
            self.p.state = DeviceState::FirmwareInstalled as i32;
        }
        self.save()?;
        Ok(Response::FirmwareStatus(FirmwareStatus {
            received: up.received,
            installed_version: self.p.fw_version,
        }))
    }

    // ───────────────────────── wallet (ADR-0023) ─────────────────────────

    fn wallet_ready(&self) -> bool {
        self.p.wallet_master.len() == 64
    }

    /// Identity steps need a wallet whose backup the user already confirmed on this
    /// device: the host cannot skip the backup check.
    fn require_wallet(&self) -> Result<(), Failure> {
        if !self.wallet_ready() {
            return Err(fail(
                ErrorCode::InvalidState,
                "configure a carteira antes (frase de recuperação)",
            ));
        }
        Ok(())
    }

    fn master(&self) -> Result<MasterKey, Failure> {
        MasterKey::from_bytes(&self.p.wallet_master)
            .ok_or_else(|| fail(ErrorCode::Internal, "carteira ausente"))
    }

    /// Random `k` distinct positions in `0..n`, ascending.
    fn pick_positions(&mut self, n: usize, k: usize) -> Vec<usize> {
        let mut pool: Vec<usize> = (0..n).collect();
        let mut out = Vec::with_capacity(k);
        for _ in 0..k.min(n) {
            let i = (self.rng.next_u32() as usize) % pool.len();
            out.push(pool.swap_remove(i));
        }
        out.sort_unstable();
        out
    }

    /// Shows the phrase on the DEVICE screen, six words per page. Words never go to
    /// the host: only a `ButtonRequest` of kind `SeedWords` does.
    fn show_phrase(&mut self, phrase: &Phrase) -> Result<(), Failure> {
        for (page, chunk) in phrase.words().chunks(6).enumerate() {
            let mut lines: Vec<String> = chunk
                .iter()
                .enumerate()
                .map(|(i, w)| format!("{:>2} {}", page * 6 + i + 1, w))
                .collect();
            lines.push(String::from("Anote em papel. Ninguém"));
            lines.push(String::from("deve ver. Nunca digite"));
            lines.push(String::from("isto no computador."));
            let title = format!("FRASE {}/{}", page + 1, WORD_COUNT / 6);
            self.confirm(ButtonKind::SeedWords, Screen::new(&title, lines))?;
        }
        Ok(())
    }

    /// Asks (on the device) whether to use a BIP-39 passphrase and reads it twice.
    fn ask_passphrase(&mut self) -> Result<Zeroizing<String>, Failure> {
        let lines = Vec::from([
            String::from("Senha extra (25ª palavra)."),
            String::from("Sem ela a frase abre a"),
            String::from("carteira. ESQUECER A SENHA"),
            String::from("= PERDER OS FUNDOS."),
            String::from("Confirmar = usar senha"),
            String::from("Recusar = sem senha"),
        ]);
        let want = match self.confirm(
            ButtonKind::WalletPassphrase,
            Screen::new("USAR PASSPHRASE?", lines),
        ) {
            Ok(()) => true,
            Err(f) if f.code == ErrorCode::UserRejected as i32 => false,
            Err(f) => return Err(f),
        };
        if !want {
            return Ok(Zeroizing::new(String::new()));
        }
        self.hal.show(&Screen::new(
            "PASSPHRASE",
            Vec::from([String::from("Digite (ASCII, até 64)")]),
        ));
        let first = self
            .hal
            .read_secret_text("Passphrase")
            .ok_or_else(|| fail(ErrorCode::UserRejected, "passphrase cancelada"))?;
        self.hal.show(&Screen::new(
            "REPITA A PASSPHRASE",
            Vec::from([String::from("Digite novamente")]),
        ));
        let again = self
            .hal
            .read_secret_text("Repita a passphrase")
            .ok_or_else(|| fail(ErrorCode::UserRejected, "passphrase cancelada"))?;
        if !constant_time_eq(first.as_bytes(), again.as_bytes()) {
            return Err(fail(
                ErrorCode::InvalidArgument,
                "as passphrases não coincidem",
            ));
        }
        if first.is_empty() {
            return Ok(Zeroizing::new(String::new()));
        }
        Ok(first)
    }

    fn persist_wallet(&mut self, master: &MasterKey, has_passphrase: bool) -> Result<(), Failure> {
        let before = (self.p.wallet_master.clone(), self.p.wallet_passphrase);
        self.p.wallet_master = master.as_bytes().to_vec();
        self.p.wallet_passphrase = has_passphrase;
        if let Err(e) = self.save() {
            self.p.wallet_master.zeroize();
            self.p.wallet_master = before.0;
            self.p.wallet_passphrase = before.1;
            return Err(e);
        }
        Ok(())
    }

    fn wallet_screen(&mut self, title: &str, address: &str) {
        let mut lines = Vec::from([String::from("Seu endereço:")]);
        lines.extend(wrap(address, 22));
        self.hal.show(&Screen::new(title, lines));
    }

    fn setup_wallet(&mut self) -> Result<Response, Failure> {
        self.require_unlocked_initialized()?;
        if self.wallet_ready() {
            return Err(fail(ErrorCode::InvalidState, "a carteira já foi criada"));
        }
        self.confirm(
            ButtonKind::WalletCreate,
            Screen::new(
                "CRIAR CARTEIRA",
                Vec::from([
                    String::from("O dispositivo vai mostrar"),
                    String::from("24 palavras. Quem tiver"),
                    String::from("a frase controla os"),
                    String::from("fundos. Separe papel e"),
                    String::from("um lugar seguro."),
                ]),
            ),
        )?;
        let mut entropy = Zeroizing::new([0u8; 32]);
        self.rng.fill_bytes(entropy.as_mut_slice());
        let phrase = Phrase::from_entropy(&entropy)
            .map_err(|_| fail(ErrorCode::Internal, "falha ao gerar a frase"))?;
        let mut passphrase: Option<Zeroizing<String>> = None;
        let mut attempts = 0u8;
        loop {
            attempts += 1;
            if attempts > 5 {
                return Err(fail(
                    ErrorCode::InvalidArgument,
                    "confirmação da frase falhou várias vezes; recomece",
                ));
            }
            self.show_phrase(&phrase)?;
            if passphrase.is_none() {
                passphrase = Some(self.ask_passphrase()?);
            }
            // The user must prove the phrase was written down: 3 random positions.
            let positions = self.pick_positions(WORD_COUNT, 3);
            self.hal.show(&Screen::new(
                "CONFIRME A FRASE",
                Vec::from([
                    String::from("Digite as palavras"),
                    String::from("pedidas, da sua cópia."),
                ]),
            ));
            self.hal.notify(&Response::ButtonRequest(ButtonRequest {
                kind: ButtonKind::WalletConfirm as i32,
            }));
            let mut all_ok = true;
            for pos in positions {
                let typed = self
                    .hal
                    .read_word(pos + 1)
                    .ok_or_else(|| fail(ErrorCode::UserRejected, "confirmação cancelada"))?;
                if !phrase.matches(pos, typed.as_str()) {
                    all_ok = false;
                    break;
                }
            }
            if all_ok {
                break;
            }
            self.hal.show(&Screen::new(
                "PALAVRA INCORRETA",
                Vec::from([
                    String::from("A frase será mostrada"),
                    String::from("de novo. Confira sua"),
                    String::from("anotação."),
                ]),
            ));
        }
        let pass = passphrase.unwrap_or_else(|| Zeroizing::new(String::new()));
        let master = phrase
            .master_key(pass.as_str())
            .map_err(|_| fail(ErrorCode::InvalidArgument, "passphrase inválida"))?;
        let address = master
            .address_string(0)
            .map_err(|_| fail(ErrorCode::Internal, "endereço"))?;
        self.persist_wallet(&master, !pass.is_empty())?;
        self.wallet_screen("CARTEIRA PRONTA", &address);
        Ok(Response::WalletReady(WalletReady {
            address,
            has_passphrase: !pass.is_empty(),
        }))
    }

    /// Reads the 24 typed words and parses them (generic error: no word, no position).
    fn read_phrase(&mut self) -> Result<Phrase, Failure> {
        let mut words: Vec<Zeroizing<String>> = Vec::with_capacity(WORD_COUNT);
        for i in 1..=WORD_COUNT {
            let w = self
                .hal
                .read_word(i)
                .ok_or_else(|| fail(ErrorCode::UserRejected, "digitação cancelada"))?;
            words.push(w);
        }
        let refs: Vec<&str> = words.iter().map(|w| w.as_str()).collect();
        Phrase::parse(&refs).map_err(|_| {
            fail(
                ErrorCode::InvalidArgument,
                "frase inválida (palavra desconhecida ou checksum errado)",
            )
        })
    }

    fn restore_wallet(&mut self) -> Result<Response, Failure> {
        self.require_unlocked_initialized()?;
        if self.wallet_ready() {
            return Err(fail(
                ErrorCode::InvalidState,
                "já existe carteira; apague o dispositivo para restaurar outra",
            ));
        }
        self.confirm(
            ButtonKind::WalletRestore,
            Screen::new(
                "RESTAURAR CARTEIRA",
                Vec::from([
                    String::from("Você vai digitar as 24"),
                    String::from("palavras AQUI, no"),
                    String::from("dispositivo."),
                ]),
            ),
        )?;
        let phrase = self.read_phrase()?;
        let pass = self.ask_passphrase()?;
        let master = phrase
            .master_key(pass.as_str())
            .map_err(|_| fail(ErrorCode::InvalidArgument, "passphrase inválida"))?;
        let address = master
            .address_string(0)
            .map_err(|_| fail(ErrorCode::Internal, "endereço"))?;
        // A wrong phrase silently yields ANOTHER wallet: the user compares the address.
        let mut lines = Vec::from([String::from("Endereço da carteira:")]);
        lines.extend(wrap(&address, 22));
        lines.push(String::from("Confere com o seu?"));
        self.confirm(
            ButtonKind::WalletRestore,
            Screen::new("CONFIRA O ENDEREÇO", lines),
        )?;
        self.persist_wallet(&master, !pass.is_empty())?;
        self.wallet_screen("CARTEIRA RESTAURADA", &address);
        Ok(Response::WalletReady(WalletReady {
            address,
            has_passphrase: !pass.is_empty(),
        }))
    }

    fn check_wallet_backup(&mut self) -> Result<Response, Failure> {
        self.require_unlocked_initialized()?;
        self.require_wallet()?;
        self.confirm(
            ButtonKind::WalletCheck,
            Screen::new(
                "VERIFICAR BACKUP",
                Vec::from([
                    String::from("Digite as 24 palavras"),
                    String::from("da sua cópia, AQUI."),
                ]),
            ),
        )?;
        let phrase = self.read_phrase()?;
        let pass = if self.p.wallet_passphrase {
            self.hal.show(&Screen::new(
                "PASSPHRASE",
                Vec::from([String::from("Digite a passphrase")]),
            ));
            self.hal
                .read_secret_text("Passphrase")
                .ok_or_else(|| fail(ErrorCode::UserRejected, "passphrase cancelada"))?
        } else {
            Zeroizing::new(String::new())
        };
        let master = phrase
            .master_key(pass.as_str())
            .map_err(|_| fail(ErrorCode::InvalidArgument, "passphrase inválida"))?;
        if !constant_time_eq(master.as_bytes(), &self.p.wallet_master) {
            self.hal.show(&Screen::new(
                "BACKUP NÃO CONFERE",
                Vec::from([
                    String::from("A frase digitada não é"),
                    String::from("a desta carteira."),
                ]),
            ));
            return Err(fail(ErrorCode::InvalidArgument, "o backup não confere"));
        }
        self.hal.show(&Screen::new(
            "BACKUP CONFERE",
            Vec::from([String::from("A frase está correta.")]),
        ));
        Ok(Response::WalletBackupOk(WalletBackupOk {}))
    }

    /// Builds, SHOWS and signs a coin transfer. The host supplies only the fields; the
    /// bytes that are signed are derived here, from exactly what the screen shows.
    fn sign_transfer(
        &mut self,
        chain_id: &str,
        to_address: &str,
        amount: u64,
        fee: u64,
        nonce: u64,
    ) -> Result<Response, Failure> {
        self.require_unlocked_initialized()?;
        self.require_wallet()?;
        let chain = sanitize(chain_id, 48)
            .filter(|c| !c.trim().is_empty() && c.as_str() == chain_id)
            .ok_or_else(|| fail(ErrorCode::InvalidArgument, "rede inválida"))?;
        let to = teko_crypto::parse_wallet_address(to_address)
            .map_err(|_| fail(ErrorCode::InvalidArgument, "endereço de destino inválido"))?;
        if amount == 0 {
            return Err(fail(ErrorCode::InvalidArgument, "valor deve ser positivo"));
        }
        let total = amount
            .checked_add(fee)
            .ok_or_else(|| fail(ErrorCode::InvalidArgument, "valor + taxa excede o limite"))?;
        let master = self.master()?;
        let sk = master
            .signing_key(0)
            .map_err(|_| fail(ErrorCode::Internal, "chave da carteira"))?;
        let sender = public_key_bytes(&sk);
        if teko_crypto::ledger_address(&sender) == to {
            return Err(fail(
                ErrorCode::InvalidArgument,
                "destino é a sua própria carteira",
            ));
        }
        // What the user will read is built from the SAME values that get signed.
        let mut lines = Vec::from([String::from("Para:")]);
        lines.extend(wrap(to_address, 22));
        push_amount(&mut lines, "Valor:", amount);
        push_amount(&mut lines, "Taxa: ", fee);
        push_amount(&mut lines, "Total:", total);
        lines.push(format!("Rede:  {chain}"));
        self.confirm(
            ButtonKind::SignTransfer,
            Screen::new("ENVIAR MOEDAS", lines),
        )?;
        let payload = teko_crypto::ledger::coin_transfer_payload(&to, amount);
        let msg = teko_crypto::ledger::tx_signing_bytes(
            &chain,
            nonce,
            fee,
            teko_crypto::ledger::MODULE_COIN,
            &payload,
            &sender,
        );
        let signature = sign_tagged(&sk, tags::LEDGER_TX, &msg);
        Ok(Response::TransferSigned(TransferSigned {
            signature,
            public_key: sender,
            sealed: Vec::new(),
        }))
    }

    /// Builds, SHOWS and signs a staking action (bond / unbond / claim) of this wallet.
    fn sign_stake(
        &mut self,
        chain_id: &str,
        action: u32,
        amount: u64,
        fee: u64,
        nonce: u64,
    ) -> Result<Response, Failure> {
        use teko_crypto::ledger::{STAKE_BOND, STAKE_CLAIM, STAKE_UNBOND, coin_stake_payload};
        self.require_unlocked_initialized()?;
        self.require_wallet()?;
        let bad = |m: &str| fail(ErrorCode::InvalidArgument, m);
        let chain = sanitize(chain_id, 48)
            .filter(|c| !c.trim().is_empty() && c.as_str() == chain_id)
            .ok_or_else(|| bad("rede inválida"))?;
        let act = u8::try_from(action).map_err(|_| bad("ação inválida"))?;
        let (title, what) = match act {
            STAKE_BOND => ("TRAVAR STAKE", "Travar"),
            STAKE_UNBOND => ("SAIR DO STAKE", "Iniciar saída de"),
            STAKE_CLAIM => ("RESGATAR STAKE", "Resgatar o que já venceu"),
            _ => return Err(bad("ação inválida")),
        };
        if act == STAKE_CLAIM {
            if amount != 0 {
                return Err(bad("resgatar não leva valor"));
            }
        } else if amount == 0 {
            return Err(bad("valor deve ser positivo"));
        }
        amount
            .checked_add(fee)
            .ok_or_else(|| bad("valor + taxa excede o limite"))?;
        let sk = self
            .master()?
            .signing_key(0)
            .map_err(|_| fail(ErrorCode::Internal, "chave da carteira"))?;
        let sender = public_key_bytes(&sk);
        let mut lines = Vec::new();
        if act == STAKE_CLAIM {
            lines.push(String::from(what));
        } else {
            push_amount(&mut lines, &format!("{what}:"), amount);
        }
        push_amount(&mut lines, "Taxa:", fee);
        lines.push(format!("Rede: {chain}"));
        if act == STAKE_BOND {
            lines.push(String::from("Sujeito a penalidade se"));
            lines.push(String::from("você votar duas vezes."));
        } else if act == STAKE_UNBOND {
            lines.push(String::from("Espera antes do resgate;"));
            lines.push(String::from("ainda sujeito a penalidade."));
        }
        self.confirm(ButtonKind::SignStake, Screen::new(title, lines))?;
        let payload = coin_stake_payload(act, amount);
        let msg = teko_crypto::ledger::tx_signing_bytes(
            &chain,
            nonce,
            fee,
            teko_crypto::ledger::MODULE_COIN,
            &payload,
            &sender,
        );
        let signature = sign_tagged(&sk, tags::LEDGER_TX, &msg);
        Ok(Response::TransferSigned(TransferSigned {
            signature,
            public_key: sender,
            sealed: Vec::new(),
        }))
    }

    /// Builds, SHOWS and signs this wallet's OWN transaction of an asset trade (ADR-0036): the
    /// owner's `Offer` or the buyer's `Accept`. The host supplies only fields; the transaction is
    /// derived here from exactly what the screen shows, so the computer cannot change it.
    fn sign_asset_trade(&mut self, t: &AssetTrade<'_>) -> Result<Response, Failure> {
        use teko_crypto::ledger::{
            MODULE_ASSETS, asset_accept_payload, asset_offer_payload, tx_signing_bytes,
        };
        self.require_unlocked_initialized()?;
        self.require_wallet()?;
        let bad = |msg: &str| fail(ErrorCode::InvalidArgument, msg);
        let clean = |s: &str, max: usize| {
            sanitize(s, max).filter(|c| !c.trim().is_empty() && c.as_str() == s)
        };
        let chain = clean(t.chain_id, 48).ok_or_else(|| bad("rede inválida"))?;
        let class = clean(t.class, 32).ok_or_else(|| bad("classe inválida"))?;
        let asset = clean(t.asset, 64).ok_or_else(|| bad("identificador do bem inválido"))?;
        // the metadata record: valid for the class, naming THIS asset (ADR-0032)
        let record = t.metadata;
        let record_id =
            teko_crypto::asset_meta::asset_id(&class, record).map_err(|e| bad(&e.to_string()))?;
        if record_id != asset {
            return Err(bad("o identificador do bem não confere com os metadados"));
        }
        let summary = teko_crypto::asset_meta::summary_lines(&class, record)
            .map_err(|e| bad(&e.to_string()))?;
        let meta_hash = teko_crypto::asset_meta::hash(&class, record);
        let other = teko_crypto::parse_wallet_address(t.other_address).map_err(|_| {
            bad(if t.accept {
                "endereço do vendedor inválido"
            } else {
                "endereço do comprador inválido"
            })
        })?;
        if t.window_secs == 0 {
            return Err(bad("a janela de veto deve ser positiva"));
        }
        t.price
            .checked_add(t.fee)
            .ok_or_else(|| bad("preço + taxa excede o limite"))?;
        let sk = self
            .master()?
            .signing_key(0)
            .map_err(|_| fail(ErrorCode::Internal, "chave da carteira"))?;
        let pk = public_key_bytes(&sk);
        let me = teko_crypto::ledger_address(&pk);
        if me == other {
            return Err(bad("vendedor e comprador são o mesmo"));
        }
        let (title, kind, role) = if t.accept {
            ("ACEITAR OFERTA", ButtonKind::SignAssetAccept, "COMPRADOR")
        } else {
            ("OFERECER BEM", ButtonKind::SignAssetOffer, "VENDEDOR")
        };
        let mut lines = Vec::from([format!("Você é o {role}")]);
        lines.push(format!("Bem: {class}/{asset}"));
        for l in &summary {
            lines.extend(wrap(l, 28));
        }
        lines.push(String::from(if t.accept {
            "Vendedor:"
        } else {
            "Comprador:"
        }));
        lines.extend(wrap(t.other_address, 22));
        if t.price == 0 {
            lines.push(String::from("Sem pagamento (só o bem)"));
        } else {
            push_amount(&mut lines, "Preço:", t.price);
        }
        if t.accept && t.price > 0 {
            lines.push(String::from("O preço fica em escrow."));
        }
        lines.push(format!("Janela de veto: {}", window_text(t.window_secs)));
        lines.push(String::from("O órgão pode vetar nela;"));
        lines.push(String::from("se vetar, tudo volta."));
        push_amount(&mut lines, "Taxa:", t.fee);
        lines.push(format!("Versão do bem: {}", t.version));
        lines.push(format!("Rede: {chain}"));
        self.confirm(kind, Screen::new(title, lines))?;
        let payload = if t.accept {
            asset_accept_payload(
                class.as_bytes(),
                asset.as_bytes(),
                t.price,
                t.version,
                &meta_hash,
            )
        } else {
            asset_offer_payload(
                class.as_bytes(),
                asset.as_bytes(),
                &other,
                t.price,
                t.version,
                &meta_hash,
            )
        };
        let msg = tx_signing_bytes(&chain, t.nonce, t.fee, MODULE_ASSETS, &payload, &pk);
        let signature = sign_tagged(&sk, tags::LEDGER_TX, &msg);
        Ok(Response::TransferSigned(TransferSigned {
            signature,
            public_key: pk,
            sealed: Vec::new(),
        }))
    }

    /// The credential key of one type, derived from the WALLET (ADR-0039): the same words (and
    /// passphrase) give the same key on any device, so restoring the wallet restores what was issued
    /// to it. The civic identity credential is NOT one of these: it belongs to the identity key.
    fn credential_key_of(&self, ctype: &str) -> Result<SigningKey, Failure> {
        if ctype == CIVIC_TYPE {
            return Err(fail(
                ErrorCode::InvalidArgument,
                "o ID cívico usa a chave de identidade, não a da carteira",
            ));
        }
        self.master()?
            .credential_key(ctype)
            .map_err(|_| fail(ErrorCode::Internal, "chave de credencial"))
    }

    /// Exports the PUBLIC credential key of one type (ADR-0037), after the physical confirmation.
    fn get_credential_key(&mut self, ctype: &str) -> Result<Response, Failure> {
        self.require_unlocked_initialized()?;
        self.require_wallet()?;
        let ctype = clean_ctype(ctype)?;
        let key = self.credential_key_of(&ctype)?;
        let mut lines = Vec::from([String::from("Tipo de credencial:")]);
        lines.extend(wrap(&ctype, 28));
        lines.push(String::from("Exportar a chave PÚBLICA"));
        lines.push(String::from("deste tipo (para o órgão"));
        lines.push(String::from("que vai emitir). Ela volta"));
        lines.push(String::from("com a frase da carteira."));
        self.confirm(
            ButtonKind::CredentialKey,
            Screen::new("CHAVE DE CREDENCIAL", lines),
        )?;
        Ok(Response::CredentialKey(teko_protocol::CredentialKey {
            public_key: public_key_bytes(&key),
        }))
    }

    /// The public credential keys of several types with ONE confirmation (after a wallet restore).
    fn get_credential_keys(&mut self, ctypes: &[String]) -> Result<Response, Failure> {
        self.require_unlocked_initialized()?;
        self.require_wallet()?;
        if ctypes.is_empty() || ctypes.len() > 32 {
            return Err(fail(ErrorCode::InvalidArgument, "de 1 a 32 tipos"));
        }
        let mut entries = Vec::new();
        for c in ctypes {
            let c = clean_ctype(c)?;
            let key = self.credential_key_of(&c)?;
            entries.push((c, public_key_bytes(&key)));
        }
        let mut lines = Vec::from([format!("{} tipos de credencial:", entries.len())]);
        for (c, _) in entries.iter().take(8) {
            lines.extend(wrap(c, 28));
        }
        if entries.len() > 8 {
            lines.push(String::from("…"));
        }
        lines.push(String::from("Exportar as chaves PÚBLICAS"));
        lines.push(String::from("(recuperar o que foi emitido"));
        lines.push(String::from("para esta carteira)."));
        self.confirm(
            ButtonKind::CredentialKey,
            Screen::new("CHAVES DE CREDENCIAL", lines),
        )?;
        Ok(Response::CredentialKeys(teko_protocol::CredentialKeys {
            keys: entries
                .into_iter()
                .map(|(ctype, public_key)| teko_protocol::CredentialKeyEntry { ctype, public_key })
                .collect(),
        }))
    }

    /// Opens the attributes sealed to this wallet's credential key (ADR-0039) and shows them. The
    /// key never leaves the device; the host gets the attributes only after the physical button.
    fn open_credential_bundle(
        &mut self,
        ctype: &str,
        subject: &[u8],
        sealed: &[u8],
    ) -> Result<Response, Failure> {
        self.require_unlocked_initialized()?;
        self.require_wallet()?;
        let ctype = clean_ctype(ctype)?;
        let key = self.credential_key_of(&ctype)?;
        if public_key_bytes(&key) != subject {
            return Err(fail(
                ErrorCode::InvalidArgument,
                "esta credencial não é de uma chave desta carteira",
            ));
        }
        let context = credential_seal_context(&ctype, subject);
        let plain = teko_crypto::seal::open(&key, &context, sealed).map_err(|_| {
            fail(
                ErrorCode::BadSignature,
                "não consegui abrir: não é para esta carteira",
            )
        })?;
        let (salt, claims) = plain
            .split_at_checked(16)
            .ok_or_else(|| fail(ErrorCode::InvalidArgument, "conteúdo malformado"))?;
        let text = core::str::from_utf8(claims)
            .ok()
            .filter(|_| parse_claims(claims).is_some())
            .ok_or_else(|| fail(ErrorCode::InvalidArgument, "atributos malformados"))?;
        let mut lines = Vec::from([String::from("Credencial do tipo:")]);
        lines.extend(wrap(&ctype, 28));
        lines.push(String::from("Atributos:"));
        for part in text.split(';').filter(|p| !p.trim().is_empty()) {
            lines.extend(wrap(part.trim(), 28));
        }
        lines.push(String::from("Mostrar à Suite (para"));
        lines.push(String::from("recuperar a credencial)."));
        self.confirm(
            ButtonKind::OpenCredential,
            Screen::new("ABRIR CREDENCIAL", lines),
        )?;
        Ok(Response::CredentialBundleOpened(
            teko_protocol::CredentialBundleOpened {
                claims: claims.to_vec(),
                salt: salt.to_vec(),
            },
        ))
    }

    /// Builds, SHOWS and signs this wallet's OWN transaction in the credentials module
    /// (ADR-0037): an organ member proposes, approves, vetoes or revokes a credential. The
    /// commitment and the credential id are recomputed HERE from the claims on the screen.
    fn sign_credential_op(
        &mut self,
        m: &teko_protocol::SignCredentialOp,
    ) -> Result<Response, Failure> {
        use teko_crypto::ledger::{
            CRED_OP_APPROVE, CRED_OP_PROPOSE, CRED_OP_REVOKE, CRED_OP_VETO, MODULE_CREDENTIALS,
            cred_id, cred_private_commitment, cred_propose_payload_sealed, cred_public_commitment,
            cred_vote_payload, tx_signing_bytes,
        };
        self.require_unlocked_initialized()?;
        self.require_wallet()?;
        let bad = |msg: &str| fail(ErrorCode::InvalidArgument, msg);
        let chain = sanitize(&m.chain_id, 48)
            .filter(|c| !c.trim().is_empty() && c.as_str() == m.chain_id)
            .ok_or_else(|| bad("rede inválida"))?;
        let ctype = clean_ctype(&m.ctype)?;
        let op = u8::try_from(m.op).map_err(|_| bad("operação inválida"))?;
        let title = match op {
            CRED_OP_PROPOSE => "EMITIR CREDENCIAL",
            CRED_OP_APPROVE => "APROVAR EMISSÃO",
            CRED_OP_VETO => "VETAR CREDENCIAL",
            CRED_OP_REVOKE => "REVOGAR CREDENCIAL",
            _ => return Err(bad("operação inválida")),
        };
        if m.subject.len() != 33 && m.subject.len() != 65 {
            return Err(bad("chave do titular inválida"));
        }
        if m.claims.len() > 2_048 {
            return Err(bad("atributos longos demais"));
        }
        let claims_text = core::str::from_utf8(&m.claims)
            .ok()
            .and_then(|t| sanitize(t, 2_048).filter(|c| c.as_str() == t))
            .ok_or_else(|| bad("atributos inválidos"))?;
        if !m.claims.is_empty() && parse_claims(&m.claims).is_none() {
            return Err(bad(
                "atributos fora do formato nome=valor;nome=valor (nomes únicos)",
            ));
        }
        // the commitment: recomputed here whenever the claims are given
        let (commitment, claims_shown): ([u8; 32], bool) = if m.public {
            if m.claims.is_empty() || !m.commitment.is_empty() {
                return Err(bad("credencial pública leva os atributos"));
            }
            (cred_public_commitment(&m.claims), true)
        } else if !m.claims.is_empty() {
            let salt: [u8; 16] = m
                .salt
                .as_slice()
                .try_into()
                .map_err(|_| bad("sal deve ter 16 bytes"))?;
            (cred_private_commitment(&salt, &m.claims), true)
        } else if op == CRED_OP_PROPOSE {
            return Err(bad("ao emitir, o emissor confere os atributos"));
        } else {
            let c: [u8; 32] = m
                .commitment
                .as_slice()
                .try_into()
                .map_err(|_| bad("compromisso deve ter 32 bytes"))?;
            (c, false)
        };
        let reason = if op == CRED_OP_VETO || op == CRED_OP_REVOKE {
            sanitize(&m.reason, 200)
                .filter(|r| !r.trim().is_empty() && r.as_str() == m.reason)
                .ok_or_else(|| bad("motivo público obrigatório"))?
        } else {
            if !m.reason.is_empty() {
                return Err(bad("só veto e revogação levam motivo"));
            }
            String::new()
        };
        if m.contest_secs == 0 {
            return Err(bad("janela de contestação inválida"));
        }
        let sk = self
            .master()?
            .signing_key(0)
            .map_err(|_| fail(ErrorCode::Internal, "chave da carteira"))?;
        let pk = public_key_bytes(&sk);
        let mut lines = Vec::from([String::from("Você é MEMBRO do órgão")]);
        lines.push(String::from("Tipo:"));
        lines.extend(wrap(&ctype, 28));
        lines.push(String::from("Titular (chave):"));
        lines.extend(hex_lines(
            &m.subject.iter().take(12).copied().collect::<Vec<u8>>(),
            6,
        ));
        if claims_shown {
            lines.push(String::from("Atributos:"));
            for part in claims_text.split(';').filter(|p| !p.trim().is_empty()) {
                lines.extend(wrap(part.trim(), 28));
            }
            if !m.public && ctype != CIVIC_TYPE {
                lines.push(String::from("Os dados vão CIFRADOS ao"));
                lines.push(String::from("titular (só ele abre) e"));
                lines.push(String::from("o compromisso, à chain."));
            } else if !m.public {
                lines.push(String::from("Só o compromisso vai à"));
                lines.push(String::from("chain; os dados ficam"));
                lines.push(String::from("com o titular."));
            }
        } else {
            lines.push(String::from("Atributos NÃO conferidos"));
            lines.push(String::from("aqui. Compromisso:"));
            lines.extend(hex_lines(&commitment[..12], 6));
        }
        if m.valid_until == 0 {
            lines.push(String::from("Validade: sem prazo"));
        } else {
            lines.push(format!("Validade até: {}", ymd(m.valid_until)));
        }
        lines.push(format!(
            "Janela (informada): {}",
            window_text(m.contest_secs)
        ));
        match op {
            CRED_OP_PROPOSE => lines.push(String::from("Conta como 1ª aprovação.")),
            CRED_OP_VETO => lines.push(String::from("Só vale dentro da janela.")),
            CRED_OP_REVOKE => lines.push(String::from("Só vale após a janela.")),
            _ => {}
        }
        if !reason.is_empty() {
            lines.push(String::from("Motivo (público):"));
            lines.extend(wrap(&reason, 28));
        }
        push_amount(&mut lines, "Taxa:", m.fee);
        lines.push(format!("Rede: {chain}"));
        self.confirm(ButtonKind::SignCredentialOp, Screen::new(title, lines))?;
        let mut sealed: Vec<u8> = Vec::new();
        let payload = if op == CRED_OP_PROPOSE {
            let claims_on_chain: &[u8] = if m.public { &m.claims } else { &[] };
            // PRIVATE attributes go to the chain SEALED to the holder (ADR-0039): the holder's
            // device re-derives its credential key from the wallet and opens them after a restore.
            // The civic credential belongs to the identity key, which is not recoverable: no seal.
            if !m.public && claims_shown && ctype != CIVIC_TYPE {
                let mut plain = m.salt.clone();
                plain.extend_from_slice(&m.claims);
                let context = credential_seal_context(&ctype, &m.subject);
                sealed = teko_crypto::seal::seal(&m.subject, &context, &plain, &mut self.rng)
                    .map_err(|_| fail(ErrorCode::InvalidArgument, "chave do titular inválida"))?;
            }
            cred_propose_payload_sealed(
                ctype.as_bytes(),
                &m.subject,
                &commitment,
                claims_on_chain,
                m.valid_until,
                &sealed,
            )
        } else {
            let claims_on_chain: &[u8] = if m.public { &m.claims } else { &[] };
            let id = cred_id(
                ctype.as_bytes(),
                &m.subject,
                &commitment,
                claims_on_chain,
                m.valid_until,
            );
            cred_vote_payload(op, ctype.as_bytes(), &id, &reason)
        };
        let msg = tx_signing_bytes(&chain, m.nonce, m.fee, MODULE_CREDENTIALS, &payload, &pk);
        let signature = sign_tagged(&sk, tags::LEDGER_TX, &msg);
        Ok(Response::TransferSigned(TransferSigned {
            signature,
            public_key: pk,
            sealed,
        }))
    }

    fn get_address(&mut self) -> Result<Response, Failure> {
        self.require_unlocked_initialized()?;
        self.require_wallet()?;
        let master = self.master()?;
        let address = master
            .address_string(0)
            .map_err(|_| fail(ErrorCode::Internal, "endereço"))?;
        let public_key = master
            .public_key(0)
            .map_err(|_| fail(ErrorCode::Internal, "chave pública"))?;
        Ok(Response::Address(teko_protocol::Address {
            address,
            public_key,
        }))
    }

    // ───────────────────────── PIN ─────────────────────────

    fn setup_pin(&mut self) -> Result<Response, Failure> {
        if self.p.state() != DeviceState::FirmwareInstalled {
            return Err(fail(
                ErrorCode::InvalidState,
                "PIN só pode ser definido após instalar o firmware",
            ));
        }
        let first_screen = Screen::new(
            "DEFINA O PIN",
            Vec::from([format!(
                "{} a {} dígitos",
                self.cfg.pin_min_len, self.cfg.pin_max_len
            )]),
        );
        self.hal.show(&first_screen);
        self.hal.notify(&Response::ButtonRequest(ButtonRequest {
            kind: ButtonKind::PinEntry as i32,
        }));
        let pin = self
            .hal
            .read_pin("Defina o PIN")
            .ok_or_else(|| fail(ErrorCode::UserRejected, "PIN cancelado"))?;
        validate_new_pin(&pin, &self.cfg).map_err(|e| match e {
            PinPolicyError::Length => fail(ErrorCode::InvalidArgument, "PIN com tamanho inválido"),
            PinPolicyError::NotDigits => {
                fail(ErrorCode::InvalidArgument, "PIN deve ter só dígitos")
            }
            PinPolicyError::Trivial => fail(ErrorCode::InvalidArgument, "PIN fraco demais"),
        })?;
        self.hal.show(&Screen::new(
            "REPITA O PIN",
            Vec::from([String::from("Digite novamente")]),
        ));
        let again = self
            .hal
            .read_pin("Repita o PIN")
            .ok_or_else(|| fail(ErrorCode::UserRejected, "PIN cancelado"))?;
        if !constant_time_eq(&pin, &again) {
            return Err(fail(ErrorCode::InvalidArgument, "os PINs não coincidem"));
        }

        let mut root = Zeroizing::new([0u8; 32]);
        self.rng.fill_bytes(root.as_mut_slice());
        let mut voter = Zeroizing::new([0u8; 32]);
        self.rng.fill_bytes(voter.as_mut_slice());
        let mut salt = [0u8; 16];
        self.rng.fill_bytes(&mut salt);
        let hash = derive_pin_hash(&pin, &salt, self.cfg.pin_kdf_iterations);

        self.p.k_root = root.to_vec();
        self.p.voter_secret = voter.to_vec();
        self.p.pin_salt = salt.to_vec();
        self.p.pin_hash = hash.to_vec();
        self.p.pin_fail_count = 0;
        self.p.last_fail_ms = 0;
        self.p.state = DeviceState::Initialized as i32;
        self.save()?;
        self.unlocked = true;
        Ok(Response::PinSet(PinSet {}))
    }

    fn unlock(&mut self) -> Result<Response, Failure> {
        if !self.is_pin_set() {
            return Err(fail(ErrorCode::InvalidState, "PIN ainda não definido"));
        }
        if self.unlocked {
            return Ok(Response::Unlocked(Unlocked {}));
        }
        let now = self.hal.now_ms();
        let wait = self.cfg.backoff_ms(self.p.pin_fail_count);
        if wait > 0 {
            let ready_at = self.p.last_fail_ms.saturating_add(wait);
            if now < ready_at {
                let mut f = fail(ErrorCode::PinLocked, "aguarde antes de tentar novamente");
                f.retry_after_ms = ready_at - now;
                return Err(f);
            }
        }
        let remaining = self
            .cfg
            .max_pin_attempts
            .saturating_sub(self.p.pin_fail_count);
        let mut lines = Vec::from([format!("Tentativas restantes: {remaining}")]);
        if remaining <= 1 {
            lines.push(String::from("ÚLTIMA TENTATIVA!"));
            lines.push(String::from("Se errar, o dispositivo APAGA"));
            lines.push(String::from("chaves e credenciais."));
        } else if remaining <= 3 {
            lines.push(String::from("ATENÇÃO: errar demais APAGA"));
            lines.push(String::from("o dispositivo."));
        }
        self.hal.show(&Screen::new("DIGITE O PIN", lines));
        self.hal.notify(&Response::ButtonRequest(ButtonRequest {
            kind: ButtonKind::PinEntry as i32,
        }));
        let pin = self
            .hal
            .read_pin("PIN")
            .ok_or_else(|| fail(ErrorCode::UserRejected, "PIN cancelado"))?;

        // Count the attempt and persist it BEFORE comparing: cutting power
        // mid-check cannot be used to get free guesses.
        self.p.pin_fail_count = self.p.pin_fail_count.saturating_add(1);
        self.p.last_fail_ms = now;
        self.save()?;

        if pin_matches(
            &pin,
            &self.p.pin_salt,
            &self.p.pin_hash,
            self.cfg.pin_kdf_iterations,
        ) {
            self.p.pin_fail_count = 0;
            self.save()?;
            self.unlocked = true;
            return Ok(Response::Unlocked(Unlocked {}));
        }
        if self.p.pin_fail_count >= self.cfg.max_pin_attempts {
            self.wipe_internal()?;
            self.hal.show(&Screen::new(
                "DISPOSITIVO APAGADO",
                Vec::from([
                    String::from("Tentativas esgotadas"),
                    String::from("Reemissão presencial necessária"),
                ]),
            ));
            return Err(fail(
                ErrorCode::DeviceWiped,
                "dispositivo apagado por excesso de tentativas",
            ));
        }
        Ok(Response::PinWrong(PinWrong {
            remaining: self.cfg.max_pin_attempts - self.p.pin_fail_count,
        }))
    }

    // ───────────────────────── identifiers ─────────────────────────

    fn get_civic_id(&mut self) -> Result<Response, Failure> {
        self.require_unlocked_initialized()?;
        self.require_wallet()?;
        let pk = public_key_bytes(&self.identity_key()?);
        let id = civic_id(&pk).map_err(|_| fail(ErrorCode::Internal, "id cívico"))?;
        let mut lines = Vec::from([String::from("Seu ID Cívico:")]);
        lines.extend(wrap(&id, 20));
        if self.p.state() == DeviceState::Initialized {
            // Onboarding: the user must compare this with the Suite.
            lines.push(String::from("Confira com a Suite"));
            self.confirm(
                ButtonKind::ConfirmCivicId,
                Screen::new("CONFERIR ID CÍVICO", lines),
            )?;
        } else {
            self.hal.show(&Screen::new("ID CÍVICO", lines));
        }
        Ok(Response::CivicId(CivicId {
            id,
            pk_identity: pk,
        }))
    }

    fn get_pairwise_id(&mut self, service: &str) -> Result<Response, Failure> {
        self.require_ready()?;
        let svc = canonical_service_id(service)
            .map_err(|_| fail(ErrorCode::InvalidArgument, "serviço inválido"))?;
        let mut lines = Vec::from([String::from("Gerar ID exclusivo para:")]);
        lines.extend(wrap(&svc, 20));
        self.confirm(ButtonKind::PairwiseId, Screen::new("ID POR SERVIÇO", lines))?;
        let root = self.root()?;
        let k = derive_key(&root, tags::PAIRWISE);
        let id = pairwise_id(&k, &svc).map_err(|_| fail(ErrorCode::Internal, "id por serviço"))?;
        Ok(Response::PairwiseId(PairwiseId { id }))
    }

    // ───────────────────────── signing ─────────────────────────

    fn sign_document(&mut self, hash: &[u8], summary: &str) -> Result<Response, Failure> {
        self.require_ready()?;
        if hash.len() != 32 {
            return Err(fail(ErrorCode::InvalidArgument, "hash deve ter 32 bytes"));
        }
        let summary = sanitize(summary, 120)
            .ok_or_else(|| fail(ErrorCode::InvalidArgument, "resumo inválido"))?;
        let mut lines = Vec::from([
            String::from("Resumo (informado pelo"),
            String::from("computador, NÃO verificado):"),
        ]);
        lines.extend(wrap(&summary, 28));
        lines.push(String::from("Hash SHA3-256 (verificado):"));
        lines.extend(hex_lines(hash, 8));
        self.confirm(
            ButtonKind::SignDocument,
            Screen::new("ASSINAR DOCUMENTO?", lines),
        )?;
        let key = self.identity_key()?;
        let signature = sign_tagged(&key, tags::DOC_SIGN, hash);
        Ok(Response::DocumentSignature(DocumentSignature {
            signature,
            pk_identity: public_key_bytes(&key),
        }))
    }

    fn prove_attribute(
        &mut self,
        predicate: Option<Predicate>,
        nonce: &[u8],
        verifier: &str,
    ) -> Result<Response, Failure> {
        self.require_ready()?;
        let predicate =
            predicate.ok_or_else(|| fail(ErrorCode::InvalidArgument, "predicado ausente"))?;
        if !(8..=64).contains(&nonce.len()) {
            return Err(fail(
                ErrorCode::InvalidArgument,
                "nonce deve ter 8 a 64 bytes",
            ));
        }
        let verifier = sanitize(verifier, 60)
            .filter(|v| !v.trim().is_empty())
            .ok_or_else(|| fail(ErrorCode::InvalidArgument, "nome do verificador inválido"))?;
        self.require_civic_bundle()?;
        let (bundle, claims) = self
            .civic_claims()
            .ok_or_else(|| fail(ErrorCode::Internal, "credencial ilegível"))?;

        let (claim_name, description) = resolve_predicate(&predicate, &claims)?;
        let mut lines = Vec::from([String::from("Para:")]);
        lines.extend(wrap(&verifier, 28));
        lines.push(String::from("Será revelado:"));
        lines.extend(wrap(&description, 28));
        lines.push(String::from("Nada mais é revelado"));
        self.confirm(
            ButtonKind::ProveAttribute,
            Screen::new("PROVAR ATRIBUTO?", lines),
        )?;

        let key = self.identity_key()?;
        let subject = public_key_bytes(&key);
        let commitment = teko_crypto::civic::commitment_of(&claims);
        let id = teko_crypto::ledger::cred_id(
            CIVIC_TYPE.as_bytes(),
            &subject,
            &commitment,
            &[],
            bundle.valid_until,
        );
        let disclosed: Vec<Claim> = claims
            .iter()
            .filter(|c| c.name == claim_name)
            .cloned()
            .collect();
        let hidden: Vec<Vec<u8>> = claims
            .iter()
            .filter(|c| c.name != claim_name)
            .map(|c| claim_digest(c).to_vec())
            .collect();
        let mut presentation = CivicPresentation {
            chain_id: bundle.chain_id.clone(),
            ctype: String::from(CIVIC_TYPE),
            credential_id: id.to_vec(),
            subject,
            valid_until: bundle.valid_until,
            disclosed,
            hidden,
            nonce: nonce.to_vec(),
            verifier,
            holder_signature: Vec::new(),
        };
        presentation.holder_signature = sign_tagged(
            &key,
            tags::ATTR_PROOF,
            &presentation_signing_bytes(&presentation),
        );
        Ok(Response::AttributeProof(AttributeProof {
            presentation: presentation.encode_to_vec(),
        }))
    }

    /// A device onboarded before the civic ID moved to the organ (ADR-0038) keeps a credential in
    /// the OLD format: nothing that needs the ID can use it, and the answer must say what to do.
    fn require_civic_bundle(&self) -> Result<(), Failure> {
        if self.civic_claims().is_some() {
            return Ok(());
        }
        Err(fail(
            ErrorCode::LegacyCredential,
            "seu ID cívico está no formato antigo: rode `make reset`, refaça o onboarding e peça a reemissão ao órgão",
        ))
    }

    /// The stored civic credential: the bundle and its claims (with their salts).
    fn civic_claims(&self) -> Option<(CivicBundle, Vec<Claim>)> {
        let bundle = CivicBundle::decode(self.p.credential.as_slice()).ok()?;
        let salt: [u8; 16] = bundle.salt.as_slice().try_into().ok()?;
        let claims = claim_list(&salt, &bundle.claims);
        Some((bundle, claims))
    }

    // ───────────────────────── elections (Phase 2) ─────────────────────────

    fn governance(&self) -> Result<GovernancePolicy, Failure> {
        GovernancePolicy::new(self.trust.gov_signers.clone(), self.trust.gov_threshold).map_err(
            |_| {
                fail(
                    ErrorCode::InvalidState,
                    "eleições desativadas: sem governança configurada",
                )
            },
        )
    }

    /// Electoral domicile from the credential (issued by the authority).
    fn domicile(&self) -> Option<Domicile> {
        let (_, claims) = self.civic_claims()?;
        let claim = |n: &str| claims.iter().find(|c| c.name == n).map(|c| c.value.clone());
        Domicile::new(
            &claim("electoral_state")?,
            &claim("electoral_municipality")?,
        )
        .ok()
    }

    fn eligible_for(&self, scope: &teko_election::Scope) -> bool {
        match scope.level() {
            ScopeLevel::National => true,
            _ => self.domicile().is_some_and(|d| is_eligible(&d, scope)),
        }
    }

    /// The stored election, RE-VERIFIED (M-of-N + history replay) on every use:
    /// the persisted bytes are never trusted just because they are stored.
    fn stored_election(&self) -> Result<(ElectionManifest, ElectionStatus), Failure> {
        if self.p.election.is_empty() {
            return Err(fail(ErrorCode::InvalidState, "nenhuma eleição carregada"));
        }
        let bundle = ElectionBundle::decode(self.p.election.as_slice())
            .map_err(|_| fail(ErrorCode::Internal, "eleição armazenada ilegível"))?;
        self.verify_bundle(&bundle.manifest, &bundle.transitions)
    }

    fn verify_bundle(
        &self,
        manifest_bytes: &[u8],
        transition_bytes: &[Vec<u8>],
    ) -> Result<(ElectionManifest, ElectionStatus), Failure> {
        let policy = self.governance()?;
        let manifest = ElectionManifest::decode(manifest_bytes)
            .map_err(|_| fail(ErrorCode::InvalidArgument, "manifesto malformado"))?;
        // A device bound to a network refuses elections of any other network.
        if !self.trust.chain_id.is_empty() && manifest.chain_id != self.trust.chain_id {
            return Err(fail(ErrorCode::InvalidArgument, "eleição de outra rede"));
        }
        let mut transitions = Vec::with_capacity(transition_bytes.len());
        for t in transition_bytes {
            transitions.push(
                Transition::decode(t.as_slice())
                    .map_err(|_| fail(ErrorCode::InvalidArgument, "transição malformada"))?,
            );
        }
        let status = replay(&policy, &manifest, &transitions).map_err(election_failure)?;
        Ok((manifest, status))
    }

    fn contest_infos(
        &self,
        manifest: &ElectionManifest,
        status: &ElectionStatus,
    ) -> Vec<ContestInfo> {
        manifest
            .contests
            .iter()
            .map(|c| {
                let scope = c.scope.clone().unwrap_or_default();
                let sched = c.schedule.clone().unwrap_or_default();
                ContestInfo {
                    contest_id: c.contest_id.clone(),
                    title: c.title.clone(),
                    phase: status.phase_of(&c.contest_id).unwrap_or(Phase::Draft) as u32,
                    scope_level: scope.level as u32,
                    scope_code: scope.code.clone(),
                    option_names: c.options.iter().map(|o| o.name.clone()).collect(),
                    eligible: self.eligible_for(&scope),
                    voted: self
                        .p
                        .ballots_signed
                        .iter()
                        .any(|h| h.as_slice() == contest_hash(c).as_slice()),
                    voting_opens: sched.voting_opens,
                    voting_closes: sched.voting_closes,
                    registration_required: !c.registration_key.is_empty(),
                    option_numbers: c.options.iter().map(|o| o.number.clone()).collect(),
                    option_kinds: c.options.iter().map(|o| o.kind as u32).collect(),
                    seats: c.seat_count() as u32,
                    parent: c.parent.clone(),
                }
            })
            .collect()
    }

    fn load_election(
        &mut self,
        manifest_bytes: &[u8],
        transition_bytes: &[Vec<u8>],
    ) -> Result<Response, Failure> {
        self.require_ready()?;
        if manifest_bytes.len() > 32 * 1024
            || transition_bytes.len() > 256
            || transition_bytes.iter().any(|t| t.len() > 4 * 1024)
        {
            return Err(fail(ErrorCode::InvalidArgument, "eleição grande demais"));
        }
        let (manifest, status) = self.verify_bundle(manifest_bytes, transition_bytes)?;

        // Anti-rollback: never accept an OLDER view of an election we already
        // know (e.g. a replayed history in which a closed contest is still open).
        // A stored election that no longer verifies (e.g. written by an older
        // format) cannot be compared against, and must never block loading a good one.
        let known = (!self.p.election.is_empty())
            .then(|| self.stored_election().ok())
            .flatten();
        if let Some((old, old_status)) = known
            && old.election_id == manifest.election_id
        {
            {
                if manifest.revision < old.revision || status.seq < old_status.seq {
                    return Err(fail(
                        ErrorCode::Downgrade,
                        "histórico da eleição mais antigo que o já conhecido",
                    ));
                }
                if manifest.revision == old.revision
                    && manifest_hash(&manifest) != manifest_hash(&old)
                {
                    return Err(fail(
                        ErrorCode::InvalidArgument,
                        "manifesto diferente na mesma revisão",
                    ));
                }
                if manifest.revision > old.revision {
                    validate_revision(&old, &manifest).map_err(election_failure)?;
                }
            }
        }

        let policy = self.governance()?;
        let infos = self.contest_infos(&manifest, &status);
        let mut lines = Vec::from([String::from("Eleição:")]);
        lines.extend(wrap(&manifest.title, 28));
        lines.push(format!(
            "Governança: {} de {}",
            policy.threshold(),
            policy.signers().len()
        ));
        for i in infos.iter().take(6) {
            let tag = if i.eligible {
                "voce vota"
            } else {
                "fora do seu domicilio"
            };
            lines.extend(wrap(&format!("- {} [{}]", i.title, tag), 28));
        }
        if infos.len() > 6 {
            lines.push(format!("(+{} pleitos)", infos.len() - 6));
        }
        self.confirm(
            ButtonKind::LoadElection,
            Screen::new("CARREGAR ELEIÇÃO?", lines),
        )?;

        self.p.election = ElectionBundle {
            manifest: manifest_bytes.to_vec(),
            transitions: transition_bytes.to_vec(),
        }
        .encode_to_vec();
        self.save()?;
        Ok(Response::ElectionLoaded(ElectionLoaded {
            election_id: manifest.election_id.clone(),
            title: manifest.title.clone(),
            revision: manifest.revision,
            seq: status.seq,
            contests: infos,
        }))
    }

    fn get_election(&mut self) -> Result<Response, Failure> {
        self.require_ready()?;
        let (manifest, status) = self.stored_election()?;
        Ok(Response::ElectionLoaded(ElectionLoaded {
            election_id: manifest.election_id.clone(),
            title: manifest.title.clone(),
            revision: manifest.revision,
            seq: status.seq,
            contests: self.contest_infos(&manifest, &status),
        }))
    }

    fn voter_root(&self) -> Result<Zeroizing<[u8; 32]>, Failure> {
        self.voter_secret()
    }

    fn get_voter_key(&mut self, election_id: &str, contest_id: &str) -> Result<Response, Failure> {
        self.require_ready()?;
        let (manifest, status) = self.stored_election()?;
        if manifest.election_id != election_id {
            return Err(fail(ErrorCode::InvalidArgument, "outra eleição"));
        }
        let contest = manifest
            .contest(contest_id)
            .ok_or_else(|| fail(ErrorCode::InvalidArgument, "pleito desconhecido"))?;
        let scope = contest.scope.clone().unwrap_or_default();
        if !self.eligible_for(&scope) {
            return Err(fail(
                ErrorCode::NotEligible,
                "fora do seu domicílio eleitoral",
            ));
        }
        let phase = status.phase_of(contest_id).unwrap_or(Phase::Draft);
        if !phase.can_register() {
            return Err(fail(
                ErrorCode::WrongPhase,
                "registro não está aberto neste pleito",
            ));
        }
        let mut lines = Vec::from([String::from("Pleito:")]);
        lines.extend(wrap(&contest.title, 28));
        lines.push(jurisdiction_label(&scope));
        lines.push(String::from("Exportar a chave PÚBLICA"));
        lines.push(String::from("de eleitor deste pleito."));
        self.confirm(ButtonKind::VoterKey, Screen::new("CHAVE DE ELEITOR", lines))?;
        let root = self.voter_root()?;
        let key = voter::derive_voter_key(&root, election_id, contest_id)
            .map_err(|_| fail(ErrorCode::Internal, "chave de eleitor"))?;
        Ok(Response::VoterKey(VoterKey {
            voter_key: public_key_bytes(&key),
        }))
    }

    /// The deadline is enforced by the DEVICE too, not only by governance: after
    /// `voting_closes` it neither registers nor signs, even if no closing
    /// transition has been published yet.
    fn past_deadline(&self, contest: &teko_election::Contest) -> bool {
        let closes = contest
            .schedule
            .as_ref()
            .map_or(u64::MAX, |s| s.voting_closes);
        self.hal.now_ms() / 1000 > closes
    }

    /// Shared checks for both registration steps.
    fn registration_context(
        &self,
        election_id: &str,
        contest_id: &str,
    ) -> Result<teko_election::Contest, Failure> {
        self.require_ready()?;
        let (manifest, status) = self.stored_election()?;
        if manifest.election_id != election_id {
            return Err(fail(ErrorCode::InvalidArgument, "outra eleição"));
        }
        let contest = manifest
            .contest(contest_id)
            .cloned()
            .ok_or_else(|| fail(ErrorCode::InvalidArgument, "pleito desconhecido"))?;
        let scope = contest.scope.clone().unwrap_or_default();
        if !self.eligible_for(&scope) {
            return Err(fail(
                ErrorCode::NotEligible,
                "fora do seu domicílio eleitoral",
            ));
        }
        if contest.registration_key.is_empty() {
            return Err(fail(
                ErrorCode::InvalidArgument,
                "este pleito não exige registro",
            ));
        }
        if self.past_deadline(&contest) {
            return Err(fail(ErrorCode::WrongPhase, "prazo encerrado neste pleito"));
        }
        let phase = status.phase_of(contest_id).unwrap_or(Phase::Draft);
        if !phase.can_register() {
            return Err(fail(
                ErrorCode::WrongPhase,
                "registro não está aberto neste pleito",
            ));
        }
        Ok(contest)
    }

    /// The blinding seed: secret, per contest, unrelated to the identity key.
    fn blind_seed(
        &self,
        election_id: &str,
        contest_id: &str,
    ) -> Result<Zeroizing<[u8; 32]>, Failure> {
        let root = self.voter_root()?;
        Ok(Zeroizing::new(teko_crypto::tagged_hash(
            tags::REGISTRATION_BLIND,
            &[
                root.as_slice(),
                election_id.as_bytes(),
                contest_id.as_bytes(),
            ],
        )))
    }

    fn begin_registration(
        &mut self,
        election_id: &str,
        contest_id: &str,
    ) -> Result<Response, Failure> {
        let contest = self.registration_context(election_id, contest_id)?;
        self.require_civic_bundle()?;
        let scope = contest.scope.clone().unwrap_or_default();
        let mut lines = Vec::from([String::from("Pleito:")]);
        lines.extend(wrap(&contest.title, 28));
        lines.push(jurisdiction_label(&scope));
        lines.push(String::from("A autoridade saberá QUEM você"));
        lines.push(String::from("é, mas NÃO a sua chave de"));
        lines.push(String::from("eleitor (assinatura cega)."));
        self.confirm(
            ButtonKind::Register,
            Screen::new("REGISTRAR PARA VOTAR?", lines),
        )?;

        let root = self.voter_root()?;
        let key = voter::derive_voter_key(&root, election_id, contest_id)
            .map_err(|_| fail(ErrorCode::Internal, "chave de eleitor"))?;
        let message = registration_message(election_id, contest_id, &public_key_bytes(&key));
        let seed = self.blind_seed(election_id, contest_id)?;
        let blind_message = self
            .blinder
            .as_ref()
            .ok_or_else(|| fail(ErrorCode::NotImplemented, "registro anônimo não suportado"))?
            .blind(&contest.registration_key, &message, &seed)
            .ok_or_else(|| fail(ErrorCode::Internal, "cegamento falhou"))?;
        let id = self.identity_key()?;
        let pop_signature = sign_tagged(
            &id,
            tags::REGISTRATION_REQUEST,
            &registration_request_message(election_id, contest_id, &blind_message),
        );
        Ok(Response::RegistrationRequest(RegistrationRequest {
            blind_message,
            pk_identity: public_key_bytes(&id),
            pop_signature,
            credential: self.p.credential.clone(),
        }))
    }

    fn finish_registration(
        &mut self,
        election_id: &str,
        contest_id: &str,
        blind_signature: &[u8],
    ) -> Result<Response, Failure> {
        let contest = self.registration_context(election_id, contest_id)?;
        let root = self.voter_root()?;
        let key = voter::derive_voter_key(&root, election_id, contest_id)
            .map_err(|_| fail(ErrorCode::Internal, "chave de eleitor"))?;
        let voter_key = public_key_bytes(&key);
        let message = registration_message(election_id, contest_id, &voter_key);
        let seed = self.blind_seed(election_id, contest_id)?;
        // `unblind` CHECKS the signature: a bad reply never becomes a token.
        let token = self
            .blinder
            .as_ref()
            .ok_or_else(|| fail(ErrorCode::NotImplemented, "registro anônimo não suportado"))?
            .unblind(&contest.registration_key, &message, &seed, blind_signature)
            .ok_or_else(|| fail(ErrorCode::BadSignature, "resposta da autoridade inválida"))?;
        Ok(Response::RegistrationToken(RegistrationToken {
            voter_key,
            token,
        }))
    }

    fn sign_ballot(
        &mut self,
        election_id: &str,
        contest_id: &str,
        option_index: u32,
        option_indices: &[u32],
    ) -> Result<Response, Failure> {
        self.require_ready()?;
        let (manifest, status) = self.stored_election()?;
        if manifest.election_id != election_id {
            return Err(fail(ErrorCode::InvalidArgument, "outra eleição"));
        }
        let contest = manifest
            .contest(contest_id)
            .ok_or_else(|| fail(ErrorCode::InvalidArgument, "pleito desconhecido"))?;
        let scope = contest.scope.clone().unwrap_or_default();
        if !self.eligible_for(&scope) {
            return Err(fail(
                ErrorCode::NotEligible,
                "fora do seu domicílio eleitoral",
            ));
        }
        if self.past_deadline(contest) {
            return Err(fail(ErrorCode::WrongPhase, "prazo encerrado neste pleito"));
        }
        let phase = status.phase_of(contest_id).unwrap_or(Phase::Draft);
        if !phase.can_vote() {
            return Err(fail(
                ErrorCode::WrongPhase,
                "a votação não está aberta neste pleito",
            ));
        }
        let chash = contest_hash(contest);
        if self
            .p
            .ballots_signed
            .iter()
            .any(|h| h.as_slice() == chash.as_slice())
        {
            return Err(fail(
                ErrorCode::AlreadyVoted,
                "este dispositivo já votou neste pleito",
            ));
        }
        // WHAT the voter chose: one option (one seat) or up to `seats` DISTINCT ones (ADR-0044)
        let wanted: Vec<u32> = if option_indices.is_empty() {
            Vec::from([option_index])
        } else if option_index != 0 {
            return Err(fail(
                ErrorCode::InvalidArgument,
                "use option_index ou option_indices, não os dois",
            ));
        } else {
            option_indices.to_vec()
        };
        let seats = contest.seat_count();
        let mut positions: Vec<usize> = Vec::new();
        for w in &wanted {
            let pos = contest
                .options
                .iter()
                .position(|o| o.index == *w)
                .ok_or_else(|| fail(ErrorCode::InvalidArgument, "opção inexistente"))?;
            if positions.contains(&pos) {
                return Err(fail(ErrorCode::InvalidArgument, "a mesma opção duas vezes"));
            }
            positions.push(pos);
        }
        let options: Vec<&teko_election::ElectionOption> = positions
            .iter()
            .filter_map(|p| contest.options.get(*p))
            .collect();
        let specials = options.iter().filter(|o| !o.is_candidate()).count();
        let candidates = options.len() - specials;
        // 1..=seats candidates, OR exactly one blank/null alone (the ballot proof enforces the same)
        let rule_ok = (specials == 1 && candidates == 0)
            || (specials == 0 && (1..=seats).contains(&candidates));
        if !rule_ok {
            return Err(fail(
                ErrorCode::InvalidArgument,
                "escolha de 1 até o número de vagas em candidatos distintos, ou só branco, ou só nulo",
            ));
        }
        if seats > 1 && manifest.tally_key.is_empty() {
            return Err(fail(
                ErrorCode::InvalidArgument,
                "pleito de várias vagas exige eleição cifrada",
            ));
        }

        // EVERYTHING shown comes from the verified manifest, never from the host.
        let mut lines = Vec::from([String::from("Eleição:")]);
        lines.extend(wrap(&manifest.title, 28));
        lines.push(String::from("Pleito:"));
        lines.extend(wrap(&contest.title, 28));
        lines.push(jurisdiction_label(&scope));
        if seats > 1 {
            lines.push(format!(
                "SEUS VOTOS ({} de {} vagas):",
                options.len(),
                seats
            ));
        } else {
            lines.push(String::from("SEU VOTO:"));
        }
        for option in &options {
            // blank and null are shown with the DEVICE's own words, never the manifest's (ADR-0042)
            let label = match option.kind() {
                OptionKind::Blank => String::from("VOTO EM BRANCO"),
                OptionKind::Null => String::from("VOTO NULO"),
                OptionKind::Candidate if option.number.is_empty() => option.name.clone(),
                OptionKind::Candidate => format!("{} - {}", option.number, option.name),
            };
            lines.extend(wrap(&label, 28));
            if !option.is_candidate() {
                lines.push(String::from("(não conta para nenhum"));
                lines.push(String::from("candidato)"));
            }
        }
        let encrypted_mode = !manifest.tally_key.is_empty();
        if encrypted_mode {
            lines.push(String::from("Voto CIFRADO: ninguém vê"));
            lines.push(String::from("o resultado antes do fim."));
        } else {
            lines.push(String::from("(protótipo: voto em claro,"));
            lines.push(String::from("sem sigilo até o fim)"));
        }
        self.confirm(
            ButtonKind::SignBallot,
            Screen::new("CONFIRMAR VOTO?", lines),
        )?;

        let root = self.voter_root()?;
        let key = voter::derive_voter_key(&root, election_id, contest_id)
            .map_err(|_| fail(ErrorCode::Internal, "chave de eleitor"))?;
        let voter_pk = public_key_bytes(&key);
        // In encrypted mode the DEVICE encrypts and proves: the host only gets ciphertexts. The
        // signed body carries the ciphertext hash, never the option.
        let (body_option, ciphertext_hash, encrypted) = if encrypted_mode {
            let pk = teko_ballot_crypto::point_from_bytes(&manifest.tally_key)
                .ok_or_else(|| fail(ErrorCode::Internal, "chave da eleição inválida"))?;
            let ctx = ballot_ctx(election_id, contest_id, &voter_pk);
            let special = contest.special_positions();
            let shape = teko_ballot_crypto::BallotShape {
                n: contest.options.len(),
                seats,
                special: &special,
            };
            let (eb, mut rs) = teko_ballot_crypto::encrypt_ballot_multi_with(
                &pk,
                &shape,
                &positions,
                &ctx,
                &mut self.rng,
            )
            .ok_or_else(|| fail(ErrorCode::Internal, "cifra da cédula falhou"))?;
            let bytes = eb.to_bytes();
            let hash = teko_election::ciphertext_hash(&bytes);
            // CAST OR AUDIT (ADR-0041): the device COMMITS to this ciphertext first (the host gets
            // it now), and only then does the voter choose, on the device. A device that encrypted
            // something else cannot know whether this ballot will be audited.
            self.hal.notify(&Response::BallotCommitment(
                teko_protocol::BallotCommitment {
                    ciphertext_hash: hash.clone(),
                    encrypted: bytes.clone(),
                },
            ));
            let cast = match self.confirm(
                ButtonKind::BallotCast,
                Screen::new(
                    "VOTAR OU AUDITAR?",
                    Vec::from([
                        String::from("CONFIRMAR = VOTAR: esta"),
                        String::from("cédula será o seu voto."),
                        String::from("RECUSAR = voltar e, se"),
                        String::from("quiser, AUDITAR esta"),
                        String::from("cédula de teste."),
                    ]),
                ),
            ) {
                Ok(()) => true,
                Err(f) if f.code() == ErrorCode::UserRejected => false,
                Err(f) => {
                    rs.zeroize();
                    return Err(f);
                }
            };
            if !cast {
                let audit = self.confirm(
                    ButtonKind::BallotAudit,
                    Screen::new(
                        "AUDITAR CÉDULA?",
                        Vec::from([
                            String::from("Esta cédula NÃO será"),
                            String::from("votada: o dispositivo"),
                            String::from("mostrará ao computador"),
                            String::from("em quem ela votava e"),
                            String::from("como a cifrou, para ele"),
                            String::from("conferir. Depois você"),
                            String::from("poderá votar de verdade."),
                        ]),
                    ),
                );
                return match audit {
                    Ok(()) => {
                        let randomness: Vec<Vec<u8>> = rs
                            .iter()
                            .map(|r| teko_ballot_crypto::scalar_to_bytes(r).to_vec())
                            .collect();
                        rs.zeroize();
                        // the voting record is untouched: auditing does not use the vote
                        Ok(Response::BallotAudited(teko_protocol::BallotAudited {
                            option_position: positions.first().map_or(0, |p| *p as u32),
                            randomness,
                            voter_key: voter_pk.clone(),
                            option_positions: positions.iter().map(|p| *p as u32).collect(),
                        }))
                    }
                    Err(f) => {
                        rs.zeroize();
                        Err(f)
                    }
                };
            }
            // CAST: the randomness is wiped and never leaves the device
            rs.zeroize();
            (0, hash, bytes)
        } else {
            (wanted.first().copied().unwrap_or(0), Vec::new(), Vec::new())
        };
        let signature = sign_ballot(
            &key,
            &BallotBody {
                election_id: String::from(election_id),
                contest_id: String::from(contest_id),
                contest_hash: chash.to_vec(),
                option_index: body_option,
                ciphertext_hash,
            },
        );
        // What the board will hold, and its Merkle leaf: public, so safe to keep. It lets the
        // device check later, ALONE, that this very ballot is recorded (ADR-0041).
        let entry = teko_election::SubmittedBallot {
            election_id: String::from(election_id),
            contest_id: String::from(contest_id),
            contest_hash: chash.to_vec(),
            option_index: body_option,
            voter_key: voter_pk.clone(),
            signature: signature.clone(),
            encrypted: encrypted.clone(),
        };
        let mut leaf_data = Vec::from([BOARD_BALLOT_KIND]);
        leaf_data.extend_from_slice(&entry.encode_to_vec());
        let leaf = tagged_hash(tags::BULLETIN_LEAF, &[leaf_data.as_slice()]);
        // Record BEFORE releasing the signature: a crash can lose the delivery,
        // but can never leave a signature out that the device forgot it made.
        self.p.ballots_signed.push(chash.to_vec());
        self.p.vote_records.push(crate::state::VoteRecord {
            election_id: String::from(election_id),
            contest_id: String::from(contest_id),
            contest_hash: chash.to_vec(),
            leaf_hash: leaf.to_vec(),
        });
        self.save()?;
        // the tracker is on the screen too: the voter can compare it later
        self.hal.show(&Screen::new(
            "VOTO REGISTRADO",
            Vec::from([
                String::from("Rastreador da sua cédula:"),
                teko_crypto::tracker(&leaf),
                String::from("(só identifica a cédula,"),
                String::from("não revela o voto)"),
            ]),
        ));
        Ok(Response::BallotSignature(BallotSignature {
            signature,
            voter_key: voter_pk,
            encrypted,
        }))
    }

    /// Checks, ALONE, that the ballot this device cast is in a block of the vote chain certified by
    /// more than 2/3 of the institutions in the trust file (ADR-0041). Nothing is taken on trust
    /// from the host: the certificate, the chain, the block hash and the Merkle proof are all
    /// re-verified here against the leaf hash the device itself kept. The answer carries only
    /// public facts; the option never leaves the device (it is not even stored).
    fn check_my_ballot(&mut self, m: &teko_protocol::CheckMyBallot) -> Result<Response, Failure> {
        self.require_ready()?;
        let rec = self
            .p
            .vote_records
            .iter()
            .find(|r| r.election_id == m.election_id && r.contest_id == m.contest_id)
            .cloned()
            .ok_or_else(|| fail(ErrorCode::InvalidArgument, "este dispositivo não votou aí"))?;
        if self.trust.vote_chain_id.is_empty() {
            return Err(fail(
                ErrorCode::InvalidState,
                "este dispositivo não confia em nenhuma rede de votos",
            ));
        }
        let leaf: [u8; 32] = rec
            .leaf_hash
            .as_slice()
            .try_into()
            .map_err(|_| fail(ErrorCode::Internal, "registro do voto corrompido"))?;
        self.confirm(
            ButtonKind::CheckBallot,
            Screen::new(
                "CONFERIR MINHA CÉDULA?",
                Vec::from([
                    String::from("O dispositivo vai conferir,"),
                    String::from("sozinho, que a sua cédula"),
                    String::from("está no quadro, certificada"),
                    String::from("pelas instituições."),
                    String::from("O seu voto não é mostrado."),
                ]),
            ),
        )?;
        let bad = |why: &'static str| fail(ErrorCode::BadSignature, why);
        let commit = <teko_bft::Commit as prost::Message>::decode(m.commit.as_slice())
            .map_err(|_| bad("certificado malformado"))?;
        // WHICH institutions certified this height: the set of the trust file, followed link by link
        // through every approved change the host brought (ADR-0045)
        let Institutions {
            keys,
            epoch,
            final_epoch,
            final_hash,
        } = self.institutions_at(commit.height, &m.rotations)?;
        let validators: Vec<teko_bft::Validator> = keys
            .iter()
            .map(|k| teko_bft::Validator {
                key: k.clone(),
                power: 1,
            })
            .collect();
        let set = teko_bft::ValidatorSet::new(validators).map_err(|_| {
            fail(
                ErrorCode::Internal,
                "instituições da rede de votos inválidas",
            )
        })?;
        if !commit.verify(&self.trust.vote_chain_id, &set) {
            return Err(bad(
                "o certificado NÃO tem mais de 2/3 das instituições em que o dispositivo confia",
            ));
        }
        if teko_bft::block_id(&m.block) != commit.block_id {
            return Err(bad("o bloco não é o que as instituições certificaram"));
        }
        let view = <BlockView as prost::Message>::decode(m.block.as_slice())
            .map_err(|_| bad("bloco malformado"))?;
        if view.chain_id != self.trust.vote_chain_id || view.height != commit.height {
            return Err(bad("bloco de outra rede ou de outra altura"));
        }
        let root: [u8; 32] = view
            .log_root
            .as_slice()
            .try_into()
            .map_err(|_| bad("raiz do quadro inválida"))?;
        let proof: Option<Vec<[u8; 32]>> = m
            .proof
            .iter()
            .map(|h| <[u8; 32]>::try_from(h.as_slice()).ok())
            .collect();
        let proof = proof.ok_or_else(|| bad("prova de inclusão malformada"))?;
        if !teko_crypto::merkle::verify_inclusion(&leaf, m.index, view.log_size, &proof, &root) {
            return Err(bad("a sua cédula NÃO está neste bloco"));
        }
        // everything checked: only now does the ratchet move (never down)
        if final_epoch > self.p.vote_epoch {
            self.p.vote_epoch = final_epoch;
            self.p.vote_set_hash = final_hash.to_vec();
            self.save()?;
        }
        let tracker = teko_crypto::tracker(&leaf);
        let signers = commit.precommits.len() as u32;
        let institutions = set.validators().len() as u32;
        self.hal.show(&Screen::new(
            "SUA CÉDULA ESTÁ REGISTRADA",
            Vec::from([
                format!("Bloco #{}", view.height),
                format!("Certificado: {signers} de {institutions}"),
                format!("Instituições: época {epoch}"),
                String::from("Rastreador:"),
                tracker.clone(),
            ]),
        ));
        Ok(Response::BallotRecorded(teko_protocol::BallotRecorded {
            height: view.height,
            signers,
            institutions,
            tracker,
            epoch,
        }))
    }

    /// The institutions that certify block height `height`, following the approved changes the host
    /// brought from the set in the trust file (epoch 0) link by link: each must create the next epoch,
    /// replace the set in force and be approved by more than 2/3 of it. The persisted RATCHET refuses
    /// a list that stops before the epoch the device already verified, or whose set at that epoch is
    /// another.
    fn institutions_at(&self, height: u64, rotations: &[Vec<u8>]) -> Result<Institutions, Failure> {
        let bad = |why: &'static str| fail(ErrorCode::BadSignature, why);
        let mut epochs: Vec<EpochView> = Vec::new();
        let genesis = self.trust.vote_validators.clone();
        epochs.push(EpochView {
            epoch: 0,
            from: 1,
            hash: teko_bft::set_hash(&genesis),
            keys: genesis,
        });
        for raw in rotations {
            let change = <teko_bft::ValidatorSetChange as prost::Message>::decode(raw.as_slice())
                .map_err(|_| bad("troca de instituições malformada"))?;
            let last = epochs
                .last()
                .ok_or_else(|| fail(ErrorCode::Internal, "sem época"))?;
            change
                .follow(&self.trust.vote_chain_id, last.epoch, &last.keys)
                .map_err(bad)?;
            if change.effective_height <= last.from {
                return Err(bad("as trocas de instituições não avançam em altura"));
            }
            epochs.push(EpochView {
                epoch: change.epoch,
                from: change.effective_height,
                hash: teko_bft::set_hash(&change.validators),
                keys: change.validators,
            });
        }
        let newest = epochs
            .last()
            .ok_or_else(|| fail(ErrorCode::Internal, "sem época"))?;
        let (final_epoch, final_hash) = (newest.epoch, newest.hash);
        // the ratchet
        if self.p.vote_epoch > 0 {
            if final_epoch < self.p.vote_epoch {
                return Err(bad(
                    "a lista de trocas é mais antiga que a já conferida por este dispositivo",
                ));
            }
            let known = epochs
                .iter()
                .find(|e| e.epoch == self.p.vote_epoch)
                .map(|e| e.hash.to_vec());
            if known.as_deref() != Some(self.p.vote_set_hash.as_slice()) {
                return Err(bad(
                    "outro ramo de trocas de instituições: este dispositivo já conferiu uma história diferente",
                ));
            }
        }
        let at = epochs
            .iter()
            .rev()
            .find(|e| e.from <= height)
            .ok_or_else(|| fail(ErrorCode::Internal, "sem época"))?;
        Ok(Institutions {
            keys: at.keys.clone(),
            epoch: at.epoch,
            final_epoch,
            final_hash,
        })
    }

    // ───────────────────────── wipe ─────────────────────────

    fn wipe(&mut self) -> Result<Response, Failure> {
        if self.p.state() == DeviceState::Virgin {
            return Err(fail(ErrorCode::InvalidState, "nada a apagar"));
        }
        self.confirm(
            ButtonKind::Wipe,
            Screen::new(
                "APAGAR TUDO?",
                Vec::from([
                    String::from("Chaves e credenciais serão"),
                    String::from("destruídas. IRREVERSÍVEL."),
                    String::from("Reemissão presencial depois."),
                ]),
            ),
        )?;
        self.wipe_internal()?;
        Ok(Response::WipeDone(WipeDone {}))
    }

    // ───────────────────────── issuance ─────────────────────────

    fn get_identity_request(&mut self, nonce: &[u8], authority: &str) -> Result<Response, Failure> {
        self.require_unlocked_initialized()?;
        self.require_wallet()?;
        if self.p.state() != DeviceState::Initialized {
            return Err(fail(
                ErrorCode::InvalidState,
                "já existe credencial; apague o dispositivo para reemitir",
            ));
        }
        if !(16..=64).contains(&nonce.len()) {
            return Err(fail(
                ErrorCode::InvalidArgument,
                "nonce deve ter 16 a 64 bytes",
            ));
        }
        let authority = sanitize(authority, 60)
            .filter(|a| !a.trim().is_empty())
            .ok_or_else(|| fail(ErrorCode::InvalidArgument, "nome da autoridade inválido"))?;
        let key = self.identity_key()?;
        let pk = public_key_bytes(&key);
        let id = civic_id(&pk).map_err(|_| fail(ErrorCode::Internal, "id cívico"))?;
        let mut lines = Vec::from([String::from("Autoridade:")]);
        lines.extend(wrap(&authority, 28));
        lines.push(String::from("ID Cívico:"));
        lines.extend(wrap(&id, 20));
        self.confirm(
            ButtonKind::IssueIdentity,
            Screen::new("EMITIR CREDENCIAL?", lines),
        )?;

        let msg = issue_request_message(nonce, &pk);
        let pop = sign_tagged(&key, tags::ISSUE_POP, &msg);
        let batch = self.batch_key()?;
        let attest = sign_tagged(&batch, tags::ISSUE_ATTEST, &msg);
        Ok(Response::IdentityRequest(IdentityRequest {
            pk_identity: pk,
            pop_signature: pop,
            attestation_chain: Vec::from([self.p.batch_cert.clone()]),
            attestation_signature: attest,
        }))
    }

    /// Installs the civic credential: a `CivicBundle` the organ's members signed (ADR-0038). The
    /// device checks it ALONE (commitment, id, M distinct trusted members, this identity key); veto,
    /// expiry and revocation are checked by whoever uses the credential, reading the chain.
    fn load_credential(&mut self, bytes: &[u8]) -> Result<Response, Failure> {
        self.require_unlocked_initialized()?;
        if self.p.state() != DeviceState::Initialized {
            return Err(fail(ErrorCode::InvalidState, "credencial já instalada"));
        }
        let bundle = CivicBundle::decode(bytes)
            .map_err(|_| fail(ErrorCode::InvalidArgument, "credencial malformada"))?;
        let policy = CivicPolicy {
            chain_id: self.trust.civic_chain_id.clone(),
            threshold: self.trust.civic_threshold,
            members: self.trust.civic_members.clone(),
        };
        let pk = public_key_bytes(&self.identity_key()?);
        verify_civic_bundle(&bundle, &policy, &pk).map_err(|e| {
            let code = match e {
                teko_crypto::civic::CivicError::Policy => ErrorCode::InvalidState,
                teko_crypto::civic::CivicError::Subject
                | teko_crypto::civic::CivicError::Payload => ErrorCode::InvalidArgument,
                _ => ErrorCode::BadSignature,
            };
            fail(code, &format!("credencial recusada: {e}"))
        })?;
        let claims = claim_list(
            bundle
                .salt
                .as_slice()
                .try_into()
                .map_err(|_| fail(ErrorCode::InvalidArgument, "sal inválido"))?,
            &bundle.claims,
        );
        let mut lines = Vec::from([String::from("Emitida pelo órgão:")]);
        lines.extend(wrap(&bundle.chain_id, 28));
        lines.push(format!(
            "{} de {} membros assinaram",
            policy.threshold,
            policy.members.len()
        ));
        for c in &claims {
            lines.extend(wrap(&format!("{}: {}", c.name, c.value), 28));
        }
        self.confirm(
            ButtonKind::LoadCredential,
            Screen::new("INSTALAR CREDENCIAL?", lines),
        )?;
        self.p.credential = bundle.encode_to_vec();
        self.p.state = DeviceState::CredentialsIssued as i32;
        self.save()?;
        Ok(Response::CredentialLoaded(CredentialLoaded {}))
    }
}

fn jurisdiction_label(scope: &teko_election::Scope) -> String {
    match scope.level() {
        ScopeLevel::National => String::from("Jurisdição: Nacional"),
        ScopeLevel::State => format!("Jurisdição: Estado {}", scope.code),
        ScopeLevel::Municipality => format!("Jurisdição: Município {}", scope.code),
        ScopeLevel::Unspecified => String::from("Jurisdição: ?"),
    }
}

fn election_failure(e: ElectionError) -> Failure {
    let code = match e {
        ElectionError::InsufficientSignatures { .. } => ErrorCode::InsufficientSignatures,
        _ => ErrorCode::InvalidArgument,
    };
    fail(code, &format!("eleição recusada: {e}"))
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    use subtle::ConstantTimeEq;
    a.ct_eq(b).into()
}

/// Maps a predicate to the single claim to disclose and a human description.
fn resolve_predicate(p: &Predicate, claims: &[Claim]) -> Result<(String, String), Failure> {
    let unavailable = || {
        fail(
            ErrorCode::InvalidArgument,
            "atributo indisponível na credencial",
        )
    };
    match PredicateKind::try_from(p.kind).unwrap_or(PredicateKind::Unspecified) {
        PredicateKind::AgeAtLeast => {
            // Smallest issuer-derived `age_over_K` with K >= requested and value "true".
            let best = claims
                .iter()
                .filter(|c| c.value == "true")
                .filter_map(|c| {
                    let k: u32 = c.name.strip_prefix("age_over_")?.parse().ok()?;
                    (k >= p.arg).then_some((k, c.name.clone()))
                })
                .min_by_key(|(k, _)| *k);
            let (k, name) = best.ok_or_else(unavailable)?;
            Ok((name, format!("Idade mínima de {k} anos: SIM")))
        }
        PredicateKind::Has => {
            let c = claims
                .iter()
                .find(|c| c.name == p.name && c.value == "true")
                .ok_or_else(unavailable)?;
            Ok((c.name.clone(), format!("Possui: {}", c.name)))
        }
        PredicateKind::Reveal => {
            let c = claims
                .iter()
                .find(|c| c.name == p.name)
                .ok_or_else(unavailable)?;
            Ok((c.name.clone(), format!("{} = {}", c.name, c.value)))
        }
        PredicateKind::Unspecified => Err(fail(ErrorCode::InvalidArgument, "predicado inválido")),
    }
}

/// One screen line `"Label: 1.234,5 TEKO"` (exact, never rounded: ADR-0029);
/// label and value go on separate lines when they do not fit in 28 columns.
/// The fields of one asset-trade request, shared by `SignAssetOffer` and `SignAssetAccept`.
struct AssetTrade<'a> {
    accept: bool,
    chain_id: &'a str,
    class: &'a str,
    asset: &'a str,
    /// The OTHER party: the buyer (offer) or the seller (accept).
    other_address: &'a str,
    price: u64,
    version: u64,
    metadata: &'a [u8],
    window_secs: u64,
    fee: u64,
    nonce: u64,
}

/// What is authenticated (not hidden) with the sealed attributes of a credential (ADR-0039).
fn credential_seal_context(ctype: &str, subject: &[u8]) -> Vec<u8> {
    let mut c = Vec::from(ctype.as_bytes());
    c.push(0);
    c.extend_from_slice(subject);
    c
}

/// One epoch of the vote chain's institutions, as the device follows them (ADR-0045).
struct EpochView {
    epoch: u64,
    /// The first block height that belongs to the epoch.
    from: u64,
    keys: Vec<Vec<u8>>,
    hash: [u8; 32],
}

/// The institutions that certify one height, and where the verified chain of changes ends.
struct Institutions {
    keys: Vec<Vec<u8>>,
    epoch: u64,
    final_epoch: u64,
    final_hash: [u8; 32],
}

/// The few fields of a vote-chain block the device reads. The block id (a hash of the WHOLE payload)
/// is what the institutions signed, so reading a subset of the fields is safe.
#[derive(Clone, PartialEq, prost::Message)]
struct BlockView {
    #[prost(string, tag = "1")]
    chain_id: String,
    #[prost(uint64, tag = "2")]
    height: u64,
    #[prost(uint64, tag = "7")]
    log_size: u64,
    #[prost(bytes = "vec", tag = "8")]
    log_root: Vec<u8>,
}

/// The board entry kind of a ballot (`EntryKind::BALLOT`), the first byte of its Merkle leaf.
const BOARD_BALLOT_KIND: u8 = 4;

/// A credential type name: short, no spoofing characters.
fn clean_ctype(s: &str) -> Result<String, Failure> {
    sanitize(s, 32)
        .filter(|c| !c.trim().is_empty() && c.as_str() == s)
        .ok_or_else(|| fail(ErrorCode::InvalidArgument, "tipo de credencial inválido"))
}

/// `AAAA-MM-DD` (UTC) of a Unix time: civil-from-days, no clock or table needed.
fn ymd(secs: u64) -> String {
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(mo <= 2);
    format!("{y:04}-{mo:02}-{d:02}")
}

/// "20 min", "3 h", "2 dias": the veto window as a person reads it.
fn window_text(secs: u64) -> String {
    match secs {
        s if s < 120 => format!("{s} s"),
        s if s < 7_200 => format!("{} min", s / 60),
        s if s < 172_800 => format!("{} h", s / 3_600),
        s => format!("{} dias", s / 86_400),
    }
}

fn push_amount(lines: &mut Vec<String>, label: &str, ypy: u64) {
    let value = teko_crypto::amount::format_amount(ypy);
    let one = format!("{label} {value}");
    if one.chars().count() <= 28 {
        lines.push(one);
    } else {
        lines.push(String::from(label));
        lines.push(format!("  {value}"));
    }
}
