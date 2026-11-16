//! ROTATION of the institutions (ADR-0045): a new ordered set takes effect from a block height if
//! MORE THAN 2/3 of the set in force signed it. The document is pure data and its checks need no
//! `std`, so the chain, the economy ledger and the voter's DEVICE all verify it the same way.
//!
//! A change names the epoch it creates (the current one + 1) and the hash of the set it replaces, so
//! changes form a chain that can only be followed in order: a device that knows epoch `k` and its set
//! checks a list of changes link by link.

use alloc::string::String;
use alloc::vec::Vec;

use prost::Message;
use teko_crypto::{SigningKey, public_key_bytes, sign_tagged, tagged_hash, tags, verify_tagged};

/// How many institutions a set may have.
pub const MIN_SET: usize = 2;
pub const MAX_SET: usize = 64;

/// The identity of an ORDERED set of consensus keys (the order decides the proposer).
pub fn set_hash(keys: &[Vec<u8>]) -> [u8; 32] {
    let mut parts: Vec<Vec<u8>> = Vec::with_capacity(keys.len() + 1);
    parts.push((keys.len() as u32).to_be_bytes().to_vec());
    for k in keys {
        parts.push(k.clone());
    }
    let refs: Vec<&[u8]> = parts.iter().map(Vec::as_slice).collect();
    tagged_hash(tags::VALIDATOR_SET, &refs)
}

/// A new institution set, approved by more than 2/3 of the one in force.
#[derive(Clone, PartialEq, Eq, Message)]
pub struct ValidatorSetChange {
    #[prost(string, tag = "1")]
    pub chain_id: String,
    /// The epoch this change creates: the current one + 1 (the genesis set is epoch 0).
    #[prost(uint64, tag = "2")]
    pub epoch: u64,
    /// The first block height that belongs to the new set (it must be after the block carrying the
    /// change).
    #[prost(uint64, tag = "3")]
    pub effective_height: u64,
    /// `set_hash` of the set this one replaces.
    #[prost(bytes = "vec", tag = "4")]
    pub prev_set_hash: Vec<u8>,
    /// The new set, ORDERED (33-byte SEC1 consensus keys).
    #[prost(bytes = "vec", repeated, tag = "5")]
    pub validators: Vec<Vec<u8>>,
    #[prost(bytes = "vec", repeated, tag = "6")]
    pub signers: Vec<Vec<u8>>,
    #[prost(bytes = "vec", repeated, tag = "7")]
    pub signatures: Vec<Vec<u8>>,
}

impl ValidatorSetChange {
    /// What every approver signs: everything except the signatures.
    pub fn signing_bytes(&self) -> Vec<u8> {
        tagged_hash(
            tags::VALIDATOR_SET_CHANGE,
            &[
                self.chain_id.as_bytes(),
                &self.epoch.to_be_bytes(),
                &self.effective_height.to_be_bytes(),
                &self.prev_set_hash,
                &set_hash(&self.validators),
            ],
        )
        .to_vec()
    }

    /// One approver's signature, added to the change.
    pub fn sign(&mut self, key: &SigningKey) {
        let me = public_key_bytes(key);
        if self.signers.contains(&me) {
            return;
        }
        let sig = sign_tagged(key, tags::VALIDATOR_SET_CHANGE, &self.signing_bytes());
        self.signers.push(me);
        self.signatures.push(sig);
    }

