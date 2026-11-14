//! Simulated civic authority ("issuer"). TEST KEYS ONLY.
//!
//! - generates the test root, firmware-signer and batch keys;
//! - signs firmware manifests (M-of-N);
//! - provisions "factory" attestation material for emulated devices;
//! - verifies a device's identity request (proof of possession + batch
//!   attestation) and issues a citizen credential bound to its identity key.
//!
//! Voter commitments are NOT handled here: they travel through a separate
//! channel with no link to the identity (ADR-0008, not implemented yet).

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use prost::Message;
use rand::Rng;
use rand::rand_core::UnwrapErr;
use rand::rngs::SysRng;
use teko_attestation::{
    FactoryData, FirmwareManifest, TrustPolicy, blob_hash, issue_batch_cert, issue_request_message,
    sign_manifest, verify_batch_chain,
};
use teko_crypto::credential::{self, IssuedCredential};
use teko_crypto::{
    SigningKey, civic_id, generate_signing_key, hex, public_key_bytes, tagged_hash, tags,
    verify_tagged,
};
use teko_election::{
    BALLOT_KIND_PUBLIC, Contest, Domicile, ElectionBundle, ElectionManifest, ElectionOption,
    Office, Phase, Schedule, Scope, Transition, contest_hash,
    sign_manifest as sign_election_manifest, sign_transition,
};
use teko_protocol::IdentityRequest;
use thiserror::Error;

pub const FW_SIGNERS: usize = 3;
pub const FW_THRESHOLD: usize = 2;
pub const GOV_SIGNERS: usize = 3;
pub const GOV_THRESHOLD: usize = 2;
pub const ISSUER_NAME: &str = "Autoridade Cívica (simulada)";
const CREDENTIAL_YEARS: u64 = 5;
const AGE_THRESHOLDS: [u32; 4] = [16, 18, 21, 65];

#[derive(Debug, Error)]
pub enum IssuerError {
    #[error("i/o: {0}")]
    Io(#[from] io::Error),
    #[error("diretório da autoridade não inicializado (rode `issuer-cli init`)")]
    NotInitialized,
    #[error("já inicializado em {0} (use --force para recriar)")]
    AlreadyInitialized(String),
    #[error("dados do cidadão inválidos: {0}")]
    InvalidSubject(String),
    #[error("pedido de emissão inválido: {0}")]
    InvalidRequest(String),
    #[error("arquivo corrompido: {0}")]
    Corrupt(String),
}

/// Person data verified OUT OF BAND by the authority.
#[derive(Debug, Clone)]
pub struct Subject {
    pub name: String,
    /// `YYYY-MM-DD`.
    pub birth_date: String,
    /// ISO 3166-1 alpha-2, upper case.
    pub nationality: String,
    /// Electoral domicile (from the electoral registry, simulated here): UF, e.g. `SP`.
    pub electoral_state: String,
    /// 7-digit IBGE municipality code, e.g. `3550308`.
    pub electoral_municipality: String,
}

fn write_private(path: &Path, content: &str) -> io::Result<()> {
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path)?;
    f.write_all(content.as_bytes())
}

fn save_key(path: &Path, key: &SigningKey) -> io::Result<()> {
    write_private(path, &hex::encode(&key.to_bytes()))
}

