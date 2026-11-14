//! The RESULT of an encrypted contest: each validator publishes its decryption
//! share of every option's homomorphic sum, with a proof that it used its real key
//! share. `threshold` valid shares reveal the totals; no ballot is ever opened.

use alloc::string::String;
use alloc::vec::Vec;

use prost::Message;
use rand_core::{CryptoRng, Rng};
use teko_ballot_crypto::{
    Ciphertext, DecryptProof, Point, Scalar, decrypt_share, point_to_bytes, prove_decryption,
};
use teko_crypto::{tagged_hash, tags};

/// One validator's shares for ONE contest, all options at once.
#[derive(Clone, PartialEq, Message)]
pub struct DecryptionShares {
    #[prost(string, tag = "1")]
    pub election_id: String,
    #[prost(string, tag = "2")]
    pub contest_id: String,
    /// The exact contest version these shares were computed for.
    #[prost(bytes = "vec", tag = "3")]
    pub contest_hash: Vec<u8>,
    /// 1-based validator index in the key ceremony.
    #[prost(uint32, tag = "4")]
    pub validator_index: u32,
    /// `d_i * C1` per option, SEC1-compressed.
    #[prost(bytes = "vec", repeated, tag = "5")]
    pub shares: Vec<Vec<u8>>,
    /// Chaum-Pedersen proof per option (64 bytes each).
    #[prost(bytes = "vec", repeated, tag = "6")]
    pub proofs: Vec<Vec<u8>>,
}

/// What a decryption proof is bound to (so it cannot be moved to another option,
/// contest version, election or validator).
pub fn decrypt_ctx(
    election_id: &str,
    contest_id: &str,
    contest_hash: &[u8],
    option_position: usize,
    validator_index: u32,
) -> Vec<u8> {
    tagged_hash(
        tags::DECRYPT_PROOF,
        &[
            election_id.as_bytes(),
            contest_id.as_bytes(),
            contest_hash,
            &(option_position as u32).to_be_bytes(),
            &validator_index.to_be_bytes(),
        ],
    )
    .to_vec()
}

/// Validator side: decrypt shares of the final sums, each with its proof.
pub fn build_shares<R: Rng + CryptoRng>(
    election_id: &str,
    contest_id: &str,
    contest_hash: &[u8],
    sums: &[Ciphertext],
    validator_index: u32,
    secret_share: &Scalar,
    rng: &mut R,
) -> DecryptionShares {
    let mut shares = Vec::with_capacity(sums.len());
    let mut proofs = Vec::with_capacity(sums.len());
    for (pos, ct) in sums.iter().enumerate() {
        let d: Point = decrypt_share(secret_share, ct);
        shares.push(point_to_bytes(&d).to_vec());
        let ctx = decrypt_ctx(election_id, contest_id, contest_hash, pos, validator_index);
        let p: DecryptProof = prove_decryption(secret_share, ct, &ctx, rng);
        proofs.push(p.to_bytes().to_vec());
    }
    DecryptionShares {
        election_id: election_id.into(),
        contest_id: contest_id.into(),
        contest_hash: contest_hash.to_vec(),
        validator_index,
        shares,
        proofs,
    }
}
