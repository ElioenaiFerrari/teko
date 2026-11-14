use std::collections::BTreeMap;

use teko_bulletin::merkle;
use teko_crypto::{tagged_hash, tags};

use crate::tx::Hash;

/// Key-value state with a deterministic Merkle root: leaves are the entries sorted
/// by key, so the root does not depend on insertion order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct State {
    map: BTreeMap<Vec<u8>, Vec<u8>>,
}

/// Inclusion proof of one entry in a state root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateProof {
    pub index: u64,
    pub size: u64,
    pub path: Vec<Hash>,
}

fn leaf(key: &[u8], value: &[u8]) -> Hash {
    merkle::leaf_hash(&tagged_hash(tags::LEDGER_STATE_LEAF, &[key, value]))
}

impl State {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, key: &[u8]) -> Option<&[u8]> {
        self.map.get(key).map(Vec::as_slice)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&[u8], &[u8])> {
        self.map.iter().map(|(k, v)| (k.as_slice(), v.as_slice()))
    }

    pub(crate) fn put(&mut self, key: Vec<u8>, value: Vec<u8>) {
        self.map.insert(key, value);
    }

    pub(crate) fn remove(&mut self, key: &[u8]) {
        self.map.remove(key);
    }

    fn leaves(&self) -> Vec<Hash> {
        self.map.iter().map(|(k, v)| leaf(k, v)).collect()
    }

    pub fn root(&self) -> Hash {
        merkle::root(&self.leaves())
    }

    /// Inclusion proof for `key` (None when the key is absent).
    pub fn prove(&self, key: &[u8]) -> Option<StateProof> {
        let index = self.map.keys().position(|k| k.as_slice() == key)?;
        let leaves = self.leaves();
        let path = merkle::inclusion_proof(&leaves, index)?;
        Some(StateProof {
            index: index as u64,
            size: leaves.len() as u64,
            path,
        })
    }
}

/// Checks that `(key, value)` is in the state with root `root`.
pub fn verify_state_proof(root: &Hash, key: &[u8], value: &[u8], proof: &StateProof) -> bool {
    merkle::verify_inclusion(
        &leaf(key, value),
        proof.index,
        proof.size,
        &proof.path,
        root,
    )
}