fn load_key(path: &Path) -> Result<SigningKey, IssuerError> {
    let text = fs::read_to_string(path).map_err(|_| IssuerError::NotInitialized)?;
    let bytes = hex::decode(&text).map_err(|_| IssuerError::Corrupt(path.display().to_string()))?;
    SigningKey::from_slice(&bytes).map_err(|_| IssuerError::Corrupt(path.display().to_string()))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub struct Issuer {
    dir: PathBuf,
}

impl Issuer {
    pub fn open(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn is_initialized(&self) -> bool {
        self.dir.join("root.key").exists() && self.dir.join("trust.txt").exists()
    }

    /// Creates test roots: authority root, 3 firmware signers, 1 batch key.
    pub fn init(&self, force: bool) -> Result<TrustPolicy, IssuerError> {
        if self.is_initialized() && !force {
            return Err(IssuerError::AlreadyInitialized(
                self.dir.display().to_string(),
            ));
        }
        fs::create_dir_all(&self.dir)?;
        let mut rng = UnwrapErr(SysRng);
        let root = generate_signing_key(&mut rng);
        save_key(&self.dir.join("root.key"), &root)?;
        let mut signers = Vec::new();
        for i in 0..FW_SIGNERS {
            let k = generate_signing_key(&mut rng);
            save_key(&self.dir.join(format!("fw-{i}.key")), &k)?;
            signers.push(public_key_bytes(&k));
        }
        let batch = generate_signing_key(&mut rng);
        save_key(&self.dir.join("batch.key"), &batch)?;
        let cert = issue_batch_cert(&root, "lote-dev-001", &public_key_bytes(&batch));
        fs::write(
            self.dir.join("batch.cert"),
            hex::encode(&cert.encode_to_vec()),
        )?;

        let mut gov = Vec::new();
        for i in 0..GOV_SIGNERS {
            let k = generate_signing_key(&mut rng);
            save_key(&self.dir.join(format!("gov-{i}.key")), &k)?;
            gov.push(public_key_bytes(&k));
        }
        let trust = TrustPolicy {
            authority_root: public_key_bytes(&root),
            fw_signers: signers,
            fw_threshold: FW_THRESHOLD,
            revoked_batches: Vec::new(),
            gov_signers: gov,
            gov_threshold: GOV_THRESHOLD,
        };
        fs::write(self.dir.join("trust.txt"), trust.to_text())?;
        Ok(trust)
    }

    pub fn trust(&self) -> Result<TrustPolicy, IssuerError> {
        let text = fs::read_to_string(self.dir.join("trust.txt"))
            .map_err(|_| IssuerError::NotInitialized)?;
        TrustPolicy::parse(&text).map_err(|e| IssuerError::Corrupt(e.to_string()))
    }

    /// Factory material for the emulated device (batch key + certificate).
    pub fn provision_device(&self) -> Result<FactoryData, IssuerError> {
        let batch = load_key(&self.dir.join("batch.key"))?;
        let cert_hex = fs::read_to_string(self.dir.join("batch.cert"))
            .map_err(|_| IssuerError::NotInitialized)?;
        let cert = hex::decode(&cert_hex).map_err(|_| IssuerError::Corrupt("batch.cert".into()))?;
        Ok(FactoryData {
            batch_key: batch.to_bytes().to_vec(),
            batch_cert: cert,
        })
    }

    /// Signs a manifest with the chosen firmware signers (indexes into the N).
    pub fn sign_firmware(
        &self,
        version: u32,
        blob: &[u8],
        signers: &[usize],
    ) -> Result<FirmwareManifest, IssuerError> {
        let mut m = FirmwareManifest {
            version,
            size: blob.len() as u64,
            hash: blob_hash(blob).to_vec(),
            signatures: Vec::new(),
        };
        for &i in signers {
            let k = load_key(&self.dir.join(format!("fw-{i}.key")))?;
            m.signatures.push(sign_manifest(&k, &m));
        }
        Ok(m)
    }

    /// Writes `firmware-v1/v2` (blob + manifest) and `factory.bin`.
    /// v1 is signed by signers {0,1}; v2 by {1,2}. Idempotent.
    pub fn dev_setup(&self) -> Result<TrustPolicy, IssuerError> {
        let mut trust = if self.is_initialized() {
            self.trust()?
        } else {
            self.init(false)?
        };
        if trust.gov_signers.is_empty() {
            // Older `dev/` folders predate elections: add governance keys in place.
            let mut rng = UnwrapErr(SysRng);
            for i in 0..GOV_SIGNERS {
                let k = generate_signing_key(&mut rng);
                save_key(&self.dir.join(format!("gov-{i}.key")), &k)?;
                trust.gov_signers.push(public_key_bytes(&k));
            }
            trust.gov_threshold = GOV_THRESHOLD;
            fs::write(self.dir.join("trust.txt"), trust.to_text())?;
        }
        self.dev_election(now_secs())?;
        for (version, signers) in [(1u32, [0usize, 1]), (2, [1, 2])] {
            let blob = make_firmware_blob(version, 48 * 1024);
            let manifest = self.sign_firmware(version, &blob, &signers)?;
            fs::write(self.dir.join(format!("firmware-v{version}.blob")), &blob)?;
            fs::write(
                self.dir.join(format!("firmware-v{version}.manifest")),
                manifest.encode_to_vec(),
            )?;
        }
        fs::write(
            self.dir.join("factory.bin"),
            self.provision_device()?.encode_to_vec(),
        )?;
        Ok(trust)
    }

    /// Writes `election.bundle`: a demo election with FOUR contests on DIFFERENT
    /// dates, so every phase is visible: President (voting open), Governor of SP
    /// (registration only, voting opens tomorrow), Governor of RJ (voting open but
    /// a different state), Mayor of São Paulo (candidates announced only).
    /// Dates are relative to `now`. Re-running refreshes them.
    pub fn dev_election(&self, now: u64) -> Result<(), IssuerError> {
        const DAY: u64 = 86_400;
        let gov: Vec<SigningKey> = (0..GOV_SIGNERS)
            .map(|i| load_key(&self.dir.join(format!("gov-{i}.key"))))
            .collect::<Result<_, _>>()?;
        let opts = |names: &[&str], first: u32| -> Vec<ElectionOption> {
            names
                .iter()
                .enumerate()
                .map(|(i, n)| ElectionOption {
                    index: i as u32,
                    name: (*n).to_string(),
                    number: format!("{}", first + i as u32),
                })
                .collect()
        };
        let contest = |id: &str,
                       office: Office,
                       title: &str,
                       scope: Scope,
                       o: Vec<ElectionOption>,
                       s: (u64, u64, u64)| Contest {
            contest_id: id.to_string(),
            office: office as i32,
            title: title.to_string(),
            scope: Some(scope),
            options: o,
            schedule: Some(Schedule {
                registration_opens: s.0,
                voting_opens: s.1,
                voting_closes: s.2,
            }),
        };
        let mut manifest = ElectionManifest {
            format_version: 2,
            election_id: "eleicoes-exemplo-2026".to_string(),
            title: "Eleições de exemplo 2026 (protótipo)".to_string(),
            contests: vec![
                contest(
                    "presidente",
                    Office::President,
                    "Presidente da República",
                    Scope::national(),
                    opts(
                        &[
                            "Candidata Onça-Pintada",
                            "Candidato Boto-Cor-de-Rosa",
                            "Candidata Arara-Azul",
                        ],
                        10,
                    ),
                    (now - 5 * DAY, now - DAY, now + 3 * DAY),
                ),
                contest(
                    "governador-sp",
                    Office::Governor,
                    "Governador de São Paulo",
                    Scope::state("SP"),
                    opts(&["Candidato Guará", "Candidata Vitória-Régia"], 20),
                    (now - 3 * DAY, now + DAY, now + 5 * DAY),
                ),
                contest(
                    "governador-rj",
                    Office::Governor,
                    "Governador do Rio de Janeiro",
                    Scope::state("RJ"),
                    opts(&["Candidato Mico-Leão-Dourado", "Candidata Capivara"], 40),
                    (now - 4 * DAY, now - DAY, now + 2 * DAY),
                ),
                contest(
                    "prefeito-3550308",
                    Office::Mayor,
                    "Prefeito de São Paulo",
                    Scope::municipality("3550308"),
                    opts(&["Candidata Jaguatirica", "Candidato Tucano"], 30),
                    (now + DAY, now + 3 * DAY, now + 4 * DAY),
                ),
            ],
            ballot_kind: BALLOT_KIND_PUBLIC.to_string(),
            registry_key: Vec::new(),
            revision: 1,
            signatures: Vec::new(),
        };
        manifest.signatures = [0usize, 1]
            .iter()
            .map(|i| sign_election_manifest(&gov[*i], &manifest))
            .collect();

        // Governance decisions, each contest on ITS OWN schedule.
        let targets = [
            ("presidente", Phase::VotingOpen),
            ("governador-sp", Phase::RegistrationOpen),
            ("governador-rj", Phase::VotingOpen),
            ("prefeito-3550308", Phase::CandidatesPublished),
        ];
        let mut transitions: Vec<Transition> = Vec::new();
        for (id, upto) in targets {
            let Some(c) = manifest.contest(id) else {
                continue;
            };
            let s = c.schedule.clone().unwrap_or_default();
            let steps = [
                (
                    Phase::Draft,
                    Phase::CandidatesPublished,
                    s.registration_opens.saturating_sub(DAY / 2),
                ),
                (
                    Phase::CandidatesPublished,
                    Phase::RegistrationOpen,
                    s.registration_opens,
                ),
                (Phase::RegistrationOpen, Phase::VotingOpen, s.voting_opens),
            ];
            for (from, to, at) in steps {
                if (from as i32) >= (upto as i32) {
                    break;
                }
                let mut t = Transition {
                    election_id: manifest.election_id.clone(),
                    contest_id: id.to_string(),
                    contest_hash: contest_hash(c).to_vec(),
                    from_phase: from as i32,
                    to_phase: to as i32,
                    seq: transitions.len() as u64 + 1,
                    at,
                    reason: String::new(),
                    signatures: Vec::new(),
                };
                t.signatures = [1usize, 2]
                    .iter()
                    .map(|k| sign_transition(&gov[*k], &t))
                    .collect();
                transitions.push(t);
            }
        }
        let bundle = ElectionBundle {
            manifest: manifest.encode_to_vec(),
            transitions: transitions.iter().map(|t| t.encode_to_vec()).collect(),
        };
        fs::write(self.dir.join("election.bundle"), bundle.encode_to_vec())?;
        Ok(())
    }

    /// Fresh random nonce the device must sign into its identity request.
    pub fn new_nonce() -> Vec<u8> {
        let mut n = vec![0u8; 32];
        UnwrapErr(SysRng).fill_bytes(&mut n);
        n
    }

    pub fn issue_credential(
        &self,
        nonce: &[u8],
        request: &IdentityRequest,
        subject: &Subject,
    ) -> Result<IssuedCredential, IssuerError> {
        self.issue_credential_at(nonce, request, subject, now_secs())
    }

    /// Verifies the device (genuine + owns the key) then issues the credential
    /// bound to the device's identity key.
    pub fn issue_credential_at(
        &self,
        nonce: &[u8],
        request: &IdentityRequest,
        subject: &Subject,
        now: u64,
    ) -> Result<IssuedCredential, IssuerError> {
        let trust = self.trust()?;
        let bad = |m: &str| IssuerError::InvalidRequest(m.to_string());
        let msg = issue_request_message(nonce, &request.pk_identity);
        verify_tagged(
            &request.pk_identity,
            tags::ISSUE_POP,
            &msg,
            &request.pop_signature,
        )
        .map_err(|_| bad("prova de posse da chave inválida"))?;
        let batch = verify_batch_chain(&trust, &request.attestation_chain)
            .map_err(|e| IssuerError::InvalidRequest(format!("atestação: {e}")))?;
        verify_tagged(
            &batch.public_key,
            tags::ISSUE_ATTEST,
            &msg,
            &request.attestation_signature,
        )
        .map_err(|_| bad("assinatura de atestação inválida"))?;

        let attributes = build_attributes(subject, now)?;
        let root = load_key(&self.dir.join("root.key"))?;
        let mut credential_id = [0u8; 16];
        UnwrapErr(SysRng).fill_bytes(&mut credential_id);
        let issued = credential::issue(
            &root,
            ISSUER_NAME,
            &request.pk_identity,
            now,
            now + CREDENTIAL_YEARS * 365 * 24 * 3600,
            credential_id,
            &attributes,
            &mut UnwrapErr(SysRng),
        );
        // Authority-side registry of issuance (identity side only; never touches voting).
        let line = format!(
            "{now} credential={} civic_id={}\n",
            hex::encode(&credential_id),
            civic_id(&request.pk_identity).unwrap_or_default()
        );
        let mut log = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join("issued.log"))?;
        log.write_all(line.as_bytes())?;
        Ok(issued)
    }
}

/// Deterministic fake firmware image (simulation only).
pub fn make_firmware_blob(version: u32, size: usize) -> Vec<u8> {
    let mut out = format!("TEKO-FIRMWARE-SIM v{version}\n").into_bytes();
    let mut counter = 0u32;
    while out.len() < size {
        out.extend_from_slice(&tagged_hash(
            "teko/sim/firmware",
            &[&version.to_be_bytes(), &counter.to_be_bytes()],
        ));
        counter += 1;
    }
    out.truncate(size);
    out
}

// ───────────────────────── subject / dates ─────────────────────────

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if is_leap(y) => 29,
        _ => 28,
    }
}

