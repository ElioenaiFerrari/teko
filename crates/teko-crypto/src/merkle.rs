//! RFC 6962 / 9162 Merkle tree over leaf hashes, with inclusion and
//! consistency proofs. Pure functions; no I/O.

use alloc::vec;
use alloc::vec::Vec;

use crate::{tagged_hash, tags};

pub type Hash = [u8; 32];

pub fn leaf_hash(data: &[u8]) -> Hash {
    tagged_hash(tags::BULLETIN_LEAF, &[data])
}

pub fn node_hash(left: &Hash, right: &Hash) -> Hash {
    tagged_hash(tags::BULLETIN_NODE, &[left.as_slice(), right.as_slice()])
}

/// Root of the empty log.
pub fn empty_root() -> Hash {
    tagged_hash(tags::BULLETIN_NODE, &[])
}

/// Largest power of two strictly less than `n` (n >= 2).
fn split(n: usize) -> usize {
    let mut k = 1usize;
    while k * 2 < n {
        k *= 2;
    }
    k
}

/// Incremental Merkle root: O(log n) memory, O(1) amortised append, O(log n)
/// root. Equal to `root(leaves)` at every size (tested).
#[derive(Clone, Debug, Default)]
pub struct Frontier {
    /// Perfect subtrees, left to right, strictly decreasing in size.
    peaks: Vec<(Hash, u64)>,
}

impl Frontier {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, leaf: Hash) {
        let mut cur = (leaf, 1u64);
        while let Some(&(left, size)) = self.peaks.last() {
            if size != cur.1 {
                break;
            }
            self.peaks.pop();
            cur = (node_hash(&left, &cur.0), size * 2);
        }
        self.peaks.push(cur);
    }

    pub fn root(&self) -> Hash {
        let mut it = self.peaks.iter().rev();
        let Some(&(mut r, _)) = it.next() else {
            return empty_root();
        };
        for &(p, _) in it {
            r = node_hash(&p, &r);
        }
        r
    }
}

/// MTH(D[n]).
pub fn root(leaves: &[Hash]) -> Hash {
    match leaves {
        [] => empty_root(),
        [one] => *one,
        _ => {
            let k = split(leaves.len());
            node_hash(&root(&leaves[..k]), &root(&leaves[k..]))
        }
    }
}

/// PATH(m, D[n]): audit path for leaf `index`.
pub fn inclusion_proof(leaves: &[Hash], index: usize) -> Option<Vec<Hash>> {
    if index >= leaves.len() {
        return None;
    }
    fn path(m: usize, d: &[Hash]) -> Vec<Hash> {
        if d.len() == 1 {
            return Vec::new();
        }
        let k = split(d.len());
        if m < k {
            let mut p = path(m, &d[..k]);
            p.push(root(&d[k..]));
            p
        } else {
            let mut p = path(m - k, &d[k..]);
            p.push(root(&d[..k]));
            p
        }
    }
    Some(path(index, leaves))
}

/// RFC 9162 §2.1.3.2.
pub fn verify_inclusion(
    leaf: &Hash,
    index: u64,
    size: u64,
    proof: &[Hash],
    expected_root: &Hash,
) -> bool {
    if index >= size {
        return false;
    }
    let (mut f, mut s) = (index, size - 1);
    let mut r = *leaf;
    for p in proof {
        if s == 0 {
            return false;
        }
        if f & 1 == 1 || f == s {
            r = node_hash(p, &r);
            if f & 1 == 0 {
                while f & 1 == 0 && f != 0 {
                    f >>= 1;
                    s >>= 1;
                }
            }
        } else {
            r = node_hash(&r, p);
        }
        f >>= 1;
        s >>= 1;
    }
    s == 0 && &r == expected_root
}

/// PROOF(m, D[n]) = SUBPROOF(m, D[n], true).
pub fn consistency_proof(leaves: &[Hash], old_size: usize) -> Option<Vec<Hash>> {
    let n = leaves.len();
    if old_size > n {
        return None;
    }
    if old_size == 0 || old_size == n {
        return Some(Vec::new());
    }
    fn sub(m: usize, d: &[Hash], complete: bool) -> Vec<Hash> {
        let n = d.len();
        if m == n {
            return if complete { Vec::new() } else { vec![root(d)] };
        }
        let k = split(n);
        if m <= k {
            let mut p = sub(m, &d[..k], complete);
            p.push(root(&d[k..]));
            p
        } else {
            let mut p = sub(m - k, &d[k..], false);
            p.push(root(&d[..k]));
            p
        }
    }
    Some(sub(old_size, leaves, true))
}

