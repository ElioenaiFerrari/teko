use serde::{Deserialize, Serialize};
use sha3::{Digest, Sha3_256};
use teko_crypto::{civic_id, hex, tags, verify_tagged};

use crate::error::ClientError;

/// Portable, independently verifiable signature bundle (JSON).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedDocument {
    pub format: String,
    pub hash_sha3_256: String,
    pub summary: String,
    pub civic_id: String,
    pub public_key: String,
    pub signature: String,
    pub scheme: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct VerifiedDocument {
    pub signature_valid: bool,
    pub civic_id: String,
    /// `None` when no file was supplied to compare against.
    pub matches_file: Option<bool>,
}

/// Plain SHA3-256 of the file (what the device displays as the hash).
pub fn file_hash(file: &[u8]) -> [u8; 32] {
    Sha3_256::digest(file).into()
}

impl SignedDocument {
    pub fn new(hash: &[u8; 32], summary: &str, pk: &[u8], sig: &[u8]) -> Result<Self, ClientError> {
        Ok(Self {
            format: "teko-signed-document/1".into(),
            hash_sha3_256: hex::encode(hash),
            summary: summary.to_string(),
            civic_id: civic_id(pk).map_err(|e| ClientError::Verification(e.to_string()))?,
            public_key: hex::encode(pk),
            signature: hex::encode(sig),
            scheme: "ECDSA-P256 over tagged-SHA3-256(teko/v1/doc-sign, hash)".into(),
        })
    }
}

/// Verifies a bundle: the signature, that the civic ID derives from the key,
/// and (optionally) that the file hashes to the signed value.
pub fn verify_signed_document(
    doc: &SignedDocument,
    file: Option<&[u8]>,
) -> Result<VerifiedDocument, ClientError> {
    let bad = |m: &str| ClientError::Verification(m.to_string());
    let hash = hex::decode(&doc.hash_sha3_256).map_err(|_| bad("hash inválido"))?;
    let pk = hex::decode(&doc.public_key).map_err(|_| bad("chave pública inválida"))?;
    let sig = hex::decode(&doc.signature).map_err(|_| bad("assinatura inválida"))?;
    if hash.len() != 32 {
        return Err(bad("hash deve ter 32 bytes"));
    }
    verify_tagged(&pk, tags::DOC_SIGN, &hash, &sig).map_err(|_| bad("assinatura NÃO confere"))?;
    let id = civic_id(&pk).map_err(|e| ClientError::Verification(e.to_string()))?;
    if id != doc.civic_id {
        return Err(bad("o ID Cívico não corresponde à chave pública"));
    }
    let matches_file = match file {
        Some(f) => {
            if file_hash(f).as_slice() != hash.as_slice() {
                return Err(bad("o arquivo NÃO corresponde ao hash assinado"));
            }
            Some(true)
        }
        None => None,
    };
    Ok(VerifiedDocument {
        signature_valid: true,
        civic_id: id,
        matches_file,
    })
}
