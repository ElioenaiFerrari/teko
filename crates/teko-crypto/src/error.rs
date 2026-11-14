use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CryptoError {
    #[error("invalid key")]
    InvalidKey,
    #[error("invalid signature")]
    InvalidSignature,
    #[error("invalid identifier")]
    InvalidId,
    #[error("invalid argument")]
    InvalidArgument,
    #[error("key derivation failed")]
    Derivation,
    #[error("invalid hex")]
    InvalidHex,
    #[error("malformed credential")]
    MalformedCredential,
    #[error("credential expired")]
    Expired,
    #[error("nonce mismatch")]
    NonceMismatch,
}