    /// Is the NEW set well formed: between 2 and 64 valid, distinct keys?
    pub fn check_new_set(&self) -> Result<(), &'static str> {
        if self.validators.len() < MIN_SET || self.validators.len() > MAX_SET {
            return Err("a set has between 2 and 64 institutions");
        }
        for (i, k) in self.validators.iter().enumerate() {
            // the canonical form: SEC1 COMPRESSED (a 0x05 'compact' prefix would also parse)
            if k.len() != 33
                || !matches!(k.first(), Some(2 | 3))
                || teko_crypto::verifying_key_from_bytes(k).is_err()
            {
                return Err("an institution key is not a valid consensus key");
            }
            if self.validators[..i].contains(k) {
                return Err("an institution appears twice");
            }
        }
        Ok(())
    }

    /// Did MORE THAN 2/3 of `current` (distinct members, valid signatures) approve this change?
    pub fn check_approval(&self, current: &[Vec<u8>]) -> Result<(), &'static str> {
        if self.signers.len() != self.signatures.len() || self.signers.len() > MAX_SET {
            return Err("malformed approvals");
        }
        let msg = self.signing_bytes();
        let mut counted: Vec<&[u8]> = Vec::new();
        for (signer, sig) in self.signers.iter().zip(&self.signatures) {
            if current.contains(signer)
                && !counted.contains(&signer.as_slice())
                && verify_tagged(signer, tags::VALIDATOR_SET_CHANGE, &msg, sig).is_ok()
            {
                counted.push(signer);
            }
        }
        if counted.len() * 3 <= current.len() * 2 {
            return Err("more than 2/3 of the institutions in force must approve the change");
        }
        Ok(())
    }

    /// Follows ONE link: this change must create epoch `epoch + 1` from the set `current` (whose
    /// epoch is `epoch`), approved by more than 2/3 of it, with a well-formed new set.
    pub fn follow(
        &self,
        chain_id: &str,
        epoch: u64,
        current: &[Vec<u8>],
    ) -> Result<(), &'static str> {
        self.check_link(chain_id, epoch, current)?;
        self.check_approval(current)
    }

    /// Everything `follow` checks EXCEPT the approvals: what an institution verifies before it adds
    /// its own signature (the approvals are still being collected).
    pub fn check_link(
        &self,
        chain_id: &str,
        epoch: u64,
        current: &[Vec<u8>],
    ) -> Result<(), &'static str> {
        if self.chain_id != chain_id {
            return Err("a change of another chain");
        }
        if self.epoch != epoch.saturating_add(1) {
            return Err("the epoch is not the next one");
        }
        if self.prev_set_hash.as_slice() != set_hash(current).as_slice() {
            return Err("the change does not replace the set in force");
        }
        self.check_new_set()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand::rngs::StdRng;
    use teko_crypto::generate_signing_key;

    fn keys(n: usize, seed: u64) -> Vec<SigningKey> {
        let mut rng = StdRng::seed_from_u64(seed);
        (0..n).map(|_| generate_signing_key(&mut rng)).collect()
    }

    fn pubs(k: &[SigningKey]) -> Vec<Vec<u8>> {
        k.iter().map(public_key_bytes).collect()
    }

    fn change(cur: &[SigningKey], new: &[SigningKey], epoch: u64) -> ValidatorSetChange {
        ValidatorSetChange {
            chain_id: "c".into(),
            epoch,
            effective_height: 10,
            prev_set_hash: set_hash(&pubs(cur)).to_vec(),
            validators: pubs(new),
            ..Default::default()
        }
    }

    #[test]
    fn a_change_needs_more_than_two_thirds_of_the_set_in_force() {
        let (old, new) = (keys(4, 1), keys(4, 2));
        let mut c = change(&old, &new, 1);
        for k in &old[..2] {
            c.sign(k);
        }
        assert!(
            c.follow("c", 0, &pubs(&old)).is_err(),
            "2 of 4 is not more than 2/3"
        );
        c.sign(&old[2]);
        assert!(c.follow("c", 0, &pubs(&old)).is_ok(), "3 of 4 is");
        // a duplicated signature counts once
        let mut d = change(&old, &new, 1);
        d.sign(&old[0]);
        d.signers.push(d.signers[0].clone());
        d.signatures.push(d.signatures[0].clone());
        d.sign(&old[1]);
        assert!(d.check_approval(&pubs(&old)).is_err());
        // outsiders do not count (the NEW set cannot approve itself)
        let mut e = change(&old, &new, 1);
        for k in &new {
            e.sign(k);
        }
        assert!(e.check_approval(&pubs(&old)).is_err());
    }

    #[test]
    fn a_change_is_bound_to_its_epoch_chain_set_and_every_field() {
        let (old, new) = (keys(4, 3), keys(3, 4));
        let mut c = change(&old, &new, 1);
        for k in &old[..3] {
            c.sign(k);
        }
        assert!(c.follow("c", 0, &pubs(&old)).is_ok());
        assert!(c.follow("other", 0, &pubs(&old)).is_err(), "another chain");
        assert!(
            c.follow("c", 1, &pubs(&old)).is_err(),
            "a skipped or repeated epoch"
        );
        assert!(
            c.follow("c", 0, &pubs(&new)).is_err(),
            "it does not replace that set"
        );
        // changing any signed field breaks the approvals
        for edit in [
            (|c: &mut ValidatorSetChange| c.effective_height += 1) as fn(&mut ValidatorSetChange),
            |c| c.validators.reverse(),
            |c| c.validators.pop().map(|_| ()).unwrap_or(()),
            |c| c.epoch += 1,
        ] {
            let mut t = c.clone();
            edit(&mut t);
            assert!(t.check_approval(&pubs(&old)).is_err());
        }
    }

    #[test]
    fn the_new_set_must_be_two_to_sixty_four_distinct_valid_keys_and_the_order_matters() {
        let old = keys(4, 5);
        let mut c = change(&old, &keys(1, 6), 1);
        assert!(c.check_new_set().is_err(), "one institution");
        c.validators = pubs(&keys(65, 7));
        assert!(c.check_new_set().is_err(), "65 institutions");
        let ok = keys(3, 8);
        c.validators = pubs(&ok);
        assert!(c.check_new_set().is_ok());
        c.validators[1] = c.validators[0].clone();
        assert!(c.check_new_set().is_err(), "twice");
        c.validators = pubs(&ok);
        c.validators[2] = vec![5; 33]; // not a SEC1 point (bad prefix)
        assert!(c.check_new_set().is_err(), "an invalid key");
        // the same keys in another order are another set (the proposer depends on the order)
        let mut rev = pubs(&ok);
        assert_eq!(set_hash(&rev), set_hash(&pubs(&ok)));
        rev.reverse();
        assert_ne!(set_hash(&rev), set_hash(&pubs(&ok)));
    }
}