fn parse_date(s: &str) -> Option<(i64, u32, u32)> {
    let mut it = s.split('-');
    let (y, m, d) = (it.next()?, it.next()?, it.next()?);
    if it.next().is_some() || y.len() != 4 || m.len() != 2 || d.len() != 2 {
        return None;
    }
    let (y, m, d): (i64, u32, u32) = (y.parse().ok()?, m.parse().ok()?, d.parse().ok()?);
    ((1..=12).contains(&m) && d >= 1 && d <= days_in_month(y, m)).then_some((y, m, d))
}

/// Completed years between `birth` and the date of `now` (unix seconds).
pub fn age_at(birth: (i64, u32, u32), now: u64) -> i64 {
    let (ty, tm, td) = civil_from_days((now / 86_400) as i64);
    let mut age = ty - birth.0;
    if (tm, td) < (birth.1, birth.2) {
        age -= 1;
    }
    age
}

fn build_attributes(subject: &Subject, now: u64) -> Result<Vec<(String, String)>, IssuerError> {
    let invalid = |m: &str| IssuerError::InvalidSubject(m.to_string());
    let name = subject.name.trim();
    if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
        return Err(invalid("nome deve ter de 1 a 80 caracteres"));
    }
    let birth = parse_date(subject.birth_date.trim())
        .ok_or_else(|| invalid("data de nascimento deve ser AAAA-MM-DD válida"))?;
    let age = age_at(birth, now);
    if !(0..=130).contains(&age) {
        return Err(invalid("data de nascimento fora do intervalo plausível"));
    }
    let nat = subject.nationality.trim().to_ascii_uppercase();
    if nat.len() != 2 || !nat.chars().all(|c| c.is_ascii_alphabetic()) {
        return Err(invalid("nacionalidade deve ter 2 letras (ex.: BR)"));
    }
    let domicile = Domicile::new(
        subject.electoral_state.trim(),
        subject.electoral_municipality.trim(),
    )
    .map_err(|_| {
        invalid("domicílio eleitoral: UF com 2 letras maiúsculas e município com 7 dígitos (IBGE)")
    })?;
    let mut reg = [0u8; 4];
    UnwrapErr(SysRng).fill_bytes(&mut reg);
    let mut attrs = vec![
        ("name".to_string(), name.to_string()),
        (
            "birth_date".to_string(),
            subject.birth_date.trim().to_string(),
        ),
        ("nationality".to_string(), nat),
        // Internal registry id: deliberately NOT an RG/CPF.
        (
            "registry_id".to_string(),
            format!("TK-{}", hex::encode(&reg).to_uppercase()),
        ),
    ];
    attrs.push(("electoral_state".to_string(), domicile.state));
    attrs.push(("electoral_municipality".to_string(), domicile.municipality));
    for t in AGE_THRESHOLDS {
        attrs.push((format!("age_over_{t}"), (age >= i64::from(t)).to_string()));
    }
    Ok(attrs)
}

