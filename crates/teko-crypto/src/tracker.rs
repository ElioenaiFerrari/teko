//! The TRACKER of a ballot (ADR-0041): a short code of its board leaf, to compare on the device, in
//! the Suite and in the explorer. The leaf is public (the board already holds the ciphertext), so the
//! tracker proves WHICH ballot is yours and nothing about the vote.

use alloc::string::String;

const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// `TK-ABCD-EFGH`: the first 5 bytes of a leaf hash in base32 (40 bits).
pub fn tracker(leaf_hash: &[u8; 32]) -> String {
    let b = &leaf_hash[..5];
    let mut bits: u64 = 0;
    for x in b {
        bits = (bits << 8) | u64::from(*x);
    }
    let mut out = String::from("TK-");
    for i in 0..8 {
        let v = ((bits >> (35 - 5 * i)) & 31) as usize;
        if i == 4 {
            out.push('-');
        }
        out.push(ALPHABET[v] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_is_short_stable_and_follows_the_leaf() {
        let a = tracker(&[0u8; 32]);
        assert_eq!(a, "TK-AAAA-AAAA");
        let mut h = [0u8; 32];
        h[0] = 0xFF;
        let t = tracker(&h);
        assert_eq!(t.len(), 12);
        assert_ne!(t, a);
        assert_eq!(t, tracker(&h));
        // only the first 5 bytes count
        h[20] = 7;
        assert_eq!(t, tracker(&h));
        assert!(
            t.chars()
                .all(|c| c == '-' || c.is_ascii_uppercase() || c.is_ascii_digit())
        );
    }
}
