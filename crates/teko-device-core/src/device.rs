use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

use prost::Message;
use rand_core::{CryptoRng, RngCore};
use teko_attestation::{
    BlobHasher, FactoryData, FirmwareError, FirmwareManifest, TrustPolicy, issue_request_message,
    manifest_fingerprint, sign_attestation, verify_blob, verify_manifest,
};
use teko_crypto::credential::{self, IssuedCredential};
use teko_crypto::{
    SigningKey, canonical_service_id, civic_id, derive_key, derive_signing_key, pairwise_id,
    public_key_bytes, sign_tagged, tags, voter,
};
use teko_protocol::{
    AttestationResponse, AttributeProof, ButtonKind, ButtonRequest, CivicId, CredentialLoaded,
    DeviceState, DocumentSignature, ErrorCode, Failure, Features, FirmwareStatus, IdentityRequest,
    PROTO_VERSION, PairwiseId, PinSet, PinWrong, Predicate, PredicateKind, Request, Response,
    Success, Unlocked, VoterCommitment, WipeDone,
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

pub struct Device<H: Hal, R: RngCore + CryptoRng> {
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
}

impl<H: Hal, R: RngCore + CryptoRng> Drop for Device<H, R> {
    fn drop(&mut self) {
        self.p.k_root.zeroize();
        self.p.voter_secret.zeroize();
        self.p.pin_hash.zeroize();
        self.p.batch_key.zeroize();
    }
}

impl<H: Hal, R: RngCore + CryptoRng> Device<H, R> {
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
            Request::GetVoterCommitment(_) => self.get_voter_commitment(),
            Request::SignBallot(m) => self.sign_ballot(&m.election_id, &m.ballot_hash, &m.summary),
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
        self.hal.show(&Screen::new(
            "DIGITE O PIN",
            Vec::from([format!(
                "Tentativas restantes: {}",
                self.cfg
                    .max_pin_attempts
                    .saturating_sub(self.p.pin_fail_count)
            )]),
        ));
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

    // ───────────────────────── voting (stubs) ─────────────────────────

    fn get_voter_commitment(&mut self) -> Result<Response, Failure> {
        self.require_ready()?;
        self.confirm(
            ButtonKind::ExportCommitment,
            Screen::new(
                "EXPORTAR COMMITMENT?",
                Vec::from([
                    String::from("Compromisso de eleitor."),
                    String::from("Não identifica você."),
                    String::from("(STUB: SHA3, não Poseidon)"),
                ]),
            ),
        )?;
        let s = self.voter_secret()?;
        Ok(Response::VoterCommitment(VoterCommitment {
            commitment: voter::commitment(&s).to_vec(),
        }))
    }

    fn sign_ballot(
        &mut self,
        election: &str,
        ballot_hash: &[u8],
        summary: &str,
    ) -> Result<Response, Failure> {
        self.require_ready()?;
        if ballot_hash.len() != 32 || election.trim().is_empty() {
            return Err(fail(ErrorCode::InvalidArgument, "voto inválido"));
        }
        let election = sanitize(election, 60)
            .ok_or_else(|| fail(ErrorCode::InvalidArgument, "eleição inválida"))?;
        let summary = sanitize(summary, 80)
            .ok_or_else(|| fail(ErrorCode::InvalidArgument, "resumo inválido"))?;
        let mut lines = Vec::from([String::from("STUB: nada será assinado")]);
        lines.extend(wrap(&election, 28));
        lines.extend(wrap(&summary, 28));
        lines.extend(hex_lines(ballot_hash, 8));
        self.confirm(ButtonKind::SignBallot, Screen::new("VOTAR (STUB)", lines))?;
        // Voting arrives with the blockchain phase; never emit a ballot signature.
        Err(fail(
            ErrorCode::NotImplemented,
            "votação ainda não implementada (stub)",
        ))
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
