//! Simulated civic authority ("issuer"). TEST KEYS ONLY.
//!
//! - generates the test root, firmware-signer and batch keys;
//! - signs firmware manifests (M-of-N);
//! - provisions "factory" attestation material for emulated devices;
//! - verifies a device's identity request (proof of possession + batch
//!   attestation) and issues a citizen credential bound to its identity key.
//!
//! - ANONYMOUS VOTER REGISTRATION (ADR-0008): per-contest RSA keys; blind-signs a
//!   registration after checking identity, domicile, phase and once-per-citizen.
//!   It sees WHO asks, never the voter key. The token it enables travels through
//!   a separate channel (the board), never back through this exchange.

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
    Office, Phase, ResultsVisibility, Schedule, Scope, Transition, contest_hash, is_eligible,
    registration_request_message, replay, sign_manifest as sign_election_manifest, sign_transition,
};
use teko_protocol::{IdentityRequest, RegistrationRequest};
use teko_registration::IssuerKey;
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
    #[error("registro recusado: {0}")]
    RegistrationRefused(String),
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

/// One credential of one citizen, as the authority recorded it.
#[derive(Clone, Debug)]
struct CitizenRec {
    citizen_ref: String,
    credential_id: String,
    civic_id: String,
    digest: String,
    revoked: bool,
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
            chain_id: String::new(),
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
        if !self.dir.join("election.bundle").exists() {
            self.dev_election(now_secs())?;
        }
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

