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
use rand::rand_core::UnwrapErr;
use rand::rngs::SysRng;
use teko_attestation::{
    FactoryData, FirmwareManifest, TrustPolicy, blob_hash, issue_batch_cert, sign_manifest,
};
use teko_crypto::civic::{
    CivicBundle, CivicPolicy, CivicRecord, claim_list, commitment_of, verify_civic_bundle,
};
use teko_crypto::credential::Claim;
use teko_crypto::{
    SigningKey, generate_signing_key, hex, public_key_bytes, tagged_hash, tags, verify_tagged,
};
use teko_election::{
    BALLOT_KIND_PUBLIC, Contest, Domicile, ElectionBundle, ElectionManifest, ElectionOption,
    Office, Phase, ResultsVisibility, Schedule, Scope, Transition, contest_hash, is_eligible,
    registration_request_message, replay, sign_manifest as sign_election_manifest, sign_transition,
};
use teko_protocol::RegistrationRequest;
use teko_registration::IssuerKey;
use thiserror::Error;

pub const FW_SIGNERS: usize = 3;
pub const FW_THRESHOLD: usize = 2;
pub const GOV_SIGNERS: usize = 3;
pub const GOV_THRESHOLD: usize = 2;
pub const ISSUER_NAME: &str = "Autoridade Cívica (simulada)";

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

/// One candidate of a contest, taken from an on-chain `candidato` credential (ADR-0040).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevCandidate {
    pub credential_id: Vec<u8>,
    pub numero: String,
    pub nome: String,
}

/// Candidates by contest id.
pub type DevCandidates = std::collections::BTreeMap<String, Vec<DevCandidate>>;

/// A contest of the sample election, as far as the candidates need to know.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevContestPlan {
    pub id: &'static str,
    /// `presidente`, `governador` or `prefeito`.
    pub cargo: &'static str,
    /// Empty (national), the UF or the IBGE code.
    pub scope_code: &'static str,
    /// When voting closes (unix seconds), with the same calendar `dev_election` uses.
    pub voting_closes: u64,
}

