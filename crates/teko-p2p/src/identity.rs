use std::fs;
use std::io::Write;
use std::path::Path;

use prost::Message;
use teko_crypto::{SigningKey, public_key_bytes, sign_tagged, tagged_hash, tags, verify_tagged};
use zeroize::Zeroizing;

use crate::error::NetError;

/// Who may talk to this node: the consensus keys of the validators in force (kept up to date by the
/// host from the chain epochs, ADR-0045).
pub trait PeerPolicy: Send + Sync {
    fn accepts(&self, consensus_key: &[u8]) -> bool;
}

/// What a node is on the network: its consensus key (the institution), the chain and genesis it
/// belongs to, and its Noise static keypair (made on the first run, never leaves the node).
pub struct NetIdentity {
    pub consensus: SigningKey,
    pub chain_id: String,
    pub genesis_hash: Vec<u8>,
    pub(crate) noise_private: Zeroizing<Vec<u8>>,
    pub(crate) noise_public: Vec<u8>,
}

/// Sent inside the handshake: the consensus key and its signature over THIS Noise static key, the
/// chain and the genesis. Nobody can lend a proof to another static key or replay it on another chain.
#[derive(Clone, PartialEq, Message)]
pub struct IdentityProof {
    #[prost(bytes = "vec", tag = "1")]
    pub consensus_key: Vec<u8>,
    #[prost(string, tag = "2")]
    pub chain_id: String,
    #[prost(bytes = "vec", tag = "3")]
    pub genesis_hash: Vec<u8>,
    #[prost(bytes = "vec", tag = "4")]
    pub signature: Vec<u8>,
}

pub(crate) const NOISE_PARAMS: &str = "Noise_XX_25519_ChaChaPoly_SHA256";

/// The bytes the proof signs. `role` (1 = initiator, 2 = responder) stops a proof from being
/// reflected back at its author.
fn bound(chain_id: &str, genesis_hash: &[u8], noise_static: &[u8], role: u8) -> [u8; 32] {
    tagged_hash(
        tags::NET_BIND,
        &[chain_id.as_bytes(), genesis_hash, noise_static, &[role]],
    )
}

impl NetIdentity {
    /// A fresh identity with a new Noise keypair (kept in memory).
    pub fn generate(
        consensus: SigningKey,
        chain_id: &str,
        genesis_hash: &[u8],
    ) -> Result<Self, NetError> {
        let params = NOISE_PARAMS
            .parse()
            .map_err(|_| NetError::Handshake("noise parameters".into()))?;
        let kp = snow::Builder::new(params)
            .generate_keypair()
            .map_err(|e| NetError::Handshake(e.to_string()))?;
        Ok(Self {
            consensus,
            chain_id: chain_id.to_string(),
            genesis_hash: genesis_hash.to_vec(),
            noise_private: Zeroizing::new(kp.private),
            noise_public: kp.public,
        })
    }

    /// Loads the Noise keypair of `path` (hex, `private || public`), or makes and saves it (0600).
    pub fn load_or_create(
        consensus: SigningKey,
        chain_id: &str,
        genesis_hash: &[u8],
        path: &Path,
    ) -> Result<Self, NetError> {
        if let Ok(text) = fs::read_to_string(path) {
            let raw = hex_decode(text.trim()).ok_or_else(|| bad_file(path))?;
            if raw.len() == 64 {
                return Ok(Self {
                    consensus,
                    chain_id: chain_id.to_string(),
                    genesis_hash: genesis_hash.to_vec(),
                    noise_private: Zeroizing::new(raw[..32].to_vec()),
                    noise_public: raw[32..].to_vec(),
                });
            }
            return Err(bad_file(path));
        }
        let id = Self::generate(consensus, chain_id, genesis_hash)?;
        let mut both = id.noise_private.to_vec();
        both.extend_from_slice(&id.noise_public);
        let mut f = fs::OpenOptions::new();
        f.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            f.mode(0o600);
        }
        let mut file = f.open(path)?;
        file.write_all(hex_encode(&both).as_bytes())?;
        Ok(id)
    }

    /// The consensus key that names this node.
    pub fn consensus_key(&self) -> Vec<u8> {
        public_key_bytes(&self.consensus)
    }

    pub(crate) fn proof(&self, role: u8) -> IdentityProof {
        let msg = bound(&self.chain_id, &self.genesis_hash, &self.noise_public, role);
        IdentityProof {
            consensus_key: self.consensus_key(),
            chain_id: self.chain_id.clone(),
            genesis_hash: self.genesis_hash.clone(),
            signature: sign_tagged(&self.consensus, tags::NET_BIND, &msg),
        }
    }

    /// Checks a peer's proof against ITS Noise static key and role; returns its consensus key.
    pub(crate) fn check_proof(
        &self,
        raw: &[u8],
        remote_static: &[u8],
        role: u8,
        policy: &dyn PeerPolicy,
        expect: Option<&[u8]>,
    ) -> Result<Vec<u8>, NetError> {
        let bad = |m: &str| NetError::Handshake(m.to_string());
        let p = IdentityProof::decode(raw).map_err(|_| bad("malformed identity proof"))?;
        if p.chain_id != self.chain_id {
            return Err(bad("another chain"));
        }
        if p.genesis_hash != self.genesis_hash {
            return Err(bad("another genesis"));
        }
        let msg = bound(&p.chain_id, &p.genesis_hash, remote_static, role);
        if verify_tagged(&p.consensus_key, tags::NET_BIND, &msg, &p.signature).is_err() {
            return Err(bad("the proof does not bind this connection's key"));
        }
        if let Some(want) = expect
            && want != p.consensus_key.as_slice()
        {
            return Err(bad("not the institution that was dialled"));
        }
        if !policy.accepts(&p.consensus_key) {
            return Err(bad("not a validator in force"));
        }
        Ok(p.consensus_key)
    }
}

fn bad_file(path: &Path) -> NetError {
    NetError::Handshake(format!("{}: not a Noise key file", path.display()))
}

pub(crate) fn hex_encode(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub(crate) fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) || !s.is_ascii() {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}
