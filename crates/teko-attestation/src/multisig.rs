//! Generic M-of-N verification shared by firmware manifests and election
//! governance: M DISTINCT, KNOWN signers with valid signatures.

use alloc::vec::Vec;

use teko_crypto::verify_tagged;

use crate::firmware::SignerSignature;

/// Number of distinct keys in `signers` with a valid signature over `msg`
/// (domain-separated by `tag`). Duplicates and unknown keys count zero.
pub fn count_distinct_valid(
    signers: &[Vec<u8>],
    tag: &str,
    msg: &[u8],
    signatures: &[SignerSignature],
) -> usize {
    let mut counted: Vec<&[u8]> = Vec::new();
    for s in signatures {
        let known = signers.iter().any(|k| k == &s.signer);
        if known
            && !counted.contains(&s.signer.as_slice())
            && verify_tagged(&s.signer, tag, msg, &s.signature).is_ok()
        {
            counted.push(&s.signer);
        }
    }
    counted.len()
}