/// The contests of the sample election: id, office, jurisdiction and closing time, relative to `now`.
pub fn dev_contest_plan(now: u64) -> Vec<DevContestPlan> {
    const DAY: u64 = 86_400;
    let (open, announced) = (now + 3 * DAY, now + 4 * DAY);
    let c = |id, cargo, scope_code, voting_closes| DevContestPlan {
        id,
        cargo,
        scope_code,
        voting_closes,
    };
    vec![
        c("presidente", "presidente", "", open),
        c("governador-sp", "governador", "SP", open),
        c("governador-rj", "governador", "RJ", open),
        c("governador-mg", "governador", "MG", open),
        c("governador-ba", "governador", "BA", open),
        c("governador-rs", "governador", "RS", open),
        c("prefeito-3550308", "prefeito", "3550308", announced),
    ]
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
            chain_id: String::new(),
            ..TrustPolicy::default()
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
        self.dev_election_with(now, None, &DevCandidates::new())
    }

    /// Like `dev_election`, with the candidates of some contests taken from the CHAIN (ADR-0040):
    /// `cands` maps a contest id to its candidates (each referencing its `candidato` credential), and
    /// `election_id` names the election the candidacies were issued for. A contest without an entry
    /// keeps the example list.
    pub fn dev_election_with(
        &self,
        now: u64,
        election_id: Option<&str>,
        cands: &DevCandidates,
    ) -> Result<(), IssuerError> {
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
                    ..Default::default()
                })
                .collect()
        };
        let open = (now - 5 * DAY, now - DAY, now + 3 * DAY);
        let announced = (now + DAY, now + 3 * DAY, now + 4 * DAY);
        // One RSA key PER CONTEST (reused if it exists), so a registration for one
        // contest is worthless in any other.
        let reg_key = |id: &str| self.registration_spki(id);
        let contest = |id: &str,
                       office: Office,
                       title: &str,
                       scope: Scope,
                       o: Vec<ElectionOption>,
                       s: (u64, u64, u64)|
         -> Result<Contest, IssuerError> {
            // candidates from the chain replace the example list of this contest
            let options = match cands.get(id) {
                Some(list) => list
                    .iter()
                    .enumerate()
                    .map(|(i, c)| ElectionOption {
                        index: i as u32,
                        name: c.nome.clone(),
                        number: c.numero.clone(),
                        credential_id: c.credential_id.clone(),
                    })
                    .collect(),
                None => o,
            };
            Ok(Contest {
                contest_id: id.to_string(),
                office: office as i32,
                title: title.to_string(),
                scope: Some(scope),
                options,
                runoff: office.is_single_seat(),
                registration_key: reg_key(id)?,
                schedule: Some(Schedule {
                    registration_opens: s.0,
                    voting_opens: s.1,
                    voting_closes: s.2,
                }),
                ..Default::default()
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
            format_version: teko_election::FORMAT_VERSION,
            election_id: election_id.map_or_else(
                || format!("eleicoes-exemplo-{y:04}{m:02}{d:02}"),
                str::to_string,
            ),
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

    /// The public key (SPKI) of a contest's registration authority; the key is created on first use.
    fn registration_spki(&self, id: &str) -> Result<Vec<u8>, IssuerError> {
        let key = match self.registration_key(id) {
            Ok(k) => k,
            Err(_) => {
                let k =
                    IssuerKey::generate(2048).map_err(|e| IssuerError::Corrupt(e.to_string()))?;
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
    }

    /// SIMULATED governance of a SECOND ROUND (ADR-0040): adds the contest `<parent>-2t` between the
    /// two finalists to the election, as a manifest REVISION signed M-of-N (revisions only ADD
    /// contests), with its own registration key and the transitions up to `VotingOpen`. Returns
    /// the new contest id. The caller publishes the bundle on the board.
    pub fn dev_add_runoff(
        &self,
        now: u64,
        parent_id: &str,
        finalists: &[DevCandidate; 2],
    ) -> Result<String, IssuerError> {
        const DAY: u64 = 86_400;
        let corrupt = || IssuerError::Corrupt("election.bundle".into());
        let path = self.dir.join("election.bundle");
        let mut bundle = ElectionBundle::decode(fs::read(&path).map_err(|_| corrupt())?.as_slice())
            .map_err(|_| corrupt())?;
        let mut manifest =
            ElectionManifest::decode(bundle.manifest.as_slice()).map_err(|_| corrupt())?;
        let parent = manifest
            .contest(parent_id)
            .cloned()
            .ok_or_else(|| IssuerError::InvalidRequest("pleito desconhecido".into()))?;
        let id = format!("{parent_id}-2t");
        if manifest.contest(&id).is_some() {
            return Err(IssuerError::InvalidRequest(
                "o 2º turno deste pleito já existe".into(),
            ));
        }
        let gov: Vec<SigningKey> = (0..GOV_SIGNERS)
            .map(|i| load_key(&self.dir.join(format!("gov-{i}.key"))))
            .collect::<Result<_, _>>()?;
        let second = Contest {
            contest_id: id.clone(),
            office: parent.office,
            title: format!("{} (2º turno)", parent.title),
            scope: parent.scope.clone(),
            options: finalists
                .iter()
                .enumerate()
                .map(|(i, c)| ElectionOption {
                    index: i as u32,
                    name: c.nome.clone(),
                    number: c.numero.clone(),
                    credential_id: c.credential_id.clone(),
                })
                .collect(),
            registration_key: self.registration_spki(&id)?,
            schedule: Some(Schedule {
                registration_opens: now.saturating_sub(DAY),
                voting_opens: now.saturating_sub(1),
                voting_closes: now + 3 * DAY,
            }),
            runoff: false,
            parent: parent_id.to_string(),
        };
        // the old transitions are bound to the OLD contests' hashes, which do not change
        let mut transitions = bundle
            .transitions
            .iter()
            .map(|t| Transition::decode(t.as_slice()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| corrupt())?;
        let policy = teko_election::GovernancePolicy::new(
            self.trust()?.gov_signers.clone(),
            self.trust()?.gov_threshold,
        )
        .map_err(|_| IssuerError::Corrupt("trust.txt (governança)".into()))?;
        let status = replay(&policy, &manifest, &transitions).map_err(|_| corrupt())?;
        manifest.revision += 1;
        manifest.contests.push(second.clone());
        manifest.signatures = [0usize, 1]
            .iter()
            .map(|i| sign_election_manifest(&gov[*i], &manifest))
            .collect();
        let s = second.schedule.clone().unwrap_or_default();
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
        for (seq, (from, to, at)) in (status.seq + 1..).zip(steps) {
            let mut t = Transition {
                election_id: manifest.election_id.clone(),
                contest_id: id.clone(),
                contest_hash: contest_hash(&second).to_vec(),
                from_phase: from as i32,
                to_phase: to as i32,
                seq,
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
        // the whole history must still replay with the revision
        replay(&policy, &manifest, &transitions)
            .map_err(|_| IssuerError::Corrupt("o 2º turno foi recusado pela governança".into()))?;
        bundle.manifest = manifest.encode_to_vec();
        bundle.transitions = transitions.iter().map(|t| t.encode_to_vec()).collect();
        fs::write(&path, bundle.encode_to_vec())?;
        Ok(id)
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

    /// Anonymous registration (ADR-0008). Checks, in order: the civic credential is the organ's
    /// (a bundle the device trusts: M distinct members signed it), the chain says it is ACTIVE
    /// (past the contest window, not vetoed, revoked or expired) with the same commitment and
    /// holder, the identity key matches and proves possession of THIS request, the contest requires
    /// registration, the domicile covers it, the contest is in `RegistrationOpen`/`VotingOpen`,
    /// and the citizen has not registered here already (a RETRY of the same blinded message is
    /// re-signed). Then blind-signs. Returns the blind signature.
    ///
    /// `chain` looks up a credential id on the (verified) economy chain.
    pub fn sign_registration(
        &self,
        election_id: &str,
        contest_id: &str,
        request: &RegistrationRequest,
        chain: &dyn Fn(&[u8]) -> Option<CivicRecord>,
    ) -> Result<Vec<u8>, IssuerError> {
        let no = |m: &str| IssuerError::RegistrationRefused(m.to_string());
        let trust = self.trust()?;
        let civic = CivicBundle::decode(request.credential.as_slice())
            .map_err(|_| no("credencial malformada"))?;
        let policy = CivicPolicy {
            chain_id: trust.civic_chain_id.clone(),
            threshold: trust.civic_threshold,
            members: trust.civic_members.clone(),
        };
        let credential_id = verify_civic_bundle(&civic, &policy, &request.pk_identity)
            .map_err(|e| no(&format!("credencial recusada: {e}")))?;
        let salt: [u8; 16] = civic
            .salt
            .as_slice()
            .try_into()
            .map_err(|_| no("sal inválido"))?;
        let claims = claim_list(&salt, &civic.claims);
        let record = chain(&credential_id).ok_or_else(|| no("a credencial não existe na chain"))?;
        if !record.active {
            return Err(no(
                "a credencial não está ativa na chain (em contestação, vetada, revogada ou expirada): peça a reemissão ao órgão",
            ));
        }
        if record.subject != request.pk_identity
            || record.commitment.as_slice() != commitment_of(&claims).as_slice()
            || record.valid_until != civic.valid_until
        {
            return Err(no("a credencial na chain não confere com o pacote"));
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
            &claim_value(&claims, "electoral_state")
                .ok_or_else(|| no("sem domicílio eleitoral"))?,
            &claim_value(&claims, "electoral_municipality")
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
        // The citizen is the organ's REGISTRY ID (the same across reissues), not the device key.
        let who = claim_value(&claims, "registry_id")
            .ok_or_else(|| no("a credencial não tem registry_id"))?;
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
}

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

fn claim_value(claims: &[Claim], name: &str) -> Option<String> {
    claims
        .iter()
        .find(|c| c.name == name)
        .map(|c| c.value.clone())
}

impl Issuer {
    /// TEST: forgets the anonymous registrations (the once-per-citizen ledger).
    pub fn dev_forget_registrations(&self) {
        let _ = fs::remove_file(self.dir.join("registrations.log"));
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

    #[test]
    fn candidates_from_the_chain_replace_the_example_list_and_bind_the_credential() {
        let issuer = Issuer::open(scratch_dir("cands"));
        issuer.dev_setup().unwrap();
        let mut cands = DevCandidates::new();
        cands.insert(
            "governador-sp".into(),
            vec![
                DevCandidate {
                    credential_id: vec![1; 32],
                    numero: "13".into(),
                    nome: "Ana Souza".into(),
                },
                DevCandidate {
                    credential_id: vec![2; 32],
                    numero: "22".into(),
                    nome: "Beto Lima".into(),
                },
            ],
        );
        issuer
            .dev_election_with(1_900_000_000, Some("geral-2026"), &cands)
            .unwrap();
        let bytes = fs::read(issuer.dir().join("election.bundle")).unwrap();
        let b = ElectionBundle::decode(bytes.as_slice()).unwrap();
        let m = ElectionManifest::decode(b.manifest.as_slice()).unwrap();
        assert_eq!(m.election_id, "geral-2026");
        let sp = m.contest("governador-sp").unwrap();
        assert_eq!(sp.options.len(), 2);
        assert_eq!(sp.options[1].name, "Beto Lima");
        assert_eq!(sp.options[1].number, "22");
        assert_eq!(sp.options[1].credential_id, vec![2; 32]);
        assert!(sp.runoff, "a governor admits a second round");
        // a contest with no candidates from the chain keeps the example list (no credential)
        let rj = m.contest("governador-rj").unwrap();
        assert!(rj.options.iter().all(|o| o.credential_id.is_empty()));
        // the plan names every contest with the same calendar
        let plan = dev_contest_plan(1_900_000_000);
        assert_eq!(plan.len(), m.contests.len());
        for p in &plan {
            let c = m.contest(p.id).unwrap();
            assert_eq!(c.schedule.as_ref().unwrap().voting_closes, p.voting_closes);
        }
        let _ = fs::remove_dir_all(issuer.dir());
    }

    #[test]
    fn a_second_round_is_a_signed_revision_that_adds_a_contest_with_the_two_finalists() {
        let issuer = Issuer::open(scratch_dir("runoff"));
        let trust = issuer.dev_setup().unwrap();
        let now = 1_900_000_000;
        issuer.dev_election(now).unwrap();
        let f = |n: u8, num: &str, nome: &str| DevCandidate {
            credential_id: vec![n; 32],
            numero: num.into(),
            nome: nome.into(),
        };
        let id = issuer
            .dev_add_runoff(
                now,
                "governador-sp",
                &[f(1, "13", "Ana"), f(2, "22", "Beto")],
            )
            .unwrap();
        assert_eq!(id, "governador-sp-2t");
        // asking twice, or for an unknown contest, is refused
        assert!(
            issuer
                .dev_add_runoff(
                    now,
                    "governador-sp",
                    &[f(1, "13", "Ana"), f(2, "22", "Beto")]
                )
                .is_err()
        );
        assert!(
            issuer
                .dev_add_runoff(now, "nao-existe", &[f(1, "13", "Ana"), f(2, "22", "Beto")])
                .is_err()
        );
        let b = ElectionBundle::decode(
            fs::read(issuer.dir().join("election.bundle"))
                .unwrap()
                .as_slice(),
        )
        .unwrap();
        let m = ElectionManifest::decode(b.manifest.as_slice()).unwrap();
        assert_eq!(m.revision, 2);
        let policy =
            teko_election::GovernancePolicy::new(trust.gov_signers.clone(), trust.gov_threshold)
                .unwrap();
        teko_election::verify_manifest(&policy, &m).unwrap();
        let second = m.contest(&id).unwrap();
        assert_eq!(second.parent, "governador-sp");
        assert_eq!(second.options.len(), 2);
        assert!(!second.runoff);
        // the whole history replays and the second round is OPEN for voting
        let transitions: Vec<Transition> = b
            .transitions
            .iter()
            .map(|t| Transition::decode(t.as_slice()).unwrap())
            .collect();
        let status = replay(&policy, &m, &transitions).unwrap();
        assert_eq!(status.phase_of(&id), Some(Phase::VotingOpen));
        assert_eq!(status.phase_of("governador-sp"), Some(Phase::VotingOpen));
        let _ = fs::remove_dir_all(issuer.dir());
    }
}