/// Unique scratch dir for tests/e2e (no external crate needed).
#[doc(hidden)]
pub fn scratch_dir(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("teko-{tag}-{}-{nanos}-{n}", std::process::id()));
    let _ = fs::create_dir_all(&p);
    p
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use teko_attestation::{verify_attestation, verify_manifest};
    use teko_crypto::credential::verify_issued;

    fn fake_request(
        issuer: &Issuer,
        nonce: &[u8],
        good_pop: bool,
    ) -> (IdentityRequest, SigningKey) {
        let id_key = generate_signing_key(&mut UnwrapErr(SysRng));
        let pk = public_key_bytes(&id_key);
        let msg = issue_request_message(nonce, &pk);
        let factory = issuer.provision_device().unwrap();
        let batch = SigningKey::from_slice(&factory.batch_key).unwrap();
        let signer = if good_pop {
            id_key.clone()
        } else {
            generate_signing_key(&mut UnwrapErr(SysRng))
        };
        (
            IdentityRequest {
                pk_identity: pk,
                pop_signature: teko_crypto::sign_tagged(&signer, tags::ISSUE_POP, &msg),
                attestation_chain: vec![factory.batch_cert],
                attestation_signature: teko_crypto::sign_tagged(&batch, tags::ISSUE_ATTEST, &msg),
            },
            id_key,
        )
    }

    fn subject() -> Subject {
        Subject {
            name: "Maria da Silva".into(),
            birth_date: "1990-05-17".into(),
            nationality: "br".into(),
            electoral_state: "SP".into(),
            electoral_municipality: "3550308".into(),
        }
    }

    #[test]
    fn init_is_idempotent_guarded_and_dev_setup_works() {
        let issuer = Issuer::open(scratch_dir("init"));
        let trust = issuer.dev_setup().unwrap();
        assert!(matches!(
            issuer.init(false),
            Err(IssuerError::AlreadyInitialized(_))
        ));
        assert_eq!(
            issuer.dev_setup().unwrap(),
            trust,
            "dev_setup reuses existing keys"
        );
        // v1 and v2 manifests pass the M-of-N policy; v1 after v2 would be a downgrade.
        for v in [1u32, 2] {
            let m = FirmwareManifest::decode(
                fs::read(issuer.dir().join(format!("firmware-v{v}.manifest")))
                    .unwrap()
                    .as_slice(),
            )
            .unwrap();
            assert!(verify_manifest(&trust, &m, 0).is_ok());
        }
        let _ = fs::remove_dir_all(issuer.dir());
    }

    #[test]
    fn issues_bound_verifiable_credential_with_derived_age_claims() {
        let issuer = Issuer::open(scratch_dir("issue"));
        let trust = issuer.dev_setup().unwrap();
        let nonce = Issuer::new_nonce();
        let (req, id_key) = fake_request(&issuer, &nonce, true);
        let now = 1_900_000_000; // 2030-03-17
        let issued = issuer
            .issue_credential_at(&nonce, &req, &subject(), now)
            .unwrap();
        verify_issued(&trust.authority_root, &issued).unwrap();
        let cred = issued.credential.as_ref().unwrap();
        assert_eq!(cred.subject_key, public_key_bytes(&id_key));
        let claim = |n: &str| {
            issued
                .claims
                .iter()
                .find(|c| c.name == n)
                .unwrap()
                .value
                .clone()
        };
        assert_eq!(claim("nationality"), "BR");
        assert_eq!(claim("age_over_18"), "true");
        assert_eq!(claim("age_over_65"), "false");
        assert!(claim("registry_id").starts_with("TK-"));
        let log = fs::read_to_string(issuer.dir().join("issued.log")).unwrap();
        assert!(log.contains("civic_id=teko1"));
        let _ = fs::remove_dir_all(issuer.dir());
    }

    #[test]
    fn rejects_bad_proof_of_possession_forged_attestation_and_replayed_nonce() {
        let issuer = Issuer::open(scratch_dir("reject"));
        issuer.dev_setup().unwrap();
        let nonce = Issuer::new_nonce();
        let (bad_pop, _) = fake_request(&issuer, &nonce, false);
        assert!(matches!(
            issuer.issue_credential(&nonce, &bad_pop, &subject()),
            Err(IssuerError::InvalidRequest(_))
        ));

        let (mut forged, _) = fake_request(&issuer, &nonce, true);
        let rogue = generate_signing_key(&mut UnwrapErr(SysRng));
        let msg = issue_request_message(&nonce, &forged.pk_identity);
        forged.attestation_signature = teko_crypto::sign_tagged(&rogue, tags::ISSUE_ATTEST, &msg);
        assert!(matches!(
            issuer.issue_credential(&nonce, &forged, &subject()),
            Err(IssuerError::InvalidRequest(_))
        ));

        // Request signed for another nonce must not verify for this one.
        let (good, _) = fake_request(&issuer, &nonce, true);
        assert!(
            issuer
                .issue_credential(&Issuer::new_nonce(), &good, &subject())
                .is_err()
        );
        let _ = fs::remove_dir_all(issuer.dir());
    }

    #[test]
    fn subject_validation_and_age_math() {
        assert_eq!(parse_date("2024-02-29"), Some((2024, 2, 29)));
        assert_eq!(parse_date("2023-02-29"), None);
        assert_eq!(parse_date("1990-13-01"), None);
        assert_eq!(parse_date("90-01-01"), None);
        // 2030-03-17 (1_900_000_000): birthday not yet reached => one year less.
        assert_eq!(age_at((1990, 5, 17), 1_900_000_000), 39);
        assert_eq!(age_at((1990, 3, 17), 1_900_000_000), 40);
        let now = 1_900_000_000;
        for (n, b, c) in [
            ("", "1990-01-01", "BR"),
            ("A", "2090-01-01", "BR"),
            ("A", "1990-01-01", "BRA"),
            ("A\nB", "1990-01-01", "BR"),
        ] {
            let s = Subject {
                name: n.into(),
                birth_date: b.into(),
                nationality: c.into(),
                electoral_state: "SP".into(),
                electoral_municipality: "3550308".into(),
            };
            assert!(
                matches!(
                    build_attributes(&s, now),
                    Err(IssuerError::InvalidSubject(_))
                ),
                "{n:?} {b} {c}"
            );
        }
    }

    #[test]
    fn credential_carries_the_electoral_domicile_and_rejects_a_bad_one() {
        let issuer = Issuer::open(scratch_dir("domicile"));
        issuer.dev_setup().unwrap();
        let nonce = Issuer::new_nonce();
        let (req, _) = fake_request(&issuer, &nonce, true);
        let issued = issuer
            .issue_credential_at(&nonce, &req, &subject(), 1_900_000_000)
            .unwrap();
        let claim = |n: &str| {
            issued
                .claims
                .iter()
                .find(|c| c.name == n)
                .unwrap()
                .value
                .clone()
        };
        assert_eq!(
            (
                claim("electoral_state").as_str(),
                claim("electoral_municipality").as_str()
            ),
            ("SP", "3550308")
        );
        for (uf, mun) in [
            ("sp", "3550308"),
            ("SP", "355"),
            ("", ""),
            ("SPA", "3550308"),
        ] {
            let mut sbj = subject();
            sbj.electoral_state = uf.into();
            sbj.electoral_municipality = mun.into();
            assert!(
                matches!(
                    build_attributes(&sbj, 1_900_000_000),
                    Err(IssuerError::InvalidSubject(_))
                ),
                "{uf} {mun}"
            );
        }
        let _ = fs::remove_dir_all(issuer.dir());
    }

    #[test]
    fn demo_election_is_signed_by_governance_and_replays_with_four_phases() {
        use teko_election::{GovernancePolicy, Phase, replay};
        let issuer = Issuer::open(scratch_dir("election"));
        let trust = issuer.dev_setup().unwrap();
        assert_eq!((trust.gov_signers.len(), trust.gov_threshold), (3, 2));
        let bundle = ElectionBundle::decode(
            fs::read(issuer.dir().join("election.bundle"))
                .unwrap()
                .as_slice(),
        )
        .unwrap();
        let manifest = ElectionManifest::decode(bundle.manifest.as_slice()).unwrap();
        let ts: Vec<Transition> = bundle
            .transitions
            .iter()
            .map(|t| Transition::decode(t.as_slice()).unwrap())
            .collect();
        let policy = GovernancePolicy::new(trust.gov_signers.clone(), trust.gov_threshold).unwrap();
        let status = replay(&policy, &manifest, &ts).unwrap();
        assert_eq!(status.phase_of("presidente"), Some(Phase::VotingOpen));
        assert_eq!(
            status.phase_of("governador-sp"),
            Some(Phase::RegistrationOpen)
        );
        assert_eq!(status.phase_of("governador-rj"), Some(Phase::VotingOpen));
        assert_eq!(
            status.phase_of("prefeito-3550308"),
            Some(Phase::CandidatesPublished)
        );
        // re-running refreshes the dates but keeps the keys (idempotent)
        assert_eq!(issuer.dev_setup().unwrap(), trust);
        let _ = fs::remove_dir_all(issuer.dir());
    }

    #[test]
    fn firmware_blob_is_deterministic_and_attestation_unaffected() {
        assert_eq!(make_firmware_blob(1, 1000), make_firmware_blob(1, 1000));
        assert_ne!(make_firmware_blob(1, 1000), make_firmware_blob(2, 1000));
        let issuer = Issuer::open(scratch_dir("att"));
        let trust = issuer.dev_setup().unwrap();
        let f = issuer.provision_device().unwrap();
        let batch = SigningKey::from_slice(&f.batch_key).unwrap();
        let sig = teko_attestation::sign_attestation(&batch, b"0123456789abcdef", 1, 0);
        assert!(
            verify_attestation(&trust, b"0123456789abcdef", 1, 0, &[f.batch_cert], &sig).is_ok()
        );
        let _ = fs::remove_dir_all(issuer.dir());
    }
}