    /// Writes `election.bundle`: a demo election on DIFFERENT dates and states:
    /// President (national), governors of SP (12 candidates, to show "top 10 +
    /// others"), RJ, MG, BA and RS (voting open), and the Mayor of São Paulo
    /// (candidates announced only). Dates are relative to `now`.
    ///
    /// A NEW bundle is a NEW election (the id carries today's date): the board and
    /// the "simulated data" marker of the previous one are removed.
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
        let open = (now - 5 * DAY, now - DAY, now + 3 * DAY);
        let announced = (now + DAY, now + 3 * DAY, now + 4 * DAY);
        // One RSA key PER CONTEST (reused if it exists), so a registration for one
        // contest is worthless in any other.
        let reg_key = |id: &str| -> Result<Vec<u8>, IssuerError> {
            let key = match self.registration_key(id) {
                Ok(k) => k,
                Err(_) => {
                    let k = IssuerKey::generate(2048)
                        .map_err(|e| IssuerError::Corrupt(e.to_string()))?;
                    let der = k
                        .to_pkcs8_der()
                        .map_err(|e| IssuerError::Corrupt(e.to_string()))?;
                    fs::create_dir_all(self.dir.join("registration"))?;
                    write_private(&self.reg_key_path(id), &hex::encode(&der))?;
                    k
                }
            };
            key.public_spki()
                .map_err(|e| IssuerError::Corrupt(e.to_string()))
        };
        let contest = |id: &str,
                       office: Office,
                       title: &str,
                       scope: Scope,
                       o: Vec<ElectionOption>,
                       s: (u64, u64, u64)|
         -> Result<Contest, IssuerError> {
            Ok(Contest {
                contest_id: id.to_string(),
                office: office as i32,
                title: title.to_string(),
                scope: Some(scope),
                options: o,
                registration_key: reg_key(id)?,
                schedule: Some(Schedule {
                    registration_opens: s.0,
                    voting_opens: s.1,
                    voting_closes: s.2,
                }),
            })
        };
        // If a local test network exists (`dev/net/network.txt`), the election is bound
        // to its chain_id and the device trust file is updated to accept only that
        // network (restart the emulator to pick it up).
        let net_chain_id = fs::read_to_string(self.dir.join("net").join("network.txt"))
            .ok()
            .and_then(|t| {
                t.lines().find_map(|l| {
                    l.trim()
                        .strip_prefix("chain_id=")
                        .map(|v| v.trim().to_string())
                })
            })
            .unwrap_or_default();
        if !net_chain_id.is_empty() {
            let mut trust = self.trust()?;
            trust.chain_id = net_chain_id.clone();
            fs::write(self.dir.join("trust.txt"), trust.to_text())?;
        }
        // Encrypted ballots: if the validators already ran the key ceremony
        // (`dev/net/tally-key.pub`), the election carries its public key.
        let tally_key = fs::read_to_string(self.dir.join("net").join("tally-key.pub"))
            .ok()
            .and_then(|t| hex::decode(t.trim()).ok())
            .unwrap_or_default();
        let (y, m, d) = civil_from_days((now / 86_400) as i64);
        let contests: Vec<Contest> = vec![
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
                open,
            ),
            contest(
                "governador-sp",
                Office::Governor,
                "Governador de São Paulo",
                Scope::state("SP"),
                opts(
                    &[
                        "Candidato Guará",
                        "Candidata Vitória-Régia",
                        "Candidato Lobo-Guará",
                        "Candidata Arara-Canindé",
                        "Candidato Tamanduá-Bandeira",
                        "Candidata Capivara",
                        "Candidato Jacaré-do-Papo-Amarelo",
                        "Candidata Onça-Parda",
                        "Candidato Mico-Leão-Dourado",
                        "Candidata Ararinha-Azul",
                        "Candidato Tatu-Canastra",
                        "Candidata Sabiá-Laranjeira",
                    ],
                    20,
                ),
                open,
            ),
            contest(
                "governador-rj",
                Office::Governor,
                "Governador do Rio de Janeiro",
                Scope::state("RJ"),
                opts(
                    &[
                        "Candidato Mico-Leão-Dourado",
                        "Candidata Capivara",
                        "Candidato Boto-Tucuxi",
                    ],
                    40,
                ),
                open,
            ),
            contest(
                "governador-mg",
                Office::Governor,
                "Governador de Minas Gerais",
                Scope::state("MG"),
                opts(
                    &[
                        "Candidato Lobo-Guará",
                        "Candidata Onça-Parda",
                        "Candidato Tamanduá-Bandeira",
                        "Candidata Seriema",
                    ],
                    50,
                ),
                open,
            ),
            contest(
                "governador-ba",
                Office::Governor,
                "Governador da Bahia",
                Scope::state("BA"),
                opts(
                    &[
                        "Candidata Arara-Vermelha",
                        "Candidato Tartaruga-Marinha",
                        "Candidata Baleia-Jubarte",
                    ],
                    60,
                ),
                open,
            ),
            contest(
                "governador-rs",
                Office::Governor,
                "Governador do Rio Grande do Sul",
                Scope::state("RS"),
                opts(
                    &[
                        "Candidato Quero-Quero",
                        "Candidata Graxaim",
                        "Candidato Zorrilho",
                        "Candidata Ema",
                    ],
                    70,
                ),
                open,
            ),
            contest(
                "prefeito-3550308",
                Office::Mayor,
                "Prefeito de São Paulo",
                Scope::municipality("3550308"),
                opts(&["Candidata Jaguatirica", "Candidato Tucano"], 30),
                announced,
            ),
        ]
        .into_iter()
        .collect::<Result<_, _>>()?;
        let mut manifest = ElectionManifest {
            format_version: 2,
            election_id: format!("eleicoes-exemplo-{y:04}{m:02}{d:02}"),
            title: "Eleições de exemplo (protótipo, dados fictícios)".to_string(),
            contests,
            ballot_kind: BALLOT_KIND_PUBLIC.to_string(),
            registry_key: Vec::new(),
            revision: 1,
            signatures: Vec::new(),
            results_visibility: ResultsVisibility::AfterClose as i32,
            chain_id: net_chain_id.clone(),
            tally_key,
        };
        manifest.signatures = [0usize, 1]
            .iter()
            .map(|i| sign_election_manifest(&gov[*i], &manifest))
            .collect();

        // Governance decisions, each contest on ITS OWN schedule.
        let mut transitions: Vec<Transition> = Vec::new();
        for c in &manifest.contests {
            let upto = if c.contest_id.starts_with("prefeito") {
                Phase::CandidatesPublished
            } else {
                Phase::VotingOpen
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
                    contest_id: c.contest_id.clone(),
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
        // A new election starts with a clean board and no "simulated" marker.
        for stale in ["board.log", "board.simulated"] {
            let _ = fs::remove_file(self.dir.join(stale));
        }
        Ok(())
    }

    fn reg_key_path(&self, contest_id: &str) -> PathBuf {
        self.dir.join("registration").join(format!(
            "{}.key",
            contest_id.replace(|c: char| !c.is_ascii_alphanumeric() && c != '-', "_")
        ))
    }

    /// SECRET key of one contest's registration authority.
    pub fn registration_key(&self, contest_id: &str) -> Result<IssuerKey, IssuerError> {
        let path = self.reg_key_path(contest_id);
        let text = fs::read_to_string(&path).map_err(|_| IssuerError::NotInitialized)?;
        let der = hex::decode(text.trim())
            .map_err(|_| IssuerError::Corrupt(path.display().to_string()))?;
        IssuerKey::from_pkcs8_der(&der)
            .map_err(|_| IssuerError::Corrupt(path.display().to_string()))
    }

    /// Anonymous registration (ADR-0008). Checks, in order: the credential is ours
    /// and unexpired, the identity key matches it and proves possession of THIS
    /// request, the contest requires registration, the domicile covers it, the
    /// contest is in `RegistrationOpen`/`VotingOpen`, and the citizen has not
    /// registered here already (a RETRY of the same blinded message is re-signed).
    /// Then blind-signs. Returns the blind signature.
    pub fn sign_registration(
        &self,
        election_id: &str,
        contest_id: &str,
        request: &RegistrationRequest,
        now: u64,
    ) -> Result<Vec<u8>, IssuerError> {
        let no = |m: &str| IssuerError::RegistrationRefused(m.to_string());
        let trust = self.trust()?;
        let issued = IssuedCredential::decode(request.credential.as_slice())
            .map_err(|_| no("credencial malformada"))?;
        credential::verify_issued(&trust.authority_root, &issued)
            .map_err(|_| no("credencial não foi emitida por esta autoridade"))?;
        let cred = issued
            .credential
            .as_ref()
            .ok_or_else(|| no("credencial vazia"))?;
        if cred.expires_at < now {
            return Err(no("credencial expirada"));
        }
        if cred.subject_key != request.pk_identity {
            return Err(no("chave de identidade não confere com a credencial"));
        }
        verify_tagged(
            &request.pk_identity,
            tags::REGISTRATION_REQUEST,
            &registration_request_message(election_id, contest_id, &request.blind_message),
            &request.pop_signature,
        )
        .map_err(|_| no("prova de posse inválida para este pedido"))?;

        let bundle_bytes = fs::read(self.dir.join("election.bundle"))
            .map_err(|_| no("nenhuma eleição configurada"))?;
        let bundle = ElectionBundle::decode(bundle_bytes.as_slice())
            .map_err(|_| IssuerError::Corrupt("election.bundle".into()))?;
        let manifest = ElectionManifest::decode(bundle.manifest.as_slice())
            .map_err(|_| IssuerError::Corrupt("election.bundle".into()))?;
        let transitions = bundle
            .transitions
            .iter()
            .map(|t| Transition::decode(t.as_slice()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| IssuerError::Corrupt("election.bundle".into()))?;
        let policy =
            teko_election::GovernancePolicy::new(trust.gov_signers.clone(), trust.gov_threshold)
                .map_err(|_| IssuerError::Corrupt("trust.txt (governança)".into()))?;
        let status = replay(&policy, &manifest, &transitions)
            .map_err(|_| IssuerError::Corrupt("election.bundle (histórico)".into()))?;
        if manifest.election_id != election_id {
            return Err(no("outra eleição"));
        }
        let contest = manifest
            .contest(contest_id)
            .ok_or_else(|| no("pleito desconhecido"))?;
        if contest.registration_key.is_empty() {
            return Err(no("este pleito não exige registro"));
        }
        let domicile = Domicile::new(
            &claim_value(&issued, "electoral_state")
                .ok_or_else(|| no("sem domicílio eleitoral"))?,
            &claim_value(&issued, "electoral_municipality")
                .ok_or_else(|| no("sem domicílio eleitoral"))?,
        )
        .map_err(|_| no("domicílio eleitoral inválido"))?;
        if !is_eligible(&domicile, &contest.scope.clone().unwrap_or_default()) {
            return Err(no("fora do domicílio eleitoral deste pleito"));
        }
        if !matches!(
            status.phase_of(contest_id),
            Some(Phase::RegistrationOpen | Phase::VotingOpen)
        ) {
            return Err(no("registro não está aberto neste pleito"));
        }

        // Once per (citizen, contest). The ledger holds WHO registered, and a hash of
        // the blinded message: nothing that could be matched to a voter key.
        // Keyed by the CITIZEN, not by the device key: a reissued citizen is still the
        // same one, so a new device cannot register a second time.
        let cred_hex = hex::encode(&cred.credential_id);
        if let Some(list) = self.revocation_list()?
            && list.is_revoked(&cred.credential_id)
        {
            return Err(no("credencial revogada: peça a reemissão presencial"));
        }
        let who = self
            .citizens()?
            .into_iter()
            .find(|c| c.credential_id == cred_hex)
            .map_or_else(
                || civic_id(&request.pk_identity).unwrap_or_default(),
                |c| c.citizen_ref,
            );
        let blinded = hex::encode(&tagged_hash(
            "teko/v2/registration-ledger",
            &[&request.blind_message],
        ));
        let ledger = self.dir.join("registrations.log");
        let existing = fs::read_to_string(&ledger).unwrap_or_default();
        let prefix = format!("{contest_id} {who} ");
        if let Some(line) = existing.lines().find(|l| l.starts_with(&prefix)) {
            if line[prefix.len()..].trim() != blinded {
                return Err(no(
                    "você já se registrou neste pleito (talvez com um dispositivo anterior): o registro antigo continua valendo",
                ));
            }
        } else {
            let mut f = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&ledger)?;
            writeln!(f, "{prefix}{blinded}")?;
        }
        let sig = self
            .registration_key(contest_id)?
            .blind_sign(&request.blind_message)
            .map_err(|_| no("mensagem cega inválida"))?;
        Ok(sig.0)
    }

    /// SIMULATED governance operators: for every contest still in `VotingOpen`
    /// whose `voting_closes` has passed, signs (2 of 3) the `VotingClosed`
    /// transition and appends it to `election.bundle`. Returns how many were added.
    /// From that point on the PUBLIC tally refuses further ballots for the contest
    /// (the log, not a clock, is what locks the vote).
    pub fn close_due(&self, now: u64) -> Result<usize, IssuerError> {
        let corrupt = || IssuerError::Corrupt("election.bundle".into());
        let path = self.dir.join("election.bundle");
        let mut bundle = ElectionBundle::decode(fs::read(&path).map_err(|_| corrupt())?.as_slice())
            .map_err(|_| corrupt())?;
        let manifest =
            ElectionManifest::decode(bundle.manifest.as_slice()).map_err(|_| corrupt())?;
        let mut transitions = bundle
            .transitions
            .iter()
            .map(|t| Transition::decode(t.as_slice()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| corrupt())?;
        let trust = self.trust()?;
        let policy =
            teko_election::GovernancePolicy::new(trust.gov_signers.clone(), trust.gov_threshold)
                .map_err(|_| IssuerError::Corrupt("trust.txt (governança)".into()))?;
        let mut status = replay(&policy, &manifest, &transitions).map_err(|_| corrupt())?;
        let gov: Vec<SigningKey> = (0..GOV_SIGNERS)
            .map(|i| load_key(&self.dir.join(format!("gov-{i}.key"))))
            .collect::<Result<_, _>>()?;
        let mut added = 0;
        for c in &manifest.contests {
            let closes = c.schedule.as_ref().map_or(u64::MAX, |s| s.voting_closes);
            if now < closes || status.phase_of(&c.contest_id) != Some(Phase::VotingOpen) {
                continue;
            }
            let mut t = Transition {
                election_id: manifest.election_id.clone(),
                contest_id: c.contest_id.clone(),
                contest_hash: contest_hash(c).to_vec(),
                from_phase: Phase::VotingOpen as i32,
                to_phase: Phase::VotingClosed as i32,
                seq: status.seq + 1,
                at: closes,
                reason: String::new(),
                signatures: Vec::new(),
            };
            t.signatures = [1usize, 2]
                .iter()
                .map(|k| sign_transition(&gov[*k], &t))
                .collect();
            bundle.transitions.push(t.encode_to_vec());
            transitions.push(t);
            // The history must still replay (M-of-N, order, phases) with the new step.
            status = replay(&policy, &manifest, &transitions)
                .map_err(|_| IssuerError::Corrupt("fechamento recusado pela governança".into()))?;
            added += 1;
        }
        if added > 0 {
            fs::write(&path, bundle.encode_to_vec())?;
        }
        Ok(added)
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

    /// FIRST issuance for a person: verifies the device (genuine + owns the key)
    /// and issues the credential bound to its identity key. A person who already
    /// has a record is refused: they must use the in-person REISSUE (which revokes
    /// the old credential), otherwise one person could hold two identities and
    /// register twice.
    pub fn issue_credential_at(
        &self,
        nonce: &[u8],
        request: &IdentityRequest,
        subject: &Subject,
        now: u64,
    ) -> Result<IssuedCredential, IssuerError> {
        self.check_request(nonce, request)?;
        let digest = self.person_digest(subject)?;
        if self.citizens()?.iter().any(|c| c.digest == digest) {
            return Err(IssuerError::InvalidRequest(
                "esta pessoa já tem cadastro: use a REEMISSÃO presencial (a credencial anterior será revogada)".into(),
            ));
        }
        self.mint(request, subject, now, None, &digest)
    }

    /// In-person REISSUE after a lost, stolen or replaced device (no seed exists,
    /// ADR-0003). The authority re-verifies the person (the data must match what it
    /// recorded at the first issuance), REVOKES every active credential of that
    /// citizen, and issues a new one for the new device, linked to the SAME citizen
    /// so "once per citizen and contest" keeps holding. Nothing here ever sees a
    /// voter key.
    pub fn reissue_credential_at(
        &self,
        nonce: &[u8],
        request: &IdentityRequest,
        subject: &Subject,
        old_civic_id: &str,
        reason: &str,
        now: u64,
    ) -> Result<IssuedCredential, IssuerError> {
        // Everything is checked BEFORE anything is revoked.
        self.check_request(nonce, request)?;
        let digest = self.person_digest(subject)?;
        let bad = |m: &str| IssuerError::InvalidRequest(m.to_string());
        let all = self.citizens()?;
        let old = all
            .iter()
            .rev()
            .find(|c| c.civic_id == old_civic_id)
            .ok_or_else(|| bad("ID cívico anterior desconhecido"))?;
        if old.digest != digest {
            return Err(bad(
                "os dados da pessoa não conferem com o cadastro original",
            ));
        }
        if civic_id(&request.pk_identity).unwrap_or_default() == old.civic_id {
            return Err(bad("o dispositivo novo tem a mesma chave do anterior"));
        }
        let citizen_ref = old.citizen_ref.clone();
        self.revoke_citizen(&citizen_ref, reason, now)?;
        self.mint(request, subject, now, Some(citizen_ref), &digest)
    }

    /// Immediate block of a stolen/lost device, with no new device yet. The person
    /// can still be reissued later.
    pub fn revoke_civic_id(
        &self,
        civic_id: &str,
        reason: &str,
        now: u64,
    ) -> Result<usize, IssuerError> {
        let all = self.citizens()?;
        let c = all
            .iter()
            .rev()
            .find(|c| c.civic_id == civic_id)
            .ok_or_else(|| IssuerError::InvalidRequest("ID cívico desconhecido".into()))?;
        self.revoke_citizen(&c.citizen_ref.clone(), reason, now)
    }

    /// The signed revocation list (None until the first revocation).
    pub fn revocation_list(&self) -> Result<Option<credential::RevocationList>, IssuerError> {
        match fs::read(self.dir.join("revocations.signed")) {
            Ok(b) => credential::RevocationList::decode(b.as_slice())
                .map(Some)
                .map_err(|_| IssuerError::Corrupt("revocations.signed".into())),
            Err(_) => Ok(None),
        }
    }

    /// DEV ONLY: forgets every citizen, registration and revocation (use with
    /// `make reset`, when the test device is wiped to start over).
    pub fn dev_forget_citizens(&self) {
        for f in [
            "citizens.log",
            "registrations.log",
            "revocations.signed",
            "issued.log",
        ] {
            let _ = fs::remove_file(self.dir.join(f));
        }
    }

    fn secret(&self) -> Result<Vec<u8>, IssuerError> {
        Ok(load_key(&self.dir.join("root.key"))?.to_bytes().to_vec())
    }

    /// Stable fingerprint of WHO the person is (name, birth, nationality), keyed by an
    /// authority secret so the log reveals nothing by itself.
    fn person_digest(&self, s: &Subject) -> Result<String, IssuerError> {
        let secret = self.secret()?;
        let name = s.name.trim().to_lowercase();
        Ok(hex::encode(&tagged_hash(
            "teko/v2/citizen-data",
            &[
                secret.as_slice(),
                name.as_bytes(),
                s.birth_date.trim().as_bytes(),
                s.nationality.trim().to_uppercase().as_bytes(),
            ],
        )))
    }

    fn citizens(&self) -> Result<Vec<CitizenRec>, IssuerError> {
        let text = fs::read_to_string(self.dir.join("citizens.log")).unwrap_or_default();
        let mut out: Vec<CitizenRec> = Vec::new();
        for line in text.lines() {
            let f: Vec<&str> = line.split_whitespace().collect();
            match f.as_slice() {
                ["issued", citizen_ref, credential_id, civic, digest, _ts] => {
                    out.push(CitizenRec {
                        citizen_ref: (*citizen_ref).into(),
                        credential_id: (*credential_id).into(),
                        civic_id: (*civic).into(),
                        digest: (*digest).into(),
                        revoked: false,
                    })
                }
                ["revoked", credential_id, _reason, _ts] => {
                    for c in out.iter_mut().filter(|c| c.credential_id == *credential_id) {
                        c.revoked = true;
                    }
                }
                _ => {}
            }
        }
        Ok(out)
    }

    fn append_event(&self, line: &str) -> Result<(), IssuerError> {
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join("citizens.log"))?;
        writeln!(f, "{line}")?;
        Ok(())
    }

    /// Revokes every ACTIVE credential of the citizen and re-signs the list.
    fn revoke_citizen(
        &self,
        citizen_ref: &str,
        reason: &str,
        now: u64,
    ) -> Result<usize, IssuerError> {
        let reason: String = reason
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .take(24)
            .collect();
        let reason = if reason.is_empty() {
            "unspecified".to_string()
        } else {
            reason
        };
        let active: Vec<CitizenRec> = self
            .citizens()?
            .into_iter()
            .filter(|c| c.citizen_ref == citizen_ref && !c.revoked)
            .collect();
        for c in &active {
            self.append_event(&format!("revoked {} {reason} {now}", c.credential_id))?;
        }
        self.write_revocation_list()?;
        Ok(active.len())
    }

    fn write_revocation_list(&self) -> Result<(), IssuerError> {
        let root = load_key(&self.dir.join("root.key"))?;
        let text = fs::read_to_string(self.dir.join("citizens.log")).unwrap_or_default();
        let mut ids = Vec::new();
        let mut seq = 0u64;
        for line in text.lines() {
            let f: Vec<&str> = line.split_whitespace().collect();
            if let ["revoked", id, ..] = f.as_slice() {
                seq += 1;
                if let Ok(b) = hex::decode(id) {
                    ids.push(b);
                }
            }
        }
        let list = credential::sign_revocation_list(&root, seq, ids);
        fs::write(self.dir.join("revocations.signed"), list.encode_to_vec())?;
        Ok(())
    }

    fn check_request(&self, nonce: &[u8], request: &IdentityRequest) -> Result<(), IssuerError> {
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
        Ok(())
    }

    /// Signs the credential and records the citizen. `link`: the citizen this
    /// credential continues (reissue), or a brand-new citizen.
    fn mint(
        &self,
        request: &IdentityRequest,
        subject: &Subject,
        now: u64,
        link: Option<String>,
        digest: &str,
    ) -> Result<IssuedCredential, IssuerError> {
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
        // Authority-side record (identity side only; never touches voting).
        let cid = civic_id(&request.pk_identity).unwrap_or_default();
        let citizen_ref = link.unwrap_or_else(|| {
            hex::encode(&tagged_hash(
                "teko/v2/citizen-ref",
                &[self.secret().unwrap_or_default().as_slice(), &credential_id],
            ))
        });
        self.append_event(&format!(
            "issued {citizen_ref} {} {cid} {digest} {now}",
            hex::encode(&credential_id)
        ))?;
        let line = format!(
            "{now} credential={} civic_id={cid}\n",
            hex::encode(&credential_id)
        );
        let mut log = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join("issued.log"))?;
        log.write_all(line.as_bytes())?;
        Ok(issued)
    }
}

