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
use teko_crypto::credential::{self, IssuedCredential};
use teko_crypto::{
    SigningKey, canonical_service_id, civic_id, derive_key, derive_signing_key, pairwise_id,
    public_key_bytes, sign_tagged, tags, voter,
};
use teko_election::{
    BallotBody, Domicile, ElectionBundle, ElectionError, ElectionManifest, ElectionStatus,
    GovernancePolicy, Phase, RegistrationBlinder, ScopeLevel, Transition, contest_hash,
    is_eligible, manifest_hash, registration_message, registration_request_message, replay,
    sign_ballot, validate_revision,
};
use teko_protocol::{
    AttestationResponse, AttributeProof, BallotSignature, ButtonKind, ButtonRequest, CivicId,
    ContestInfo, CredentialLoaded, DeviceState, DocumentSignature, ElectionLoaded, ErrorCode,
    Failure, Features, FirmwareStatus, IdentityRequest, PROTO_VERSION, PairwiseId, PinSet,
    PinWrong, Predicate, PredicateKind, RegistrationRequest, RegistrationToken, Request, Response,
    Success, Unlocked, VoterKey, WipeDone,
};
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
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
            label: self.cfg.label.clone(),
            has_credential: !self.p.credential.is_empty(),
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
            Request::SignBallot(m) => {
                self.sign_ballot(&m.election_id, &m.contest_id, m.option_index)
            }
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
        let issued = IssuedCredential::decode(self.p.credential.as_slice())
            .map_err(|_| fail(ErrorCode::Internal, "credencial ilegível"))?;

        let (claim_name, description) = resolve_predicate(&predicate, &issued)?;
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
        let presentation =
            credential::present(&key, &issued, &[claim_name.as_str()], nonce, &verifier)
                .map_err(|_| fail(ErrorCode::Internal, "falha ao montar prova"))?;
        Ok(Response::AttributeProof(AttributeProof {
            presentation: presentation.encode_to_vec(),
        }))
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
        let issued = IssuedCredential::decode(self.p.credential.as_slice()).ok()?;
        let claim = |n: &str| {
            issued
                .claims
                .iter()
                .find(|c| c.name == n)
                .map(|c| c.value.clone())
        };
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
        let option = contest
            .options
            .iter()
            .find(|o| o.index == option_index)
            .ok_or_else(|| fail(ErrorCode::InvalidArgument, "opção inexistente"))?;

        // EVERYTHING shown comes from the verified manifest, never from the host.
        let mut lines = Vec::from([String::from("Eleição:")]);
        lines.extend(wrap(&manifest.title, 28));
        lines.push(String::from("Pleito:"));
        lines.extend(wrap(&contest.title, 28));
        lines.push(jurisdiction_label(&scope));
        lines.push(String::from("SEU VOTO:"));
        let label = if option.number.is_empty() {
            option.name.clone()
        } else {
            format!("{} - {}", option.number, option.name)
        };
        lines.extend(wrap(&label, 28));
        lines.push(String::from("(protótipo: não é enviado"));
        lines.push(String::from("ao quadro; sem prova ZK)"));
        self.confirm(
            ButtonKind::SignBallot,
            Screen::new("CONFIRMAR VOTO?", lines),
        )?;

        let root = self.voter_root()?;
        let key = voter::derive_voter_key(&root, election_id, contest_id)
            .map_err(|_| fail(ErrorCode::Internal, "chave de eleitor"))?;
        let signature = sign_ballot(
            &key,
            &BallotBody {
                election_id: String::from(election_id),
                contest_id: String::from(contest_id),
                contest_hash: chash.to_vec(),
                option_index,
            },
        );
        // Record BEFORE releasing the signature: a crash can lose the delivery,
        // but can never leave a signature out that the device forgot it made.
        self.p.ballots_signed.push(chash.to_vec());
        self.save()?;
        Ok(Response::BallotSignature(BallotSignature {
            signature,
            voter_key: public_key_bytes(&key),
        }))
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

    fn load_credential(&mut self, bytes: &[u8]) -> Result<Response, Failure> {
        self.require_unlocked_initialized()?;
        if self.p.state() != DeviceState::Initialized {
            return Err(fail(ErrorCode::InvalidState, "credencial já instalada"));
        }
        let issued = IssuedCredential::decode(bytes)
            .map_err(|_| fail(ErrorCode::InvalidArgument, "credencial malformada"))?;
        credential::verify_issued(&self.trust.authority_root, &issued).map_err(|_| {
            fail(
                ErrorCode::BadSignature,
                "credencial não assinada pela autoridade",
            )
        })?;
        let cred = issued
            .credential
            .as_ref()
            .ok_or_else(|| fail(ErrorCode::InvalidArgument, "credencial vazia"))?;
        let pk = public_key_bytes(&self.identity_key()?);
        if cred.subject_key != pk {
            return Err(fail(
                ErrorCode::BadSignature,
                "credencial emitida para outra chave",
            ));
        }
        let mut lines = Vec::from([String::from("Emissor:")]);
        lines.extend(wrap(&cred.issuer, 28));
        for c in &issued.claims {
            lines.extend(wrap(&format!("{}: {}", c.name, c.value), 28));
        }
        self.confirm(
            ButtonKind::LoadCredential,
            Screen::new("INSTALAR CREDENCIAL?", lines),
        )?;
        self.p.credential = issued.encode_to_vec();
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
fn resolve_predicate(
    p: &Predicate,
    issued: &IssuedCredential,
) -> Result<(String, String), Failure> {
    let unavailable = || {
        fail(
            ErrorCode::InvalidArgument,
            "atributo indisponível na credencial",
        )
    };
    match PredicateKind::try_from(p.kind).unwrap_or(PredicateKind::Unspecified) {
        PredicateKind::AgeAtLeast => {
            // Smallest issuer-derived `age_over_K` with K >= requested and value "true".
            let best = issued
                .claims
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
            let c = issued
                .claims
                .iter()
                .find(|c| c.name == p.name && c.value == "true")
                .ok_or_else(unavailable)?;
            Ok((c.name.clone(), format!("Possui: {}", c.name)))
        }
        PredicateKind::Reveal => {
            let c = issued
                .claims
                .iter()
                .find(|c| c.name == p.name)
                .ok_or_else(unavailable)?;
            Ok((c.name.clone(), format!("{} = {}", c.name, c.value)))
        }
        PredicateKind::Unspecified => Err(fail(ErrorCode::InvalidArgument, "predicado inválido")),
    }
}
