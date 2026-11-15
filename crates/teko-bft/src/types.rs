use std::collections::BTreeSet;

use prost::Message;
use teko_crypto::{SigningKey, public_key_bytes, sign_tagged, tagged_hash, tags, verify_tagged};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BftError {
    EmptySet,
    ZeroPower,
    DuplicateValidator,
    PowerOverflow,
}

/// A validator: its public key and its voting power (its stake).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Validator {
    pub key: Vec<u8>,
    pub power: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidatorSet {
    validators: Vec<Validator>,
    total: u64,
}

impl ValidatorSet {
    pub fn new(validators: Vec<Validator>) -> Result<Self, BftError> {
        if validators.is_empty() {
            return Err(BftError::EmptySet);
        }
        let mut total = 0u64;
        let mut seen = BTreeSet::new();
        for v in &validators {
            if v.power == 0 {
                return Err(BftError::ZeroPower);
            }
            if !seen.insert(v.key.clone()) {
                return Err(BftError::DuplicateValidator);
            }
            total = total.checked_add(v.power).ok_or(BftError::PowerOverflow)?;
        }
        // 3 * total must not overflow either (quorum arithmetic).
        total.checked_mul(3).ok_or(BftError::PowerOverflow)?;
        Ok(Self { validators, total })
    }

    pub fn total(&self) -> u64 {
        self.total
    }

    pub fn validators(&self) -> &[Validator] {
        &self.validators
    }

    pub fn power_of(&self, key: &[u8]) -> Option<u64> {
        self.validators
            .iter()
            .find(|v| v.key == key)
            .map(|v| v.power)
    }

    /// STRICTLY more than 2/3 of the total power.
    pub fn is_quorum(&self, power: u64) -> bool {
        power.saturating_mul(3) > self.total.saturating_mul(2)
    }

    /// STRICTLY more than 1/3 (at least one honest validator is among them).
    pub fn is_one_third_plus(&self, power: u64) -> bool {
        power.saturating_mul(3) > self.total
    }

