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
use teko_attestation::{
    FirmwareManifest, TrustPolicy, blob_hash, count_valid_signers, manifest_fingerprint,
    verify_attestation, verify_blob, verify_manifest,
};
use teko_crypto::credential::{self, Presentation};
use teko_crypto::{civic_id, hex, ledger_address, parse_civic_id, parse_wallet_address};
use teko_election::{
    BallotBody, ElectionBundle, ElectionManifest, GovernancePolicy, Transition, contest_hash,
    replay, verify_ballot,
};
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
    /// Manifest of the election last loaded (to verify ballots against).
    election: Option<ElectionManifest>,
    /// Latest verified revocation list from the authority (None = none loaded).
    revocations: Option<credential::RevocationList>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceStatus {
    pub fw_version: u32,
    pub state: String,
    pub locked: bool,
    pub retries_remaining: u32,
    pub label: String,
    pub has_credential: bool,
    /// A wallet exists (created with a confirmed backup, or restored).
    pub wallet_ready: bool,
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

/// Public result of creating/restoring a wallet. The phrase NEVER reaches the host.
#[derive(Debug, Clone, Serialize)]
pub struct WalletInfo {
    pub address: String,
    pub has_passphrase: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AddressInfo {
    pub address: String,
    pub public_key: String,
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
            election: None,
            revocations: None,
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

    // ───────────────────────── wallet (ADR-0023) ─────────────────────────

    fn check_address(address: &str) -> Result<(), ClientError> {
        parse_wallet_address(address)
            .map(|_| ())
            .map_err(|_| ClientError::Verification("endereço da carteira inválido".into()))
    }

    /// The device creates the wallet: it shows the 24 words ON ITS SCREEN and asks the
    /// user for word positions. This call only waits for the outcome.
    pub fn wallet_setup(&mut self) -> Result<WalletInfo, ClientError> {
        let w = self.expect(Request::SetupWallet(SetupWallet {}), |r| match r {
            Response::WalletReady(w) => Ok(w),
            o => Err(o),
        })?;
        Self::check_address(&w.address)?;
        Ok(WalletInfo {
            address: w.address,
            has_passphrase: w.has_passphrase,
        })
    }

    /// The user types the 24 words ON THE DEVICE.
    pub fn wallet_restore(&mut self) -> Result<WalletInfo, ClientError> {
        let w = self.expect(Request::RestoreWallet(RestoreWallet {}), |r| match r {
            Response::WalletReady(w) => Ok(w),
            o => Err(o),
        })?;
        Self::check_address(&w.address)?;
        Ok(WalletInfo {
            address: w.address,
            has_passphrase: w.has_passphrase,
        })
    }

    /// The user retypes the phrase on the device; Ok = it matches this wallet.
    pub fn wallet_check_backup(&mut self) -> Result<(), ClientError> {
        self.expect(
            Request::CheckWalletBackup(CheckWalletBackup {}),
            |r| match r {
                Response::WalletBackupOk(_) => Ok(()),
                o => Err(o),
            },
        )
    }

    /// Address and public key, cross-checked locally (address = hash of the key).
    pub fn wallet_address(&mut self) -> Result<AddressInfo, ClientError> {
        let a = self.expect(Request::GetAddress(GetAddress {}), |r| match r {
            Response::Address(a) => Ok(a),
            o => Err(o),
        })?;
        let raw = parse_wallet_address(&a.address)
            .map_err(|_| ClientError::Verification("endereço da carteira inválido".into()))?;
        if raw != ledger_address(&a.public_key) {
            return Err(ClientError::Verification(
                "o endereço não corresponde à chave pública".into(),
            ));
        }
        Ok(AddressInfo {
            address: a.address,
            public_key: hex::encode(&a.public_key),
        })
    }

    /// Asks the device to sign a coin transfer. The DEVICE builds and shows the
    /// transaction and signs only after the physical confirmation; the client
    /// assembles the ledger transaction and checks the signature before returning it.
    pub fn wallet_sign_transfer(
        &mut self,
        chain_id: &str,
        to_address: &str,
        amount: u64,
        fee: u64,
        nonce: u64,
    ) -> Result<teko_ledger::Tx, ClientError> {
        let to = parse_wallet_address(to_address)
            .map_err(|_| ClientError::Verification("endereço de destino inválido".into()))?;
        let signed = self.expect(
            Request::SignTransfer(SignTransfer {
                chain_id: chain_id.to_string(),
                to_address: to_address.to_string(),
                amount,
                fee,
                nonce,
            }),
            |r| match r {
                Response::TransferSigned(t) => Ok(t),
                o => Err(o),
            },
        )?;
        let tx = teko_ledger::Tx {
            chain_id: chain_id.to_string(),
            nonce,
            module: teko_coin::COIN_MODULE,
            payload: teko_coin::Op::Transfer { to, amount }.encode(),
            sender: signed.public_key,
            signature: signed.signature,
            fee,
        };
        tx.verify().map_err(|_| {
            ClientError::Verification("a assinatura devolvida não confere com a transação".into())
        })?;
        Ok(tx)
    }

    /// Asks the device for this wallet's CONSENT (as seller or buyer) to move an asset.
    /// The device shows the asset, parties, price and version and signs only after the
    /// physical confirmation. `kind`: 2 = transfer, 3 = swap. The client checks the
    /// signature before returning it.
    #[allow(clippy::too_many_arguments)]
    pub fn wallet_sign_asset_consent(
        &mut self,
        chain_id: &str,
        kind: u8,
        class: &str,
        asset: &str,
        from_address: &str,
        to_address: &str,
        price: u64,
        version: u64,
    ) -> Result<teko_assets::Approval, ClientError> {
        let from = parse_wallet_address(from_address)
            .map_err(|_| ClientError::Verification("endereço do vendedor inválido".into()))?;
        let to = parse_wallet_address(to_address)
            .map_err(|_| ClientError::Verification("endereço do comprador inválido".into()))?;
        let signed = self.expect(
            Request::SignAssetConsent(SignAssetConsent {
                chain_id: chain_id.to_string(),
                kind: u32::from(kind),
                class: class.to_string(),
                asset: asset.to_string(),
                from_address: from_address.to_string(),
                to_address: to_address.to_string(),
                price,
                version,
            }),
            |r| match r {
                Response::AssetConsentSigned(a) => Ok(a),
                o => Err(o),
            },
        )?;
        let h = teko_crypto::ledger::asset_intent_hash(
            chain_id,
            kind,
            class.as_bytes(),
            asset.as_bytes(),
            &from,
            &to,
            price,
            version,
            &[],
        );
        teko_crypto::verify_tagged(
            &signed.public_key,
            teko_crypto::tags::ASSET_APPROVAL,
            &h,
            &signed.signature,
        )
        .map_err(|_| {
            ClientError::Verification("o consentimento devolvido não confere com a operação".into())
        })?;
        Ok(teko_assets::Approval {
            key: signed.public_key,
            signature: signed.signature,
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
        // A REVOKED credential never verifies, even if its signature is perfect (a lost
        // device cannot be told: the check lives here, on the verifier).
        if let (Some(list), Some(c)) = (&self.revocations, &pres.credential)
            && list.is_revoked(&c.credential_id)
        {
            return Err(ClientError::Verification(
                "credencial REVOGADA pela autoridade (dispositivo perdido, roubado ou substituído)"
                    .into(),
            ));
        }
        Ok(AttributeResult {
            verified: true,
            issuer: pres.credential.map(|c| c.issuer).unwrap_or_default(),
            disclosed,
            nonce: hex::encode(&nonce),
        })
    }

    /// Loads the authority's signed revocation list. A list that does not verify,
    /// or is OLDER than the one already held (a rollback), is refused.
    pub fn use_revocations(
        &mut self,
        policy: &TrustPolicy,
        list: credential::RevocationList,
    ) -> Result<(), ClientError> {
        credential::verify_revocation_list(&policy.authority_root, &list)
            .map_err(|_| ClientError::Verification("lista de revogação inválida".into()))?;
        if self.revocations.as_ref().is_some_and(|l| l.seq > list.seq) {
            return Err(ClientError::Verification(
                "lista de revogação mais antiga que a já conhecida".into(),
            ));
        }
        self.revocations = Some(list);
        Ok(())
    }

    // ───────────────────────── elections (Phase 2) ─────────────────────────

    /// Verifies a bundle on the HOST only (M-of-N + history replay) and returns
    /// its manifest and transitions. Nothing is sent to the device.
    fn check_bundle(
        policy: &TrustPolicy,
        bundle_bytes: &[u8],
    ) -> Result<(ElectionManifest, ElectionBundle), ClientError> {
        let bundle = ElectionBundle::decode(bundle_bytes)
            .map_err(|_| ClientError::Verification("pacote de eleição malformado".into()))?;
        let manifest = ElectionManifest::decode(bundle.manifest.as_slice())
            .map_err(|_| ClientError::Verification("manifesto malformado".into()))?;
        let mut transitions = Vec::new();
        for t in &bundle.transitions {
            transitions.push(
                Transition::decode(t.as_slice())
                    .map_err(|_| ClientError::Verification("transição malformada".into()))?,
            );
        }
        let gov = GovernancePolicy::new(policy.gov_signers.clone(), policy.gov_threshold).map_err(
            |_| ClientError::Verification("sem política de governança no trust.txt".into()),
        )?;
        replay(&gov, &manifest, &transitions)
            .map_err(|e| ClientError::Verification(format!("eleição recusada: {e}")))?;
        Ok((manifest, bundle))
    }

    /// Remembers an election (verified on the host) so ballots can be checked,
    /// without sending anything to the device. Used after a reconnection.
    pub fn use_election_bundle(
        &mut self,
        policy: &TrustPolicy,
        bundle_bytes: &[u8],
    ) -> Result<(), ClientError> {
        let (manifest, _) = Self::check_bundle(policy, bundle_bytes)?;
        self.election = Some(manifest);
        Ok(())
    }

    pub fn has_election(&self) -> bool {
        self.election.is_some()
    }

    /// Gives the device an election bundle (manifest + governance history).
    /// The Suite verifies M-of-N and replays the history FIRST (fail fast); the
    /// device then does the same checks itself and asks for confirmation.
    pub fn load_election(
        &mut self,
        policy: &TrustPolicy,
        bundle_bytes: &[u8],
    ) -> Result<ElectionSummary, ClientError> {
        let (manifest, bundle) = Self::check_bundle(policy, bundle_bytes)?;
        let loaded = self.expect(
            Request::LoadElection(LoadElection {
                manifest: bundle.manifest.clone(),
                transitions: bundle.transitions.clone(),
            }),
            |r| match r {
                Response::ElectionLoaded(l) => Ok(l),
                o => Err(o),
            },
        )?;
        self.election = Some(manifest);
        Ok(summarize(&loaded))
    }

    /// Reads the election the device already holds (no confirmation needed).
    pub fn election(&mut self) -> Result<ElectionSummary, ClientError> {
        let loaded = self.expect(Request::GetElection(GetElection {}), |r| match r {
            Response::ElectionLoaded(l) => Ok(l),
            o => Err(o),
        })?;
        Ok(summarize(&loaded))
    }

    /// The PUBLIC voter key of one contest (hex), e.g. to register it.
    pub fn voter_key(
        &mut self,
        election_id: &str,
        contest_id: &str,
    ) -> Result<String, ClientError> {
        self.expect(
            Request::GetVoterKey(GetVoterKey {
                election_id: election_id.to_string(),
                contest_id: contest_id.to_string(),
            }),
            |r| match r {
                Response::VoterKey(k) => Ok(hex::encode(&k.voter_key)),
                o => Err(o),
            },
        )
    }

    /// Step 1 of the anonymous registration: the DEVICE builds the blinded message
    /// (confirming on its screen). Hand the result to the registration authority.
    pub fn begin_registration(
        &mut self,
        election_id: &str,
        contest_id: &str,
    ) -> Result<teko_protocol::RegistrationRequest, ClientError> {
        self.expect(
            Request::BeginRegistration(BeginRegistration {
                election_id: election_id.to_string(),
                contest_id: contest_id.to_string(),
            }),
            |r| match r {
                Response::RegistrationRequest(q) => Ok(q),
                o => Err(o),
            },
        )
    }

    /// Step 2: the authority's blind signature in, the unlinkable token out. The
    /// token is CHECKED here against the contest's registration key too, so a bad
    /// token is never published (it would only be rejected by the tally).
    pub fn finish_registration(
        &mut self,
        election_id: &str,
        contest_id: &str,
        blind_signature: &[u8],
    ) -> Result<teko_protocol::RegistrationToken, ClientError> {
        let key = self
            .election
            .as_ref()
            .and_then(|m| m.contest(contest_id))
            .map(|c| c.registration_key.clone())
            .ok_or_else(|| ClientError::Unexpected("carregue a eleição primeiro".into()))?;
        let t = self.expect(
            Request::FinishRegistration(FinishRegistration {
                election_id: election_id.to_string(),
                contest_id: contest_id.to_string(),
                blind_signature: blind_signature.to_vec(),
            }),
            |r| match r {
                Response::RegistrationToken(t) => Ok(t),
                o => Err(o),
            },
        )?;
        let msg = teko_election::registration_message(election_id, contest_id, &t.voter_key);
        if !teko_election::RegistrationVerifier::verify(
            &teko_registration::RsaRegistration,
            &key,
            &msg,
            &t.token,
        ) {
            return Err(ClientError::Verification(
                "o token de registro NÃO confere".into(),
            ));
        }
        Ok(t)
    }

    /// Asks the device to sign a ballot. The device shows the candidate NAME
    /// from its own verified copy of the manifest; we only send the INDEX.
    /// The signature is verified here before being handed to the user.
    /// PROTOTYPE: nothing is submitted anywhere and there is no ZK proof yet.
    pub fn sign_ballot(
        &mut self,
        election_id: &str,
        contest_id: &str,
        option_index: u32,
    ) -> Result<SignedBallot, ClientError> {
        let manifest = self
            .election
            .clone()
            .ok_or_else(|| ClientError::Unexpected("carregue a eleição primeiro".into()))?;
        let contest = manifest
            .contest(contest_id)
            .filter(|_| manifest.election_id == election_id)
            .ok_or_else(|| ClientError::Verification("pleito desconhecido nesta eleição".into()))?;
        let option_name = contest
            .option_name(option_index)
            .ok_or_else(|| ClientError::Verification("opção inexistente".into()))?
            .to_string();
        let b = self.expect(
            Request::SignBallot(SignBallot {
                election_id: election_id.to_string(),
                contest_id: contest_id.to_string(),
                option_index,
            }),
            |r| match r {
                Response::BallotSignature(b) => Ok(b),
                o => Err(o),
            },
        )?;
        let encrypted_mode = !manifest.tally_key.is_empty();
        if encrypted_mode != !b.encrypted.is_empty() {
            return Err(ClientError::Verification(
                "o dispositivo devolveu uma cédula em modo diferente do da eleição".into(),
            ));
        }
        let body = BallotBody {
            election_id: election_id.to_string(),
            contest_id: contest_id.to_string(),
            contest_hash: contest_hash(contest).to_vec(),
            option_index: if encrypted_mode { 0 } else { option_index },
            ciphertext_hash: if encrypted_mode {
                teko_election::ciphertext_hash(&b.encrypted)
            } else {
                Vec::new()
            },
        };
        verify_ballot(&b.voter_key, &body, &b.signature)
            .map_err(|_| ClientError::Verification("a assinatura da cédula NÃO confere".into()))?;
        if encrypted_mode {
            // The proofs say "exactly one option, each entry 0 or 1, for THIS election,
            // contest and voter key": checked here too, so a bad ballot is never published.
            let pk = teko_ballot_crypto::point_from_bytes(&manifest.tally_key)
                .ok_or_else(|| ClientError::Verification("chave da eleição inválida".into()))?;
            let n = contest.options.len();
            let ctx = teko_election::ballot_ctx(election_id, contest_id, &b.voter_key);
            let ok = teko_ballot_crypto::EncryptedBallot::from_bytes(n, &b.encrypted)
                .is_some_and(|eb| teko_ballot_crypto::verify_ballot(&pk, n, &ctx, &eb));
            if !ok {
                return Err(ClientError::Verification(
                    "as provas da cédula cifrada NÃO conferem".into(),
                ));
            }
        }
        Ok(SignedBallot {
            election_id: election_id.to_string(),
            contest_id: contest_id.to_string(),
            contest_hash: hex::encode(&body.contest_hash),
            contest_title: contest.title.clone(),
            option_index,
            option_name,
            voter_key: hex::encode(&b.voter_key),
            signature: hex::encode(&b.signature),
            encrypted: hex::encode(&b.encrypted),
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ContestSummary {
    pub contest_id: String,
    pub title: String,
    pub phase: &'static str,
    pub phase_code: u32,
    pub scope: String,
    pub scope_code: String,
    pub option_names: Vec<String>,
    pub eligible: bool,
    pub voted: bool,
    pub voting_opens: u64,
    pub voting_closes: u64,
    pub registration_required: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ElectionSummary {
    pub election_id: String,
    pub title: String,
    pub revision: u32,
    pub seq: u64,
    pub contests: Vec<ContestSummary>,
}

/// A ballot signed by the device and verified by the Suite. PROTOTYPE: not submitted.
#[derive(Debug, Clone, Serialize)]
pub struct SignedBallot {
    pub election_id: String,
    pub contest_id: String,
    pub contest_hash: String,
    pub contest_title: String,
    pub option_index: u32,
    pub option_name: String,
    pub voter_key: String,
    pub signature: String,
    /// Encrypted ballot (hex), empty in plaintext mode.
    pub encrypted: String,
}

fn phase_name(code: u32) -> &'static str {
    match code {
        0 => "draft",
        1 => "candidates_published",
        2 => "registration_open",
        3 => "voting_open",
        4 => "voting_closed",
        5 => "tallied",
        6 => "audited",
        7 => "cancelled",
        _ => "unknown",
    }
}

fn summarize(l: &ElectionLoaded) -> ElectionSummary {
    ElectionSummary {
        election_id: l.election_id.clone(),
        title: l.title.clone(),
        revision: l.revision,
        seq: l.seq,
        contests: l
            .contests
            .iter()
            .map(|c| ContestSummary {
                contest_id: c.contest_id.clone(),
                title: c.title.clone(),
                phase: phase_name(c.phase),
                phase_code: c.phase,
                scope: match c.scope_level {
                    1 => "national",
                    2 => "state",
                    3 => "municipality",
                    _ => "unknown",
                }
                .to_string(),
                scope_code: c.scope_code.clone(),
                option_names: c.option_names.clone(),
                eligible: c.eligible,
                voted: c.voted,
                voting_opens: c.voting_opens,
                voting_closes: c.voting_closes,
                registration_required: c.registration_required,
            })
            .collect(),
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
        wallet_ready: f.wallet_ready,
        capabilities: f.capabilities.clone(),
    }
}
