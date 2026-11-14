//! Host-side client: everything the Suite does with the device, as plain Rust
//! (so it can be tested headless). The host is UNTRUSTED: it verifies what it
//! can, but the device remains the arbiter of anything that matters.

mod error;
mod paths;
mod signed_doc;

use std::time::{SystemTime, UNIX_EPOCH};

use prost::Message;
use rand::Rng;
use rand::rand_core::UnwrapErr;
use rand::rngs::SysRng;
use serde::Serialize;
use sha3::{Digest, Sha3_256};
use teko_attestation::{
    FirmwareManifest, TrustPolicy, blob_hash, count_valid_signers, manifest_fingerprint,
    verify_attestation, verify_blob, verify_manifest,
};
use teko_crypto::credential::{self, Presentation};
use teko_crypto::{civic_id, hex, parse_civic_id};
use teko_protocol::*;
use teko_transport::Transport;

pub use error::{ClientError, DeviceFailure};
pub use paths::{DEFAULT_ADDR, resolve_dev_dir};
pub use signed_doc::{SignedDocument, VerifiedDocument, file_hash, verify_signed_document};

type OnButton = Box<dyn FnMut(ButtonKind) + Send>;

pub struct DeviceClient<T: Transport> {
    transport: T,
    next_id: u32,
    on_button: Option<OnButton>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceStatus {
    pub fw_version: u32,
    pub state: String,
    pub locked: bool,
    pub retries_remaining: u32,
    pub label: String,
    pub has_credential: bool,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthenticityReport {
    pub genuine: bool,
    pub batch_id: Option<String>,
    pub fw_version: u32,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FirmwareInfo {
    pub version: u32,
    pub size: u64,
    pub fingerprint: String,
    pub valid_signers: usize,
    pub threshold: usize,
    pub total_signers: usize,
    pub installed_version: u32,
    pub is_downgrade: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CivicIdInfo {
    pub id: String,
    pub public_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum UnlockOutcome {
    Unlocked,
    WrongPin { remaining: u32 },
}

#[derive(Debug, Clone, Serialize)]
pub struct AttributeResult {
    pub verified: bool,
    pub issuer: String,
    pub disclosed: Vec<(String, String)>,
    pub nonce: String,
}

pub fn state_name(state: DeviceState) -> &'static str {
    match state {
        DeviceState::Virgin => "virgin",
        DeviceState::FirmwareInstalled => "firmware_installed",
        DeviceState::Initialized => "initialized",
        DeviceState::CredentialsIssued => "credentials_issued",
    }
}

fn unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Host-side inspection of a firmware offer BEFORE sending anything.
pub fn inspect_firmware(
    policy: &TrustPolicy,
    manifest_bytes: &[u8],
    blob: &[u8],
    installed_version: u32,
) -> Result<FirmwareInfo, ClientError> {
    let m = FirmwareManifest::decode(manifest_bytes)
        .map_err(|_| ClientError::Verification("manifesto malformado".into()))?;
    verify_blob(&m, &blob_hash(blob)).map_err(|_| {
        ClientError::Verification("o arquivo de firmware não corresponde ao manifesto".into())
    })?;
    Ok(FirmwareInfo {
        version: m.version,
        size: m.size,
        fingerprint: manifest_fingerprint(&m),
        valid_signers: count_valid_signers(policy, &m),
        threshold: policy.fw_threshold,
        total_signers: policy.fw_signers.len(),
        installed_version,
        is_downgrade: m.version < installed_version,
    })
}

impl<T: Transport> DeviceClient<T> {
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            next_id: 1,
            on_button: None,
        }
    }

    /// Called whenever the device says "waiting for you on the device".
    pub fn set_on_button(&mut self, f: impl FnMut(ButtonKind) + Send + 'static) {
        self.on_button = Some(Box::new(f));
    }

    /// Sends a request; handles interim `ButtonRequest`s; maps `Failure` to `Err`.
    pub fn request(&mut self, req: Request) -> Result<Response, ClientError> {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.transport.write_frame(&req.to_frame(id))?;
        loop {
            let frame = self.transport.read_frame()?;
            if frame.request_id != id {
                return Err(ClientError::Unexpected(
                    "resposta de outra requisição".into(),
                ));
            }
            match Response::from_frame(&frame)? {
                Response::ButtonRequest(b) => {
                    if let Some(cb) = self.on_button.as_mut() {
                        cb(ButtonKind::try_from(b.kind).unwrap_or(ButtonKind::Unspecified));
                    }
                }
                Response::Failure(f) => return Err(ClientError::Device(f.into())),
                other => return Ok(other),
            }
        }
    }

    fn expect<R>(
        &mut self,
        req: Request,
        pick: impl FnOnce(Response) -> Result<R, Response>,
    ) -> Result<R, ClientError> {
        let resp = self.request(req)?;
        pick(resp).map_err(|r| ClientError::Unexpected(format!("resposta inesperada: {r:?}")))
    }

    // ───────────────────────── status ─────────────────────────

    pub fn initialize(&mut self) -> Result<DeviceStatus, ClientError> {
        let f = self.expect(
            Request::Initialize(Initialize {
                host_proto_max: u32::from(PROTO_VERSION),
            }),
            |r| match r {
                Response::Features(f) => Ok(f),
                o => Err(o),
            },
        )?;
        Ok(status_of(&f))
    }

    pub fn features(&mut self) -> Result<DeviceStatus, ClientError> {
        let f = self.expect(Request::GetFeatures(GetFeatures {}), |r| match r {
            Response::Features(f) => Ok(f),
            o => Err(o),
        })?;
        Ok(status_of(&f))
    }

    // ───────────────────────── authenticity ─────────────────────────

    /// Challenge–response attestation up to the pinned authority root.
    /// A device that fails to attest is reported (not an `Err`): it is a result.
    pub fn verify_authenticity(
        &mut self,
        policy: &TrustPolicy,
    ) -> Result<AuthenticityReport, ClientError> {
        let mut nonce = [0u8; 32];
        UnwrapErr(SysRng).fill_bytes(&mut nonce);
        let a = match self.expect(
            Request::AttestationChallenge(AttestationChallenge {
                nonce: nonce.to_vec(),
            }),
            |r| match r {
                Response::AttestationResponse(a) => Ok(a),
                o => Err(o),
            },
        ) {
            Ok(a) => a,
            Err(ClientError::Device(f)) => {
                return Ok(AuthenticityReport {
                    genuine: false,
                    batch_id: None,
                    fw_version: 0,
                    reason: Some(format!("o dispositivo não soube atestar: {}", f.message)),
                });
            }
            Err(e) => return Err(e),
        };
        Ok(
            match verify_attestation(
                policy,
                &nonce,
                a.fw_version,
                a.state,
                &a.chain,
                &a.signature,
            ) {
                Ok(batch) => AuthenticityReport {
                    genuine: true,
                    batch_id: Some(batch.batch_id),
                    fw_version: a.fw_version,
                    reason: None,
                },
                Err(e) => AuthenticityReport {
                    genuine: false,
                    batch_id: None,
                    fw_version: a.fw_version,
                    reason: Some(e.to_string()),
                },
            },
        )
    }

    // ───────────────────────── firmware ─────────────────────────

    /// Verifies the offer here first (fail fast), then streams it. The device's
    /// bootloader re-verifies everything and asks for on-device confirmation.
    pub fn install_firmware(
        &mut self,
        policy: &TrustPolicy,
        manifest_bytes: &[u8],
        blob: &[u8],
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<u32, ClientError> {
        let installed = self.features()?.fw_version;
        let info = inspect_firmware(policy, manifest_bytes, blob, installed)?;
        if info.valid_signers < info.threshold {
            return Err(ClientError::Verification(format!(
                "firmware recusado: apenas {} de {} assinaturas exigidas",
                info.valid_signers, info.threshold
            )));
        }
        let m = FirmwareManifest::decode(manifest_bytes)
            .map_err(|_| ClientError::Verification("manifesto malformado".into()))?;
        verify_manifest(policy, &m, installed)
            .map_err(|e| ClientError::Verification(e.to_string()))?;

        self.expect(
            Request::FirmwareBegin(FirmwareBegin {
                manifest: manifest_bytes.to_vec(),
            }),
            |r| match r {
                Response::FirmwareStatus(s) => Ok(s),
                o => Err(o),
            },
        )?;
        let total = blob.len() as u64;
        let mut offset = 0u64;
        for chunk in blob.chunks(16 * 1024) {
            self.expect(
                Request::FirmwareChunk(FirmwareChunk {
                    offset,
                    data: chunk.to_vec(),
                }),
                |r| match r {
                    Response::FirmwareStatus(s) => Ok(s),
                    o => Err(o),
                },
            )?;
            offset += chunk.len() as u64;
            progress(offset, total);
        }
        let done = self.expect(Request::FirmwareFinish(FirmwareFinish {}), |r| match r {
            Response::FirmwareStatus(s) => Ok(s),
            o => Err(o),
        })?;
        Ok(done.installed_version)
    }

    // ───────────────────────── PIN ─────────────────────────

    /// The PIN is typed ON THE DEVICE. This call only waits for the result.
    pub fn setup_pin(&mut self) -> Result<(), ClientError> {
        self.expect(Request::SetupPin(SetupPin {}), |r| match r {
            Response::PinSet(_) => Ok(()),
            o => Err(o),
        })
    }

    pub fn unlock(&mut self) -> Result<UnlockOutcome, ClientError> {
        self.expect(Request::Unlock(Unlock {}), |r| match r {
            Response::Unlocked(_) => Ok(UnlockOutcome::Unlocked),
            Response::PinWrong(p) => Ok(UnlockOutcome::WrongPin {
                remaining: p.remaining,
            }),
            o => Err(o),
        })
    }

    pub fn wipe(&mut self) -> Result<(), ClientError> {
        self.expect(Request::Wipe(Wipe {}), |r| match r {
            Response::WipeDone(_) => Ok(()),
            o => Err(o),
        })
    }

    // ───────────────────────── identity & issuance ─────────────────────────

    /// Asks the device for its civic ID and double-checks it locally.
    pub fn civic_id(&mut self) -> Result<CivicIdInfo, ClientError> {
        let c = self.expect(Request::GetCivicId(GetCivicId {}), |r| match r {
            Response::CivicId(c) => Ok(c),
            o => Err(o),
        })?;
        let expected =
            civic_id(&c.pk_identity).map_err(|e| ClientError::Verification(e.to_string()))?;
        if expected != c.id || parse_civic_id(&c.id).is_err() {
            return Err(ClientError::Verification(
                "o ID Cívico informado não corresponde à chave pública".into(),
            ));
        }
        Ok(CivicIdInfo {
            id: c.id,
            public_key: hex::encode(&c.pk_identity),
        })
    }

    pub fn identity_request(
        &mut self,
        authority_nonce: &[u8],
        authority_name: &str,
    ) -> Result<IdentityRequest, ClientError> {
        self.expect(
            Request::GetIdentityRequest(GetIdentityRequest {
                authority_nonce: authority_nonce.to_vec(),
                authority_name: authority_name.to_string(),
            }),
            |r| match r {
                Response::IdentityRequest(i) => Ok(i),
                o => Err(o),
            },
        )
    }

    pub fn load_credential(&mut self, credential: &[u8]) -> Result<(), ClientError> {
        self.expect(
            Request::LoadCredential(LoadCredential {
                credential: credential.to_vec(),
            }),
            |r| match r {
                Response::CredentialLoaded(_) => Ok(()),
                o => Err(o),
            },
        )
    }

    pub fn pairwise_id(&mut self, service: &str) -> Result<String, ClientError> {
        self.expect(
            Request::GetPairwiseId(GetPairwiseId {
                service_id: service.to_string(),
            }),
            |r| match r {
                Response::PairwiseId(p) => Ok(p.id),
                o => Err(o),
            },
        )
    }

    // ───────────────────────── signing & proofs ─────────────────────────

    /// file → SHA3-256 → confirmation ON THE DEVICE → verifiable signature.
    pub fn sign_document(
        &mut self,
        file: &[u8],
        summary: &str,
    ) -> Result<SignedDocument, ClientError> {
        let hash = file_hash(file);
        let s = self.expect(
            Request::SignDocument(SignDocument {
                hash: hash.to_vec(),
                summary: summary.to_string(),
            }),
            |r| match r {
                Response::DocumentSignature(s) => Ok(s),
                o => Err(o),
            },
        )?;
        let doc = SignedDocument::new(&hash, summary, &s.pk_identity, &s.signature)?;
        // Never trust the device blindly: verify before handing it to the user.
        verify_signed_document(&doc, Some(file))?;
        Ok(doc)
    }

    /// Acts as the verifier: fresh nonce, then checks the presentation against
    /// the pinned authority root.
    pub fn prove_attribute(
        &mut self,
        policy: &TrustPolicy,
        predicate: Predicate,
        verifier_name: &str,
    ) -> Result<AttributeResult, ClientError> {
        let mut nonce = [0u8; 16];
        UnwrapErr(SysRng).fill_bytes(&mut nonce);
        let proof = self.expect(
            Request::ProveAttribute(ProveAttribute {
                predicate: Some(predicate),
                verifier_nonce: nonce.to_vec(),
                verifier_name: verifier_name.to_string(),
            }),
            |r| match r {
                Response::AttributeProof(p) => Ok(p),
                o => Err(o),
            },
        )?;
        let pres = Presentation::decode(proof.presentation.as_slice())
            .map_err(|_| ClientError::Verification("prova malformada".into()))?;
        let disclosed =
            credential::verify_presentation(&policy.authority_root, &pres, &nonce, unix_secs())
                .map_err(|e| ClientError::Verification(format!("prova inválida: {e}")))?;
        Ok(AttributeResult {
            verified: true,
            issuer: pres.credential.map(|c| c.issuer).unwrap_or_default(),
            disclosed,
            nonce: hex::encode(&nonce),
        })
    }

    // ───────────────────────── voting (stubs) ─────────────────────────

    pub fn voter_commitment(&mut self) -> Result<String, ClientError> {
        self.expect(
            Request::GetVoterCommitment(GetVoterCommitment {}),
            |r| match r {
                Response::VoterCommitment(c) => Ok(hex::encode(&c.commitment)),
                o => Err(o),
            },
        )
    }

    /// Wired to the device stub: the user confirms on the device and the device
    /// answers `NotImplemented` (voting arrives with the blockchain phase).
    pub fn sign_ballot(
        &mut self,
        election_id: &str,
        choice_summary: &str,
    ) -> Result<(), ClientError> {
        let mut h = Sha3_256::new();
        h.update(election_id.as_bytes());
        h.update(choice_summary.as_bytes());
        let ballot_hash: [u8; 32] = h.finalize().into();
        self.expect(
            Request::SignBallot(SignBallot {
                election_id: election_id.to_string(),
                ballot_hash: ballot_hash.to_vec(),
                summary: choice_summary.to_string(),
            }),
            |r| match r {
                Response::BallotSignature(_) => Ok(()),
                o => Err(o),
            },
        )
    }
}

fn status_of(f: &Features) -> DeviceStatus {
    DeviceStatus {
        fw_version: f.fw_version,
        state: state_name(f.state()).to_string(),
        locked: f.locked,
        retries_remaining: f.retries_remaining,
        label: f.label.clone(),
        has_credential: f.has_credential,
        capabilities: f.capabilities.clone(),
    }
}
