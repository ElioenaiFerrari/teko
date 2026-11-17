//! Host-side client: everything the Suite does with the device, as plain Rust
//! (so it can be tested headless). The host is UNTRUSTED: it verifies what it
//! can, but the device remains the arbiter of anything that matters.

mod error;
mod paths;
mod signed_doc;

use prost::Message;
use rand::Rng;
use rand::rand_core::UnwrapErr;
use rand::rngs::SysRng;
use serde::Serialize;
use teko_attestation::{
    FirmwareManifest, TrustPolicy, blob_hash, count_valid_signers, issue_request_message,
    manifest_fingerprint, verify_attestation, verify_batch_chain, verify_blob, verify_manifest,
};
use teko_crypto::civic::{CIVIC_TYPE, ChainRecord, CivicPresentation, verify_civic_presentation};
use teko_crypto::{
    civic_id, hex, ledger_address, parse_civic_id, parse_wallet_address, tags, verify_tagged,
};
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
    /// The ciphertext the device committed to in the ballot being signed (ADR-0041).
    commitment: Option<BallotCommitment>,
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

pub use teko_crypto::civic::CivicRecord;

/// What an ORGAN checks before it issues the civic credential: the device proved it holds the
/// identity key (`ISSUE_POP`) and is a genuine device (batch attestation chain), for THIS nonce.
pub fn verify_identity_request(
    policy: &TrustPolicy,
    nonce: &[u8],
    request: &IdentityRequest,
) -> Result<(), ClientError> {
    let bad = |m: String| ClientError::Verification(m);
    let msg = issue_request_message(nonce, &request.pk_identity);
    verify_tagged(
        &request.pk_identity,
        tags::ISSUE_POP,
        &msg,
        &request.pop_signature,
    )
    .map_err(|_| bad("prova de posse da chave inválida".into()))?;
    let batch = verify_batch_chain(policy, &request.attestation_chain)
        .map_err(|e| bad(format!("atestação: {e}")))?;
    verify_tagged(
        &batch.public_key,
        tags::ISSUE_ATTEST,
        &msg,
        &request.attestation_signature,
    )
    .map_err(|_| bad("assinatura de atestação inválida".into()))?;
    Ok(())
}