/// RFC 9162 §2.1.4.2: the log of `new_size` entries with root `new_root`
/// extends the log of `old_size` entries with root `old_root`.
pub fn verify_consistency(
    old_size: u64,
    new_size: u64,
    old_root: &Hash,
    new_root: &Hash,
    proof: &[Hash],
) -> bool {
    if old_size > new_size {
        return false;
    }
    if old_size == new_size {
        return proof.is_empty() && old_root == new_root;
    }
    if old_size == 0 {
        // The empty log is a prefix of everything.
        return proof.is_empty() && old_root == &empty_root();
    }
    let mut path: Vec<Hash> = Vec::with_capacity(proof.len() + 1);
    if old_size.is_power_of_two() {
        path.push(*old_root);
    }
    path.extend_from_slice(proof);
    if path.is_empty() {
        return false;
    }
    let (mut f, mut s) = (old_size - 1, new_size - 1);
    while f & 1 == 1 {
        f >>= 1;
        s >>= 1;
    }
    let (mut fr, mut sr) = (path[0], path[0]);
    for c in &path[1..] {
        if s == 0 {
            return false;
        }
        if f & 1 == 1 || f == s {
            fr = node_hash(c, &fr);
            sr = node_hash(c, &sr);
            if f & 1 == 0 {
                while f & 1 == 0 && f != 0 {
                    f >>= 1;
                    s >>= 1;
                }
            }
        } else {
            sr = node_hash(&sr, c);
        }
        f >>= 1;
        s >>= 1;
    }
    s == 0 && &fr == old_root && &sr == new_root
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use proptest::prelude::*;

    fn leaves(n: usize) -> Vec<Hash> {
        (0..n)
            .map(|i| leaf_hash(&(i as u64).to_be_bytes()))
            .collect()
    }

    #[test]
    fn every_leaf_of_every_size_has_a_valid_inclusion_proof() {
        for n in 1..=40usize {
            let l = leaves(n);
            let r = root(&l);
            for i in 0..n {
                let p = inclusion_proof(&l, i).unwrap();
                assert!(
                    verify_inclusion(&l[i], i as u64, n as u64, &p, &r),
                    "n={n} i={i}"
                );
            }
        }
    }

    #[test]
    fn every_prefix_has_a_valid_consistency_proof() {
        for n in 1..=40usize {
            let l = leaves(n);
            let new_root = root(&l);
            for m in 1..=n {
                let old_root = root(&l[..m]);
                let p = consistency_proof(&l, m).unwrap();
                assert!(
                    verify_consistency(m as u64, n as u64, &old_root, &new_root, &p),
                    "m={m} n={n}"
                );
            }
        }
    }

    #[test]
    fn proofs_fail_when_anything_is_wrong() {
        let l = leaves(13);
        let r = root(&l);
        let p = inclusion_proof(&l, 5).unwrap();
        assert!(verify_inclusion(&l[5], 5, 13, &p, &r));
        assert!(!verify_inclusion(&l[6], 5, 13, &p, &r), "wrong leaf");
        assert!(!verify_inclusion(&l[5], 4, 13, &p, &r), "wrong index");
        // NOTE: a Merkle root does not commit to the tree size, so a proof can verify
        // under a nearby size when the path shape is identical. The size is bound by the
        // operator's SIGNED HEAD (see board::sign_head), never by the root alone.
        assert!(
            !verify_inclusion(&l[5], 5, 5, &p, &r),
            "index must be < size"
        );
        assert!(
            !verify_inclusion(&l[5], 13, 13, &p, &r),
            "index out of range"
        );
        let mut bad = p.clone();
        bad[0][0] ^= 1;
        assert!(!verify_inclusion(&l[5], 5, 13, &bad, &r), "tampered path");
        assert!(!verify_inclusion(&l[5], 5, 13, &p[1..], &r), "short path");

        let c = consistency_proof(&l, 6).unwrap();
        let old = root(&l[..6]);
        assert!(verify_consistency(6, 13, &old, &r, &c));
        let mut rewritten = l.clone();
        rewritten[2] = leaf_hash(b"history rewritten");
        let forged_old = root(&rewritten[..6]);
        assert!(
            !verify_consistency(6, 13, &forged_old, &r, &c),
            "rewritten history"
        );
        assert!(!verify_consistency(7, 13, &old, &r, &c), "wrong old size");
        assert!(!verify_consistency(13, 6, &r, &old, &c), "shrinking");
    }

    #[test]
    fn the_incremental_frontier_equals_the_recursive_root_at_every_size() {
        let mut f = Frontier::new();
        assert_eq!(f.root(), empty_root());
        let l = leaves(200);
        for (i, leaf) in l.iter().enumerate() {
            f.push(*leaf);
            assert_eq!(f.root(), root(&l[..=i]), "size {}", i + 1);
        }
    }

    #[test]
    fn empty_and_single_edge_cases() {
        assert_eq!(root(&[]), empty_root());
        let one = leaves(1);
        assert_eq!(root(&one), one[0]);
        assert!(inclusion_proof(&[], 0).is_none());
        assert!(verify_consistency(
            0,
            5,
            &empty_root(),
            &root(&leaves(5)),
            &[]
        ));
        assert!(!verify_consistency(
            0,
            5,
            &leaf_hash(b"x"),
            &root(&leaves(5)),
            &[]
        ));
        assert!(verify_consistency(
            5,
            5,
            &root(&leaves(5)),
            &root(&leaves(5)),
            &[]
        ));
    }

    proptest! {
        #[test]
        fn random_proof_bytes_never_panic_or_falsely_verify(n in 1usize..30, i in 0usize..30, junk in proptest::collection::vec(any::<[u8;32]>(), 0..8)) {
            let l = leaves(n);
            let r = root(&l);
            let i = i % n;
            if junk != inclusion_proof(&l, i).unwrap() {
                prop_assert!(!verify_inclusion(&l[i], i as u64, n as u64, &junk, &r));
            }
        }

        /// The log is append-only: any old head stays provably a prefix of any newer one.
        #[test]
        fn growth_is_always_provable(a in 1usize..30, b in 0usize..30) {
            let l = leaves(a + b);
            let old = root(&l[..a]);
            let new = root(&l);
            let p = consistency_proof(&l, a).unwrap();
            prop_assert!(verify_consistency(a as u64, (a + b) as u64, &old, &new, &p));
        }
    }
}
