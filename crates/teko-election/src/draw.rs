//! The verifiable DRAW that breaks a tie (ADR-0043): which options are tied, the public input of the
//! draw, the permutation a seed makes of them, and the board entry through which each institution
//! publishes its share of the draw. The cryptography (threshold VRF) is in `teko-ballot-crypto::draw`.

use alloc::string::String;
use alloc::vec::Vec;

use prost::Message;
use teko_crypto::{tagged_hash, tags};

/// What the draw decides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawGoal {
    /// The first of the drawn order WINS (a tie at the top).
    Winner,
    /// The first `take` of the drawn order fill the open finalist slots of a runoff.
    Finalists,
    /// The first `take` of the drawn order fill the seats left open at the cut of a multi-seat
    /// contest (ADR-0044).
    Seats,
}

/// A tie the rule cannot break: the tied options (ascending ballot index), how many of them the
/// draw picks, and what for. Derived ONLY from the verified totals.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrawNeed {
    pub tied: Vec<u32>,
    pub take: u32,
    pub goal: DrawGoal,
    /// The votes every tied option has (part of the input, so a draw is bound to the real tie).
    pub votes: u64,
}

/// The public input of the draw: the same for everybody, fixed by the contest and its tie.
pub fn draw_input(
    election_id: &str,
    contest_id: &str,
    contest_hash: &[u8],
    need: &DrawNeed,
) -> Vec<u8> {
    let mut tied = Vec::with_capacity(need.tied.len() * 4);
    for t in &need.tied {
        tied.extend_from_slice(&t.to_be_bytes());
    }
    tagged_hash(
        tags::DRAW_POINT,
        &[
            election_id.as_bytes(),
            contest_id.as_bytes(),
            contest_hash,
            &tied,
            &need.take.to_be_bytes(),
            &[need.goal as u8],
            &need.votes.to_be_bytes(),
        ],
    )
    .to_vec()
}

/// The tied options in the order the seed draws them (Fisher-Yates over a hash stream).
pub fn draw_order(seed: &[u8; 32], tied: &[u32]) -> Vec<u32> {
    let mut v: Vec<u32> = tied.to_vec();
    let n = v.len();
    for i in 0..n.saturating_sub(1) {
        let h = tagged_hash(tags::DRAW_SEED, &[seed, &(i as u32).to_be_bytes()]);
        let mut x = [0u8; 8];
        x.copy_from_slice(&h[..8]);
        // 64 random bits over a range of a few options: the modulo bias is below 2^-56
        let j = i + (u64::from_be_bytes(x) % (n - i) as u64) as usize;
        v.swap(i, j);
    }
    v
}

/// One institution's share of the draw of one contest tie, as published on the board
/// (`EntryKind::DRAW`). Anyone can verify it against the key ceremony record.
#[derive(Clone, PartialEq, Message)]
pub struct DrawShareEntry {
    #[prost(string, tag = "1")]
    pub election_id: String,
    #[prost(string, tag = "2")]
    pub contest_id: String,
    #[prost(bytes = "vec", tag = "3")]
    pub contest_hash: Vec<u8>,
    /// The draw input (`draw_input`): the share is only valid for THIS tie.
    #[prost(bytes = "vec", tag = "4")]
    pub input: Vec<u8>,
    /// 1-based validator index in the key ceremony.
    #[prost(uint32, tag = "5")]
    pub validator_index: u32,
    /// `d_i * H`, SEC1-compressed.
    #[prost(bytes = "vec", tag = "6")]
    pub share: Vec<u8>,
    /// Chaum-Pedersen proof (64 bytes).
    #[prost(bytes = "vec", tag = "7")]
    pub proof: Vec<u8>,
}

/// What a share's proof is bound to (election, contest version, tie input and validator).
pub fn draw_ctx(
    election_id: &str,
    contest_id: &str,
    input: &[u8],
    validator_index: u32,
) -> Vec<u8> {
    tagged_hash(
        tags::DRAW_SEED,
        &[
            election_id.as_bytes(),
            contest_id.as_bytes(),
            input,
            &validator_index.to_be_bytes(),
        ],
    )
    .to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_input_is_bound_to_the_contest_and_to_the_exact_tie() {
        let need = DrawNeed {
            tied: alloc::vec![1, 2],
            take: 1,
            goal: DrawGoal::Finalists,
            votes: 30,
        };
        let base = draw_input("e", "c", &[1; 32], &need);
        assert_eq!(base, draw_input("e", "c", &[1; 32], &need));
        assert_ne!(base, draw_input("e2", "c", &[1; 32], &need));
        assert_ne!(base, draw_input("e", "c2", &[1; 32], &need));
        assert_ne!(base, draw_input("e", "c", &[2; 32], &need));
        for other in [
            DrawNeed {
                tied: alloc::vec![1, 3],
                ..need.clone()
            },
            DrawNeed {
                take: 2,
                ..need.clone()
            },
            DrawNeed {
                goal: DrawGoal::Winner,
                ..need.clone()
            },
            DrawNeed {
                votes: 31,
                ..need.clone()
            },
        ] {
            assert_ne!(base, draw_input("e", "c", &[1; 32], &other));
        }
    }

    #[test]
    fn the_order_is_a_permutation_decided_only_by_the_seed() {
        let tied = [3u32, 5, 8, 9];
        let a = draw_order(&[1; 32], &tied);
        assert_eq!(a, draw_order(&[1; 32], &tied), "deterministic");
        let mut sorted = a.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, tied, "a permutation of the tied options");
        // different seeds reach different orders and, over many seeds, every option can come first
        let mut firsts = alloc::collections::BTreeSet::new();
        for s in 0..200u8 {
            firsts.insert(draw_order(&[s; 32], &tied)[0]);
        }
        assert_eq!(firsts.len(), tied.len());
        assert_eq!(draw_order(&[1; 32], &[7]), [7]);
        assert!(draw_order(&[1; 32], &[]).is_empty());
    }
}