    /// The proposer for `(height, round)`: deterministic and proportional to the power in
    /// the long run, chosen by a hash of `(height, round, total power)` so that a
    /// validator with a large power does NOT propose long contiguous runs of heights
    /// (a contiguous weighted rotation would hand the first validator thousands of
    /// consecutive heights). Every node computes the same answer.
    pub fn proposer(&self, height: u64, round: u32) -> &Validator {
        let h = tagged_hash(
            tags::BFT_PROPOSAL,
            &[
                b"proposer",
                &height.to_be_bytes(),
                &round.to_be_bytes(),
                &self.total.to_be_bytes(),
            ],
        );
        let mut word = [0u8; 8];
        word.copy_from_slice(&h[..8]);
        let mut slot = u64::from_be_bytes(word) % self.total;
        for v in &self.validators {
            if slot < v.power {
                return v;
            }
            slot -= v.power;
        }
        &self.validators[0]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum VoteKind {
    Prevote = 1,
    Precommit = 2,
}

/// The identity of a proposed block: the hash of its payload.
pub fn block_id(payload: &[u8]) -> Vec<u8> {
    tagged_hash(tags::BFT_BLOCK, &[payload]).to_vec()
}

#[derive(Clone, PartialEq, Message)]
pub struct Vote {
    #[prost(uint32, tag = "1")]
    pub kind: u32,
    #[prost(uint64, tag = "2")]
    pub height: u64,
    #[prost(uint32, tag = "3")]
    pub round: u32,
    /// Empty = a vote for NIL (no block).
    #[prost(bytes = "vec", tag = "4")]
    pub block_id: Vec<u8>,
    #[prost(bytes = "vec", tag = "5")]
    pub validator: Vec<u8>,
    #[prost(bytes = "vec", tag = "6")]
    pub signature: Vec<u8>,
}

impl Vote {
    fn sign_bytes(chain_id: &str, kind: u32, height: u64, round: u32, id: &[u8]) -> Vec<u8> {
        tagged_hash(
            tags::BFT_VOTE,
            &[
                chain_id.as_bytes(),
                &kind.to_be_bytes(),
                &height.to_be_bytes(),
                &round.to_be_bytes(),
                id,
            ],
        )
        .to_vec()
    }

    pub fn new(
        chain_id: &str,
        key: &SigningKey,
        kind: VoteKind,
        height: u64,
        round: u32,
        id: Option<&[u8]>,
    ) -> Self {
        let block_id = id.map(<[u8]>::to_vec).unwrap_or_default();
        let msg = Self::sign_bytes(chain_id, kind as u32, height, round, &block_id);
        Self {
            kind: kind as u32,
            height,
            round,
            block_id,
            validator: public_key_bytes(key),
            signature: sign_tagged(key, tags::BFT_VOTE, &msg),
        }
    }

    pub fn verify(&self, chain_id: &str) -> bool {
        (self.kind == 1 || self.kind == 2)
            && (self.block_id.is_empty() || self.block_id.len() == 32)
            && verify_tagged(
                &self.validator,
                tags::BFT_VOTE,
                &Self::sign_bytes(chain_id, self.kind, self.height, self.round, &self.block_id),
                &self.signature,
            )
            .is_ok()
    }

    pub fn vote_kind(&self) -> Option<VoteKind> {
        match self.kind {
            1 => Some(VoteKind::Prevote),
            2 => Some(VoteKind::Precommit),
            _ => None,
        }
    }
}

#[derive(Clone, PartialEq, Message)]
pub struct Proposal {
    #[prost(uint64, tag = "1")]
    pub height: u64,
    #[prost(uint32, tag = "2")]
    pub round: u32,
    /// The round at which this value got 2/3 prevotes, or -1 for a fresh proposal.
    #[prost(sint32, tag = "3")]
    pub valid_round: i32,
    #[prost(bytes = "vec", tag = "4")]
    pub payload: Vec<u8>,
    #[prost(bytes = "vec", tag = "5")]
    pub proposer: Vec<u8>,
    #[prost(bytes = "vec", tag = "6")]
    pub signature: Vec<u8>,
}

impl Proposal {
    fn sign_bytes(chain_id: &str, height: u64, round: u32, valid_round: i32, id: &[u8]) -> Vec<u8> {
        tagged_hash(
            tags::BFT_PROPOSAL,
            &[
                chain_id.as_bytes(),
                &height.to_be_bytes(),
                &round.to_be_bytes(),
                &valid_round.to_be_bytes(),
                id,
            ],
        )
        .to_vec()
    }

    pub fn new(
        chain_id: &str,
        key: &SigningKey,
        height: u64,
        round: u32,
        valid_round: i32,
        payload: Vec<u8>,
    ) -> Self {
        let msg = Self::sign_bytes(chain_id, height, round, valid_round, &block_id(&payload));
        Self {
            height,
            round,
            valid_round,
            payload,
            proposer: public_key_bytes(key),
            signature: sign_tagged(key, tags::BFT_PROPOSAL, &msg),
        }
    }

    pub fn verify(&self, chain_id: &str) -> bool {
        verify_tagged(
            &self.proposer,
            tags::BFT_PROPOSAL,
            &Self::sign_bytes(
                chain_id,
                self.height,
                self.round,
                self.valid_round,
                &block_id(&self.payload),
            ),
            &self.signature,
        )
        .is_ok()
    }
}

/// Finality proof: more than 2/3 of the power signed a precommit for this block.
#[derive(Clone, PartialEq, Message)]
pub struct Commit {
    #[prost(uint64, tag = "1")]
    pub height: u64,
    #[prost(uint32, tag = "2")]
    pub round: u32,
    #[prost(bytes = "vec", tag = "3")]
    pub block_id: Vec<u8>,
    #[prost(message, repeated, tag = "4")]
    pub precommits: Vec<Vote>,
}

impl Commit {
    /// Checks the certificate against a validator set: distinct validators, valid
    /// signatures, all precommits for THIS block, this height and round, and a quorum.
    pub fn verify(&self, chain_id: &str, set: &ValidatorSet) -> bool {
        if self.block_id.len() != 32 {
            return false;
        }
        let mut seen = BTreeSet::new();
        let mut power = 0u64;
        for v in &self.precommits {
            let ok = v.vote_kind() == Some(VoteKind::Precommit)
                && v.height == self.height
                && v.round == self.round
                && v.block_id == self.block_id
                && v.verify(chain_id);
            let Some(p) = set.power_of(&v.validator) else {
                return false;
            };
            if !ok || !seen.insert(v.validator.clone()) {
                return false;
            }
            power = power.saturating_add(p);
        }
        set.is_quorum(power)
    }
}

/// Two conflicting signed votes by the SAME validator for the same height, round and
/// kind: it signed both, so it equivocated. Verifiable by anyone with the public key.
#[derive(Clone, PartialEq, Message)]
pub struct Evidence {
    #[prost(message, optional, tag = "1")]
    pub a: Option<Vote>,
    #[prost(message, optional, tag = "2")]
    pub b: Option<Vote>,
}

impl Evidence {
    pub fn verify(&self, chain_id: &str) -> bool {
        let (Some(a), Some(b)) = (&self.a, &self.b) else {
            return false;
        };
        a.validator == b.validator
            && a.kind == b.kind
            && a.height == b.height
            && a.round == b.round
            && a.block_id != b.block_id
            && a.verify(chain_id)
            && b.verify(chain_id)
    }

    pub fn validator(&self) -> Option<&[u8]> {
        self.a.as_ref().map(|v| v.validator.as_slice())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(powers: &[u64]) -> ValidatorSet {
        ValidatorSet::new(
            powers
                .iter()
                .enumerate()
                .map(|(i, p)| Validator {
                    key: vec![i as u8 + 1; 33],
                    power: *p,
                })
                .collect(),
        )
        .unwrap()
    }

    #[test]
    fn quorum_is_strictly_more_than_two_thirds_and_a_third_plus_is_strict_too() {
        let s = set(&[1, 1, 1, 1]);
        assert!(!s.is_quorum(2) && s.is_quorum(3));
        assert!(!s.is_one_third_plus(1) && s.is_one_third_plus(2));
        let s = set(&[1, 1, 1]); // total 3: 2 is exactly 2/3, NOT more than 2/3
        assert!(!s.is_quorum(2) && s.is_quorum(3));
        let s = set(&[5, 1, 1, 1]); // total 8
        assert!(!s.is_quorum(5) && s.is_quorum(6));
    }

    #[test]
    fn the_proposer_rotates_in_proportion_to_the_power() {
        let s = set(&[3, 1]);
        let mut hits = [0u32; 2];
        let mut longest = 0u32;
        let mut run = 0u32;
        let mut last = 9usize;
        for h in 0..4_000u64 {
            let p = s.proposer(h, 0);
            let i = usize::from(p.key[0] - 1);
            hits[i] += 1;
            run = if i == last { run + 1 } else { 1 };
            longest = longest.max(run);
            last = i;
        }
        // 3:1 power gives about 3:1 turns (a hash draw is proportional in the long run)
        assert!((2_850..=3_150).contains(&hits[0]), "{hits:?}");
        assert!((850..=1_150).contains(&hits[1]), "{hits:?}");
        // ...and never a long contiguous run (the reason for the hash draw)
        assert!(longest < 40, "longest run {longest}");
        // the same input gives the same proposer everywhere
        assert_eq!(s.proposer(123, 4).key, s.proposer(123, 4).key);
        // a different round gives a different proposer for the same height (view change)
        let s = set(&[1, 1, 1, 1]);
        let first = s.proposer(7, 0).key.clone();
        assert!((1..4).any(|r| s.proposer(7, r).key != first));
    }

    #[test]
    fn invalid_sets_are_refused() {
        assert_eq!(ValidatorSet::new(vec![]), Err(BftError::EmptySet));
        assert_eq!(
            ValidatorSet::new(vec![Validator {
                key: vec![1],
                power: 0
            }]),
            Err(BftError::ZeroPower)
        );
        assert_eq!(
            ValidatorSet::new(vec![
                Validator {
                    key: vec![1],
                    power: 1
                },
                Validator {
                    key: vec![1],
                    power: 1
                }
            ]),
            Err(BftError::DuplicateValidator)
        );
        assert_eq!(
            ValidatorSet::new(vec![
                Validator {
                    key: vec![1],
                    power: u64::MAX / 2
                },
                Validator {
                    key: vec![2],
                    power: u64::MAX / 2
                }
            ]),
            Err(BftError::PowerOverflow)
        );
    }
}