fn claim_value(issued: &IssuedCredential, name: &str) -> Option<String> {
    issued
        .claims
        .iter()
        .find(|c| c.name == name)
        .map(|c| c.value.clone())
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
    fn demo_election_is_signed_by_governance_and_replays_with_open_and_announced_contests() {
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
        for open in [
            "presidente",
            "governador-sp",
            "governador-rj",
            "governador-mg",
            "governador-ba",
            "governador-rs",
        ] {
            assert_eq!(status.phase_of(open), Some(Phase::VotingOpen), "{open}");
        }
        assert_eq!(
            status.phase_of("prefeito-3550308"),
            Some(Phase::CandidatesPublished)
        );
        assert_eq!(
            manifest.contest("governador-sp").unwrap().options.len(),
            12,
            "top-10 + others demo"
        );
        assert!(manifest.election_id.starts_with("eleicoes-exemplo-"));
        // re-running dev-setup keeps the SAME election (it is only recreated on purpose)
        let before = fs::read(issuer.dir().join("election.bundle")).unwrap();
        assert_eq!(issuer.dev_setup().unwrap(), trust);
        assert_eq!(
            before,
            fs::read(issuer.dir().join("election.bundle")).unwrap()
        );
        // a deliberate refresh removes the old board and the simulated marker
        fs::write(issuer.dir().join("board.log"), b"x").unwrap();
        fs::write(issuer.dir().join("board.simulated"), b"x").unwrap();
        issuer.dev_election(1_900_000_000).unwrap();
        assert!(
            !issuer.dir().join("board.log").exists()
                && !issuer.dir().join("board.simulated").exists()
        );
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

    // ───────────────────────── reissue and revocation ─────────────────────────

    fn now() -> u64 {
        1_800_000_000
    }

    fn setup(tag: &str) -> Issuer {
        let i = Issuer::open(scratch_dir(tag));
        i.dev_setup().unwrap();
        i
    }

    fn first_issue(i: &Issuer) -> (credential::IssuedCredential, SigningKey, String) {
        let nonce = Issuer::new_nonce();
        let (req, key) = fake_request(i, &nonce, true);
        let issued = i
            .issue_credential_at(&nonce, &req, &subject(), now())
            .unwrap();
        let cid = civic_id(&req.pk_identity).unwrap();
        (issued, key, cid)
    }

    fn cred_id(c: &credential::IssuedCredential) -> Vec<u8> {
        c.credential.as_ref().unwrap().credential_id.clone()
    }

    #[test]
    fn a_person_cannot_get_a_second_identity_without_a_reissue() {
        let i = setup("dup");
        first_issue(&i);
        let nonce = Issuer::new_nonce();
        let (req, _) = fake_request(&i, &nonce, true);
        let err = i
            .issue_credential_at(&nonce, &req, &subject(), now())
            .unwrap_err();
        assert!(err.to_string().contains("REEMISSÃO"), "{err}");
        // same person with different capitalisation / spacing is still the same person
        let mut s = subject();
        s.name = "  MARIA DA SILVA ".into();
        assert!(i.issue_credential_at(&nonce, &req, &s, now()).is_err());
        let _ = fs::remove_dir_all(i.dir());
    }

    #[test]
    fn reissue_revokes_the_old_credential_and_keeps_the_same_citizen() {
        let i = setup("reissue");
        let (old, _, old_civic) = first_issue(&i);
        assert!(i.revocation_list().unwrap().is_none());

        let nonce = Issuer::new_nonce();
        let (req, _) = fake_request(&i, &nonce, true);
        let new = i
            .reissue_credential_at(&nonce, &req, &subject(), &old_civic, "lost", now() + 10)
            .unwrap();
        verify_issued(&i.trust().unwrap().authority_root, &new).unwrap();
        assert_ne!(cred_id(&old), cred_id(&new));

        // the signed list now holds the OLD id only, and verifies under the authority root
        let list = i.revocation_list().unwrap().unwrap();
        credential::verify_revocation_list(&i.trust().unwrap().authority_root, &list).unwrap();
        assert!(list.is_revoked(&cred_id(&old)) && !list.is_revoked(&cred_id(&new)));
        assert_eq!(list.seq, 1);

        // same citizen across both credentials
        let c = i.citizens().unwrap();
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].citizen_ref, c[1].citizen_ref);
        assert!(c[0].revoked && !c[1].revoked);

        // reissuing AGAIN revokes the previous one too (and only the active ones)
        let nonce = Issuer::new_nonce();
        let (req2, _) = fake_request(&i, &nonce, true);
        let new_civic = civic_id(&req.pk_identity).unwrap();
        let third = i
            .reissue_credential_at(&nonce, &req2, &subject(), &new_civic, "stolen", now() + 20)
            .unwrap();
        let list = i.revocation_list().unwrap().unwrap();
        assert!(list.is_revoked(&cred_id(&new)) && !list.is_revoked(&cred_id(&third)));
        assert_eq!(list.seq, 2);
        let _ = fs::remove_dir_all(i.dir());
    }

    #[test]
    fn reissue_is_refused_for_the_wrong_person_unknown_ids_bad_proofs_and_the_same_key() {
        let i = setup("refuse");
        let (_, _, old_civic) = first_issue(&i);
        let nonce = Issuer::new_nonce();
        let (req, _) = fake_request(&i, &nonce, true);
        let mut other = subject();
        other.name = "Outra Pessoa".into();
        let e = i
            .reissue_credential_at(&nonce, &req, &other, &old_civic, "lost", now())
            .unwrap_err();
        assert!(e.to_string().contains("não conferem"), "{e}");
        assert!(
            i.reissue_credential_at(&nonce, &req, &subject(), "teko1inexistente", "lost", now())
                .is_err()
        );
        let (bad_pop, _) = fake_request(&i, &nonce, false);
        assert!(
            i.reissue_credential_at(&nonce, &bad_pop, &subject(), &old_civic, "lost", now())
                .is_err()
        );
        // nothing was revoked by any refused attempt
        assert!(i.revocation_list().unwrap().is_none());
        let _ = fs::remove_dir_all(i.dir());
    }

    #[test]
    fn immediate_revoke_blocks_the_old_credential_and_allows_a_later_reissue() {
        let i = setup("revoke");
        let (old, _, old_civic) = first_issue(&i);
        assert_eq!(i.revoke_civic_id(&old_civic, "stolen", now()).unwrap(), 1);
        assert_eq!(
            i.revoke_civic_id(&old_civic, "stolen", now()).unwrap(),
            0,
            "idempotent"
        );
        assert!(
            i.revocation_list()
                .unwrap()
                .unwrap()
                .is_revoked(&cred_id(&old))
        );
        let nonce = Issuer::new_nonce();
        let (req, _) = fake_request(&i, &nonce, true);
        i.reissue_credential_at(&nonce, &req, &subject(), &old_civic, "stolen", now())
            .unwrap();
        let _ = fs::remove_dir_all(i.dir());
    }

    #[test]
    fn a_tampered_revocation_list_is_rejected() {
        let i = setup("tamper");
        let (_, _, old_civic) = first_issue(&i);
        i.revoke_civic_id(&old_civic, "lost", now()).unwrap();
        let mut list = i.revocation_list().unwrap().unwrap();
        list.revoked.clear(); // an attacker "un-revokes"
        assert!(
            credential::verify_revocation_list(&i.trust().unwrap().authority_root, &list).is_err()
        );
        let _ = fs::remove_dir_all(i.dir());
    }
}