pub fn state_name(state: DeviceState) -> &'static str {
    match state {
        DeviceState::Virgin => "virgin",
        DeviceState::FirmwareInstalled => "firmware_installed",
        DeviceState::Initialized => "initialized",
        DeviceState::CredentialsIssued => "credentials_issued",
    }
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
            commitment: None,
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
                Response::BallotCommitment(c) => self.commitment = Some(c),
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

    /// Asks the device to sign a staking action: 1 = bond, 2 = unbond, 3 = claim. The
    /// device builds and shows the transaction; the client checks the signature.
    pub fn wallet_sign_stake(
        &mut self,
        chain_id: &str,
        action: u8,
        amount: u64,
        fee: u64,
        nonce: u64,
    ) -> Result<teko_ledger::Tx, ClientError> {
        let signed = self.expect(
            Request::SignStake(SignStake {
                chain_id: chain_id.to_string(),
                action: u32::from(action),
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
            payload: teko_crypto::ledger::coin_stake_payload(action, amount),
            sender: signed.public_key,
            signature: signed.signature,
            fee,
        };
        tx.verify().map_err(|_| {
            ClientError::Verification("a assinatura devolvida não confere com a transação".into())
        })?;
        Ok(tx)
    }

    /// The PUBLIC credential key of this holder for one type (ADR-0037), after the confirmation
    /// on the device. The holder shows it to the issuing organ.
    pub fn credential_key(&mut self, ctype: &str) -> Result<Vec<u8>, ClientError> {
        let k = self.expect(
            Request::GetCredentialKey(GetCredentialKey {
                ctype: ctype.to_string(),
            }),
            |r| match r {
                Response::CredentialKey(k) => Ok(k),
                o => Err(o),
            },
        )?;
        if k.public_key.len() != 33 && k.public_key.len() != 65 {
            return Err(ClientError::Verification(
                "chave de credencial inválida".into(),
            ));
        }
        Ok(k.public_key)
    }

    /// Asks the device to OPEN attributes sealed to one of this wallet's credential keys (ADR-0039)
    /// and returns the attributes and the salt, after the host checks them against `commitment`
    /// (the one on the chain), so a device or a chain that lies is caught here.
    pub fn open_credential_bundle(
        &mut self,
        ctype: &str,
        subject: &[u8],
        sealed: &[u8],
        commitment: &[u8],
    ) -> Result<(Vec<u8>, [u8; 16]), ClientError> {
        let r = self.expect(
            Request::OpenCredentialBundle(OpenCredentialBundle {
                ctype: ctype.to_string(),
                subject: subject.to_vec(),
                sealed: sealed.to_vec(),
            }),
            |r| match r {
                Response::CredentialBundleOpened(o) => Ok(o),
                o => Err(o),
            },
        )?;
        let salt: [u8; 16] = r
            .salt
            .as_slice()
            .try_into()
            .map_err(|_| ClientError::Verification("sal inválido".into()))?;
        let got = teko_crypto::ledger::cred_private_commitment(&salt, &r.claims);
        if got.as_slice() != commitment {
            return Err(ClientError::Verification(
                "os atributos abertos não conferem com o compromisso da chain".into(),
            ));
        }
        Ok((r.claims, salt))
    }

    /// The credential keys of several types, derived from the WALLET, with one confirmation on the
    /// device (ADR-0039). After a wallet restore the Suite searches the chain for what was issued to
    /// them, and the credentials come back with the wallet.
    pub fn credential_keys(
        &mut self,
        ctypes: &[String],
    ) -> Result<Vec<(String, Vec<u8>)>, ClientError> {
        let r = self.expect(
            Request::GetCredentialKeys(GetCredentialKeys {
                ctypes: ctypes.to_vec(),
            }),
            |r| match r {
                Response::CredentialKeys(k) => Ok(k),
                o => Err(o),
            },
        )?;
        if r.keys.len() != ctypes.len()
            || r.keys
                .iter()
                .any(|k| k.public_key.len() != 33 && k.public_key.len() != 65)
        {
            return Err(ClientError::Verification(
                "chaves de credencial inválidas".into(),
            ));
        }
        Ok(r.keys
            .into_iter()
            .map(|k| (k.ctype, k.public_key))
            .collect())
    }

    /// Asks the device to build, show and sign an organ member's OWN transaction in the
    /// credentials module (ADR-0037). The device recomputes the commitment and the credential id
    /// from the claims it shows; the client rebuilds the same transaction and checks the signature
    /// before returning it, so a device or host that changed anything is caught here.
    pub fn wallet_sign_credential_op(
        &mut self,
        a: &CredentialOpArgs<'_>,
    ) -> Result<teko_ledger::Tx, ClientError> {
        let signed = self.expect(
            Request::SignCredentialOp(SignCredentialOp {
                chain_id: a.chain_id.to_string(),
                op: a.kind as u32,
                ctype: a.ctype.to_string(),
                subject: a.subject.to_vec(),
                claims: a.claims.to_vec(),
                public: a.public,
                salt: a.salt.map(<[u8]>::to_vec).unwrap_or_default(),
                commitment: a.commitment.map(<[u8]>::to_vec).unwrap_or_default(),
                valid_until: a.valid_until,
                reason: a.reason.to_string(),
                contest_secs: a.contest_secs,
                fee: a.fee,
                nonce: a.nonce,
            }),
            |r| match r {
                Response::TransferSigned(t) => Ok(t),
                o => Err(o),
            },
        )?;
        let bad = |m: &str| ClientError::Verification(m.into());
        // the same commitment the device computes
        let commitment: [u8; 32] = if a.public {
            teko_crypto::ledger::cred_public_commitment(a.claims)
        } else if !a.claims.is_empty() {
            let salt: [u8; 16] = a
                .salt
                .and_then(|s| s.try_into().ok())
                .ok_or_else(|| bad("sal deve ter 16 bytes"))?;
            teko_crypto::ledger::cred_private_commitment(&salt, a.claims)
        } else {
            a.commitment
                .and_then(|c| c.try_into().ok())
                .ok_or_else(|| bad("compromisso deve ter 32 bytes"))?
        };
        let on_chain_claims: &[u8] = if a.public { a.claims } else { &[] };
        let op = match a.kind {
            CredentialOpKind::Propose => {
                teko_credentials::Op::Propose(teko_credentials::ProposeOp {
                    ctype: a.ctype.as_bytes().to_vec(),
                    subject: a.subject.to_vec(),
                    commitment: commitment.to_vec(),
                    claims: on_chain_claims.to_vec(),
                    valid_until: a.valid_until,
                    // what the DEVICE sealed to the holder: part of the signed payload
                    sealed: signed.sealed.clone(),
                })
            }
            kind => {
                let vote = teko_credentials::VoteOp {
                    ctype: a.ctype.as_bytes().to_vec(),
                    id: teko_credentials::credential_id(
                        a.ctype.as_bytes(),
                        a.subject,
                        &commitment,
                        on_chain_claims,
                        a.valid_until,
                    )
                    .to_vec(),
                    reason: a.reason.to_string(),
                };
                match kind {
                    CredentialOpKind::Approve => teko_credentials::Op::Approve(vote),
                    CredentialOpKind::Veto => teko_credentials::Op::Veto(vote),
                    _ => teko_credentials::Op::Revoke(vote),
                }
            }
        };
        let tx = teko_ledger::Tx {
            chain_id: a.chain_id.to_string(),
            nonce: a.nonce,
            module: teko_credentials::CREDENTIALS_MODULE,
            payload: op.encode(),
            sender: signed.public_key,
            signature: signed.signature,
            fee: a.fee,
        };
        tx.verify()
            .map_err(|_| bad("a assinatura devolvida não confere com a transação"))?;
        Ok(tx)
    }

    /// Asks the device to build, show and sign the OWNER's `Offer` of an asset to `buyer_address`
    /// (ADR-0036): the owner's OWN transaction, from the wallet. The device derives the whole
    /// transaction from these fields and signs after the physical confirmation; the client
    /// rebuilds the same transaction and checks the signature before returning it.
    #[allow(clippy::too_many_arguments)]
    pub fn wallet_sign_asset_offer(
        &mut self,
        chain_id: &str,
        class: &str,
        asset: &str,
        buyer_address: &str,
        price: u64,
        version: u64,
        metadata: &[u8],
        window_secs: u64,
        fee: u64,
        nonce: u64,
    ) -> Result<teko_ledger::Tx, ClientError> {
        let buyer = parse_wallet_address(buyer_address)
            .map_err(|_| ClientError::Verification("endereço do comprador inválido".into()))?;
        let signed = self.expect(
            Request::SignAssetOffer(SignAssetOffer {
                chain_id: chain_id.to_string(),
                class: class.to_string(),
                asset: asset.to_string(),
                buyer_address: buyer_address.to_string(),
                price,
                version,
                metadata: metadata.to_vec(),
                window_secs,
                fee,
                nonce,
            }),
            |r| match r {
                Response::TransferSigned(t) => Ok(t),
                o => Err(o),
            },
        )?;
        let op = teko_assets::Op::Offer(teko_assets::OfferOp {
            class: class.as_bytes().to_vec(),
            asset: asset.as_bytes().to_vec(),
            buyer: buyer.to_vec(),
            price,
            version,
            metadata_hash: teko_crypto::asset_meta::hash(class, metadata).to_vec(),
        });
        asset_tx(chain_id, nonce, fee, op, signed)
    }

    /// Asks the device to build, show and sign the BUYER's `Accept` of an offer, on exactly the
    /// terms shown (price, version, metadata): the price goes to the escrow (ADR-0036).
    #[allow(clippy::too_many_arguments)]
    pub fn wallet_sign_asset_accept(
        &mut self,
        chain_id: &str,
        class: &str,
        asset: &str,
        seller_address: &str,
        price: u64,
        version: u64,
        metadata: &[u8],
        window_secs: u64,
        fee: u64,
        nonce: u64,
    ) -> Result<teko_ledger::Tx, ClientError> {
        parse_wallet_address(seller_address)
            .map_err(|_| ClientError::Verification("endereço do vendedor inválido".into()))?;
        let signed = self.expect(
            Request::SignAssetAccept(SignAssetAccept {
                chain_id: chain_id.to_string(),
                class: class.to_string(),
                asset: asset.to_string(),
                seller_address: seller_address.to_string(),
                price,
                version,
                metadata: metadata.to_vec(),
                window_secs,
                fee,
                nonce,
            }),
            |r| match r {
                Response::TransferSigned(t) => Ok(t),
                o => Err(o),
            },
        )?;
        let op = teko_assets::Op::Accept(teko_assets::AcceptOp {
            class: class.as_bytes().to_vec(),
            asset: asset.as_bytes().to_vec(),
            price,
            version,
            metadata_hash: teko_crypto::asset_meta::hash(class, metadata).to_vec(),
        });
        asset_tx(chain_id, nonce, fee, op, signed)
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

    /// Acts as the verifier: fresh nonce, then checks the presentation against what the CHAIN says
    /// about the credential (`chain` looks up its id): the commitment rebuilt from the disclosed
    /// claims and the hidden digests, the holder's signature, and that the credential is active
    /// (not vetoed, revoked or expired). A lost device is handled by the organ revoking on the chain.
    pub fn prove_attribute(
        &mut self,
        predicate: Predicate,
        verifier_name: &str,
        chain: &dyn Fn(&[u8]) -> Option<CivicRecord>,
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
        let pres = CivicPresentation::decode(proof.presentation.as_slice())
            .map_err(|_| ClientError::Verification("prova malformada".into()))?;
        if pres.ctype != CIVIC_TYPE {
            return Err(ClientError::Verification(
                "não é uma credencial cívica".into(),
            ));
        }
        let rec = chain(&pres.credential_id)
            .ok_or_else(|| ClientError::Verification("a credencial não existe na chain".into()))?;
        if !rec.active {
            return Err(ClientError::Verification(
                "credencial NÃO está ativa na chain (em contestação, vetada, revogada ou expirada)"
                    .into(),
            ));
        }
        verify_civic_presentation(
            &pres,
            &ChainRecord {
                subject: &rec.subject,
                commitment: &rec.commitment,
                valid_until: rec.valid_until,
                active: rec.active,
            },
            &nonce,
            verifier_name,
        )
        .map_err(|e| ClientError::Verification(format!("prova inválida: {e}")))?;
        Ok(AttributeResult {
            verified: true,
            issuer: format!("órgão emissor em {}", pres.chain_id),
            disclosed: pres
                .disclosed
                .iter()
                .map(|c| (c.name.clone(), c.value.clone()))
                .collect(),
            nonce: hex::encode(&nonce),
        })
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
    /// Asks the DEVICE to check, alone, that the ballot it cast is in a block certified by more than
    /// 2/3 of the institutions it trusts (ADR-0041). The host only brings the evidence (block,
    /// certificate, Merkle proof); nothing is taken on its word.
    pub fn check_my_ballot(
        &mut self,
        election_id: &str,
        contest_id: &str,
        evidence: BallotEvidence,
    ) -> Result<RecordedReport, ClientError> {
        let BallotEvidence {
            block,
            commit,
            index,
            proof,
            rotations,
        } = evidence;
        let r = self.expect(
            Request::CheckMyBallot(CheckMyBallot {
                election_id: election_id.to_string(),
                contest_id: contest_id.to_string(),
                block,
                commit,
                index,
                proof: proof.iter().map(|h| h.to_vec()).collect(),
                rotations,
            }),
            |r| match r {
                Response::BallotRecorded(r) => Ok(r),
                o => Err(o),
            },
        )?;
        Ok(RecordedReport {
            height: r.height,
            signers: r.signers,
            institutions: r.institutions,
            tracker: r.tracker,
            epoch: r.epoch,
        })
    }

    /// Like `sign_ballot_or_audit`, for callers that only handle a cast ballot: an AUDITED ballot
    /// comes back as an error carrying the audit result (the vote was NOT used).
    pub fn sign_ballot(
        &mut self,
        election_id: &str,
        contest_id: &str,
        option_index: u32,
    ) -> Result<SignedBallot, ClientError> {
        match self.sign_ballot_or_audit(election_id, contest_id, option_index)? {
            BallotOutcome::Cast(b) => Ok(b),
            BallotOutcome::Audited(a) => Err(ClientError::Unexpected(format!(
                "cédula de teste auditada ({}): o voto NÃO foi usado",
                a.summary
            ))),
        }
    }

    /// Asks the device to sign a ballot. In an encrypted election the device first COMMITS to the
    /// ciphertext and the voter then chooses, ON THE DEVICE, to cast it or to audit it (ADR-0041).
    /// An audited ballot is opened here with the randomness the device reveals: it must hold the
    /// option that was asked for, or the device lied and the call fails loudly.
    pub fn sign_ballot_or_audit(
        &mut self,
        election_id: &str,
        contest_id: &str,
        option_index: u32,
    ) -> Result<BallotOutcome, ClientError> {
        self.sign_ballot_or_audit_multi(election_id, contest_id, &[option_index])
    }

    /// Like `sign_ballot`, for a contest with several seats (ADR-0044): up to `seats` DISTINCT options.
    pub fn sign_ballot_multi(
        &mut self,
        election_id: &str,
        contest_id: &str,
        option_indices: &[u32],
    ) -> Result<SignedBallot, ClientError> {
        match self.sign_ballot_or_audit_multi(election_id, contest_id, option_indices)? {
            BallotOutcome::Cast(b) => Ok(b),
            BallotOutcome::Audited(a) => Err(ClientError::Unexpected(format!(
                "cédula de teste auditada ({}): o voto NÃO foi usado",
                a.summary
            ))),
        }
    }

    /// `sign_ballot_or_audit` for one or several options. An audited ballot must open to EXACTLY the
    /// set of options that was asked for.
    pub fn sign_ballot_or_audit_multi(
        &mut self,
        election_id: &str,
        contest_id: &str,
        option_indices: &[u32],
    ) -> Result<BallotOutcome, ClientError> {
        self.commitment = None;
        let manifest = self
            .election
            .clone()
            .ok_or_else(|| ClientError::Unexpected("carregue a eleição primeiro".into()))?;
        let contest = manifest
            .contest(contest_id)
            .filter(|_| manifest.election_id == election_id)
            .ok_or_else(|| ClientError::Verification("pleito desconhecido nesta eleição".into()))?;
        if option_indices.is_empty() {
            return Err(ClientError::Verification("nenhuma opção escolhida".into()));
        }
        let mut names = Vec::new();
        for i in option_indices {
            names.push(
                contest
                    .option_name(*i)
                    .ok_or_else(|| ClientError::Verification("opção inexistente".into()))?
                    .to_string(),
            );
        }
        let option_name = names.join(", ");
        let option_index = option_indices.first().copied().unwrap_or(0);
        let req = if option_indices.len() == 1 {
            // one choice keeps the original message (an old host and a one-seat contest)
            SignBallot {
                election_id: election_id.to_string(),
                contest_id: contest_id.to_string(),
                option_index,
                ..Default::default()
            }
        } else {
            SignBallot {
                election_id: election_id.to_string(),
                contest_id: contest_id.to_string(),
                option_index: 0,
                option_indices: option_indices.to_vec(),
            }
        };
        let resp = self.request(Request::SignBallot(req))?;
        let encrypted_mode = !manifest.tally_key.is_empty();
        let b = match resp {
            Response::BallotSignature(b) => b,
            Response::BallotAudited(a) => {
                let commitment = self.commitment.take().ok_or_else(|| {
                    ClientError::Verification(
                        "o dispositivo auditou uma cédula sem ter se comprometido com ela".into(),
                    )
                })?;
                let mut want = Vec::new();
                for i in option_indices {
                    want.push(
                        contest
                            .options
                            .iter()
                            .position(|o| o.index == *i)
                            .ok_or_else(|| ClientError::Verification("opção inexistente".into()))?,
                    );
                }
                return audit_report(
                    &manifest,
                    contest,
                    election_id,
                    contest_id,
                    &want,
                    &commitment,
                    &a,
                )
                .map(BallotOutcome::Audited);
            }
            other => {
                return Err(ClientError::Unexpected(format!(
                    "resposta inesperada: {other:?}"
                )));
            }
        };
        if encrypted_mode {
            // what is cast is exactly what the device committed to before the choice
            let committed = self
                .commitment
                .take()
                .is_some_and(|c| c.encrypted == b.encrypted);
            if !committed {
                return Err(ClientError::Verification(
                    "a cédula votada NÃO é a que o dispositivo anunciou antes da escolha".into(),
                ));
            }
        }
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
            let special = contest.special_positions();
            let shape = teko_ballot_crypto::BallotShape {
                n,
                seats: contest.seat_count(),
                special: &special,
            };
            let ctx = teko_election::ballot_ctx(election_id, contest_id, &b.voter_key);
            let ok =
                teko_ballot_crypto::EncryptedBallot::from_bytes_seats(n, shape.seats, &b.encrypted)
                    .is_some_and(|eb| {
                        teko_ballot_crypto::verify_ballot_shape(&pk, &shape, &ctx, &eb)
                    });
            if !ok {
                return Err(ClientError::Verification(
                    "as provas da cédula cifrada NÃO conferem".into(),
                ));
            }
        }
        Ok(BallotOutcome::Cast(SignedBallot {
            election_id: election_id.to_string(),
            contest_id: contest_id.to_string(),
            contest_hash: hex::encode(&body.contest_hash),
            contest_title: contest.title.clone(),
            option_index,
            option_indices: option_indices.to_vec(),
            option_name,
            voter_key: hex::encode(&b.voter_key),
            signature: hex::encode(&b.signature),
            encrypted: hex::encode(&b.encrypted),
        }))
    }
}

/// What the host brings so the DEVICE can check its own ballot alone (ADR-0041, ADR-0045).
#[derive(Debug, Clone, Default)]
pub struct BallotEvidence {
    /// The block payload (what the institutions signed).
    pub block: Vec<u8>,
    /// The encoded commit certificate of the block.
    pub commit: Vec<u8>,
    /// Index of the ballot's leaf in the cumulative log, and its Merkle proof in the block's root.
    pub index: u64,
    pub proof: Vec<[u8; 32]>,
    /// Every approved change of the institutions since the genesis set (encoded), in order.
    pub rotations: Vec<Vec<u8>>,
}

/// What the DEVICE confirmed about its own ballot (ADR-0041): public facts only.
#[derive(Debug, Clone, Serialize)]
pub struct RecordedReport {
    pub height: u64,
    pub signers: u32,
    pub institutions: u32,
    pub tracker: String,
    /// The epoch of the institutions that certified the block (0 = the genesis set, ADR-0045).
    pub epoch: u64,
}

/// What the voter chose ON THE DEVICE for a committed ballot (ADR-0041).
#[derive(Debug, Clone)]
pub enum BallotOutcome {
    Cast(SignedBallot),
    Audited(AuditReport),
}

/// Result of opening an AUDITED (spoiled, never cast) ballot.
#[derive(Debug, Clone, Serialize)]
pub struct AuditReport {
    pub election_id: String,
    pub contest_id: String,
    pub contest_title: String,
    pub option_name: String,
    pub ciphertext_hash: String,
    /// One line for the screen: "a cédula de teste continha X".
    pub summary: String,
}

/// Opens an audited ballot: the commitment must be self-consistent, the proofs valid, and the
/// ballot must hold EXACTLY the option that was asked for. Anything else is a device that lied.
fn audit_report(
    manifest: &ElectionManifest,
    contest: &teko_election::Contest,
    election_id: &str,
    contest_id: &str,
    want_positions: &[usize],
    commitment: &BallotCommitment,
    a: &BallotAudited,
) -> Result<AuditReport, ClientError> {
    let bad = |m: &str| ClientError::Verification(format!("auditoria: {m}"));
    if manifest.tally_key.is_empty() {
        return Err(bad("esta eleição não é cifrada: não há o que auditar"));
    }
    if teko_election::ciphertext_hash(&commitment.encrypted) != commitment.ciphertext_hash {
        return Err(bad("o compromisso não confere com a própria cédula"));
    }
    let pk = teko_ballot_crypto::point_from_bytes(&manifest.tally_key)
        .ok_or_else(|| bad("chave da eleição inválida"))?;
    let n = contest.options.len();
    let special = contest.special_positions();
    let shape = teko_ballot_crypto::BallotShape {
        n,
        seats: contest.seat_count(),
        special: &special,
    };
    let ctx = teko_election::ballot_ctx(election_id, contest_id, &a.voter_key);
    let eb = teko_ballot_crypto::EncryptedBallot::from_bytes_seats(
        n,
        shape.seats,
        &commitment.encrypted,
    )
    .ok_or_else(|| bad("cédula malformada"))?;
    let rs: Option<Vec<_>> = a
        .randomness
        .iter()
        .map(|r| teko_ballot_crypto::scalar_from_bytes(r))
        .collect();
    let rs = rs.ok_or_else(|| bad("aleatório malformado"))?;
    let opened = teko_ballot_crypto::open_ballot_multi(&pk, &shape, &ctx, &eb, &rs)
        .ok_or_else(|| bad("a cédula NÃO abre com o aleatório revelado"))?;
    // the opened set must be EXACTLY what the voter asked for (order does not matter)
    let mut want = want_positions.to_vec();
    want.sort_unstable();
    want.dedup();
    if opened != want {
        return Err(bad(
            "o dispositivo cifrou OUTRA escolha, não a que você fez: NÃO use este dispositivo",
        ));
    }
    let name = opened
        .iter()
        .filter_map(|p| contest.options.get(*p).map(|o| o.name.clone()))
        .collect::<Vec<_>>()
        .join(", ");
    Ok(AuditReport {
        election_id: election_id.to_string(),
        contest_id: contest_id.to_string(),
        contest_title: contest.title.clone(),
        option_name: name.clone(),
        ciphertext_hash: hex::encode(&commitment.ciphertext_hash),
        summary: format!("continha o voto em {name}"),
    })
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
    /// The ballot number of each option (ADR-0040); empty strings when an option has none.
    pub option_numbers: Vec<String>,
    /// Per option: 0 candidate, 1 blank, 2 null (ADR-0042).
    pub option_kinds: Vec<u32>,
    /// Per option: the party number of a candidate (proportional contests, ADR-0046).
    pub option_parties: Vec<String>,
    /// Seats of the chamber of a proportional contest, else 0.
    pub chamber_seats: u32,
    /// How many people the contest elects (ADR-0044): the voter marks up to this many options.
    pub seats: u32,
    /// Set on a SECOND ROUND: the first-round contest it continues.
    pub parent: String,
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
    /// ALL the options of a multi-seat vote (ADR-0044); one entry for one seat.
    pub option_indices: Vec<u32>,
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
                option_numbers: c.option_numbers.clone(),
                option_kinds: c.option_kinds.clone(),
                option_parties: c.option_parties.clone(),
                chamber_seats: c.chamber_seats,
                seats: c.seats.max(1),
                parent: c.parent.clone(),
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

/// The assets transaction the device signed: rebuilt from the fields, signature checked.
/// Which credentials operation the device is asked to sign (the numbers are the protocol's).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum CredentialOpKind {
    Propose = 1,
    Approve = 2,
    Veto = 3,
    Revoke = 4,
}

/// Arguments of `wallet_sign_credential_op`.
#[derive(Debug, Clone)]
pub struct CredentialOpArgs<'a> {
    pub chain_id: &'a str,
    pub kind: CredentialOpKind,
    pub ctype: &'a str,
    /// The holder's credential key (from `credential_key` on the HOLDER's device).
    pub subject: &'a [u8],
    /// The claims `name=value;name=value`; empty only for a vote on a private credential.
    pub claims: &'a [u8],
    pub public: bool,
    /// 16 bytes, for a private credential with claims.
    pub salt: Option<&'a [u8]>,
    /// The commitment on the chain, for a vote without the claims.
    pub commitment: Option<&'a [u8]>,
    pub valid_until: u64,
    pub reason: &'a str,
    pub contest_secs: u64,
    pub fee: u64,
    pub nonce: u64,
}

fn asset_tx(
    chain_id: &str,
    nonce: u64,
    fee: u64,
    op: teko_assets::Op,
    signed: TransferSigned,
) -> Result<teko_ledger::Tx, ClientError> {
    let tx = teko_ledger::Tx {
        chain_id: chain_id.to_string(),
        nonce,
        module: teko_assets::ASSETS_MODULE,
        payload: op.encode(),
        sender: signed.public_key,
        signature: signed.signature,
        fee,
    };
    tx.verify().map_err(|_| {
        ClientError::Verification("a assinatura devolvida não confere com a transação".into())
    })?;
    Ok(tx)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod audit_tests {
    use super::*;
    use p256::elliptic_curve::Field;
    use rand::SeedableRng;
    use rand::rngs::StdRng;
    use teko_ballot_crypto::{Point, Scalar, encrypt_ballot_with, point_to_bytes, scalar_to_bytes};

    fn fixture() -> (ElectionManifest, teko_election::Contest, Point) {
        let x = Scalar::random(&mut StdRng::seed_from_u64(1));
        let pk = Point::GENERATOR * x;
        let contest = teko_election::Contest {
            contest_id: "presidente".into(),
            title: "Presidente".into(),
            options: (0..3u32)
                .map(|i| teko_election::ElectionOption {
                    index: i,
                    name: format!("Candidato {i}"),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let m = ElectionManifest {
            election_id: "e".into(),
            tally_key: point_to_bytes(&pk).to_vec(),
            ..Default::default()
        };
        (m, contest, pk)
    }

    /// What a device would send for a ballot encrypting `encrypts` (the voter asked for `asked`).
    fn run(encrypts: usize, asked: usize, tamper_hash: bool) -> Result<AuditReport, ClientError> {
        let (m, c, pk) = fixture();
        let vk = vec![2u8; 33];
        let ctx = teko_election::ballot_ctx("e", "presidente", &vk);
        let (eb, rs) =
            encrypt_ballot_with(&pk, 3, encrypts, &ctx, &mut StdRng::seed_from_u64(9)).unwrap();
        let bytes = eb.to_bytes();
        let mut hash = teko_election::ciphertext_hash(&bytes);
        if tamper_hash {
            hash[0] ^= 1;
        }
        audit_report(
            &m,
            &c,
            "e",
            "presidente",
            &[asked],
            &BallotCommitment {
                ciphertext_hash: hash,
                encrypted: bytes,
            },
            &BallotAudited {
                option_position: encrypts as u32,
                randomness: rs.iter().map(|r| scalar_to_bytes(r).to_vec()).collect(),
                voter_key: vk,
                ..Default::default()
            },
        )
    }

    #[test]
    fn an_honest_device_passes_the_audit() {
        let r = run(1, 1, false).unwrap();
        assert_eq!(r.option_name, "Candidato 1");
        assert!(r.summary.contains("Candidato 1"));
    }

    #[test]
    fn a_device_that_encrypted_another_option_is_caught() {
        let e = run(0, 2, false).unwrap_err().to_string();
        assert!(e.contains("OUTRA escolha"), "{e}");
    }

    /// A two-seat senate contest: 4 candidates, blank and null.
    fn senate_run(encrypts: &[usize], asked: &[usize]) -> Result<AuditReport, ClientError> {
        use teko_ballot_crypto::{BallotShape, encrypt_ballot_multi_with};
        let x = Scalar::random(&mut StdRng::seed_from_u64(3));
        let pk = Point::GENERATOR * x;
        let mut options: Vec<teko_election::ElectionOption> = (0..4u32)
            .map(|i| teko_election::ElectionOption {
                index: i,
                name: format!("Candidato {i}"),
                ..Default::default()
            })
            .collect();
        for (i, (k, n)) in [
            (teko_election::OptionKind::Blank, "Branco"),
            (teko_election::OptionKind::Null, "Nulo"),
        ]
        .into_iter()
        .enumerate()
        {
            options.push(teko_election::ElectionOption {
                index: 4 + i as u32,
                name: n.into(),
                kind: k as i32,
                ..Default::default()
            });
        }
        let contest = teko_election::Contest {
            contest_id: "senador-sp".into(),
            office: teko_election::Office::Senator as i32,
            title: "Senador".into(),
            seats: 2,
            options,
            ..Default::default()
        };
        let m = ElectionManifest {
            election_id: "e".into(),
            tally_key: point_to_bytes(&pk).to_vec(),
            ..Default::default()
        };
        let vk = vec![2u8; 33];
        let ctx = teko_election::ballot_ctx("e", "senador-sp", &vk);
        let special = contest.special_positions();
        let shape = BallotShape {
            n: 6,
            seats: 2,
            special: &special,
        };
        let (eb, rs) =
            encrypt_ballot_multi_with(&pk, &shape, encrypts, &ctx, &mut StdRng::seed_from_u64(9))
                .unwrap();
        let bytes = eb.to_bytes();
        audit_report(
            &m,
            &contest,
            "e",
            "senador-sp",
            asked,
            &BallotCommitment {
                ciphertext_hash: teko_election::ciphertext_hash(&bytes),
                encrypted: bytes,
            },
            &BallotAudited {
                randomness: rs.iter().map(|r| scalar_to_bytes(r).to_vec()).collect(),
                voter_key: vk,
                option_positions: encrypts.iter().map(|p| *p as u32).collect(),
                ..Default::default()
            },
        )
    }

    #[test]
    fn a_two_seat_audit_must_open_to_exactly_the_set_that_was_asked_for() {
        let r = senate_run(&[1, 3], &[3, 1]).unwrap();
        assert!(
            r.summary.contains("Candidato 1") && r.summary.contains("Candidato 3"),
            "{}",
            r.summary
        );
        assert!(senate_run(&[4], &[4]).is_ok(), "blank alone");
        // another set, a subset, a superset, one swapped candidate: the device lied
        for (enc, asked) in [
            (&[0usize, 2][..], &[0usize, 1][..]),
            (&[0], &[0, 1]),
            (&[0, 1], &[0]),
            (&[0, 2], &[0, 5]),
        ] {
            let e = senate_run(enc, asked).unwrap_err().to_string();
            assert!(e.contains("OUTRA escolha"), "{enc:?} vs {asked:?}: {e}");
        }
    }

    #[test]
    fn a_commitment_that_does_not_match_its_ballot_is_refused() {
        let e = run(1, 1, true).unwrap_err().to_string();
        assert!(e.contains("compromisso"), "{e}");
    }
}
