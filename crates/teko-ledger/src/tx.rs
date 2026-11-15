use prost::Message;
use teko_crypto::{SigningKey, public_key_bytes, sign_tagged, tagged_hash, tags, verify_tagged};

pub type Hash = [u8; 32];
/// Account address: hash of the signer's public key.
pub type Address = [u8; 32];

/// Largest accepted payload (bounds memory and verification cost).
pub const MAX_PAYLOAD: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TxError {
    Malformed,
    PayloadTooLarge,
    BadSignature,
    WrongChain,
    BadNonce { expected: u64, got: u64 },
    UnknownModule(u32),
    FeeTooLow { min: u64, got: u64 },
    Module(String),
}

/// Address of a public key.
pub fn address_of(public_key: &[u8]) -> Address {
    tagged_hash(tags::LEDGER_ADDRESS, &[public_key])
}

/// Signed transaction envelope.
#[derive(Clone, PartialEq, Eq, Message)]
pub struct Tx {
    #[prost(string, tag = "1")]
    pub chain_id: String,
    /// Equals the number of transactions the sender has already executed.
    #[prost(uint64, tag = "2")]
    pub nonce: u64,
    /// Which module interprets `payload` (0 is reserved for the core).
    #[prost(uint32, tag = "3")]
    pub module: u32,
    #[prost(bytes = "vec", tag = "4")]
    pub payload: Vec<u8>,
    /// SEC1 public key of the signer.
    #[prost(bytes = "vec", tag = "5")]
    pub sender: Vec<u8>,
    #[prost(bytes = "vec", tag = "6")]
    pub signature: Vec<u8>,
    /// Fee in the smallest coin unit, covered by the signature (nobody can raise it
    /// after you sign). Charged by the fee module; ignored when there is none.
    #[prost(uint64, tag = "7")]
    pub fee: u64,
}

impl Tx {
    /// Builds and signs a transaction.
    pub fn new_signed(
        key: &SigningKey,
        chain_id: &str,
        nonce: u64,
        module: u32,
        payload: Vec<u8>,
    ) -> Tx {
        Self::new_signed_with_fee(key, chain_id, nonce, 0, module, payload)
    }

    /// Builds and signs a transaction that pays `fee`.
    pub fn new_signed_with_fee(
        key: &SigningKey,
        chain_id: &str,
        nonce: u64,
        fee: u64,
        module: u32,
        payload: Vec<u8>,
    ) -> Tx {
        let mut tx = Tx {
            fee,
            chain_id: chain_id.into(),
            nonce,
            module,
            payload,
            sender: public_key_bytes(key),
            signature: Vec::new(),
        };
        tx.signature = sign_tagged(key, tags::LEDGER_TX, &tx.signing_bytes());
        tx
    }

    /// Canonical bytes covered by the signature (everything but the signature).
    fn signing_bytes(&self) -> Vec<u8> {
        teko_crypto::ledger::tx_signing_bytes(
            &self.chain_id,
            self.nonce,
            self.fee,
            self.module,
            &self.payload,
            &self.sender,
        )
    }

    /// Cheap stateless checks: size and signature.
    pub fn verify(&self) -> Result<(), TxError> {
        if self.payload.len() > MAX_PAYLOAD {
            return Err(TxError::PayloadTooLarge);
        }
        verify_tagged(
            &self.sender,
            tags::LEDGER_TX,
            &self.signing_bytes(),
            &self.signature,
        )
        .map_err(|_| TxError::BadSignature)
    }

    pub fn address(&self) -> Address {
        address_of(&self.sender)
    }

    /// Stable identifier (hash of the canonical encoding).
    pub fn id(&self) -> Hash {
        tagged_hash(tags::LEDGER_TX_ID, &[&self.encode_to_vec()])
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        self.encode_to_vec()
    }

    /// Decodes and re-encodes to reject non-canonical encodings.
    pub fn from_bytes(bytes: &[u8]) -> Result<Tx, TxError> {
        let tx = Tx::decode(bytes).map_err(|_| TxError::Malformed)?;
        if tx.encode_to_vec() != bytes {
            return Err(TxError::Malformed);
        }
        Ok(tx)
    }
}
