use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use teko_crypto::{SigningKey, public_key_bytes, sign_tagged, tags, verify_tagged};

use crate::error::BoardError;
use crate::merkle::{self, Hash};

const MAX_PAYLOAD: usize = 1024 * 1024;

/// What an entry is. Stored as one byte inside the leaf, so the kind is
/// covered by the Merkle hash.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntryKind(pub u8);

impl EntryKind {
    pub const MANIFEST: EntryKind = EntryKind(1);
    pub const TRANSITION: EntryKind = EntryKind(2);
    pub const REGISTRATION: EntryKind = EntryKind(3);
    pub const BALLOT: EntryKind = EntryKind(4);
    pub const RESULT: EntryKind = EntryKind(5);
    /// Key ceremony record (public commitments of the election key).
    pub const DKG: EntryKind = EntryKind(6);
    /// One institution's signed RESULT of a contest (ADR-0040); the tally ignores it.
    pub const OUTCOME: EntryKind = EntryKind(7);
    /// One institution's share of the verifiable draw that breaks a tie (ADR-0043); the tally reads it
    /// only to learn the seed.
    pub const DRAW: EntryKind = EntryKind(8);
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub kind: EntryKind,
    pub payload: Vec<u8>,
}

impl Entry {
    pub fn new(kind: EntryKind, payload: Vec<u8>) -> Self {
        Self { kind, payload }
    }

    fn leaf_data(&self) -> Vec<u8> {
        let mut d = Vec::with_capacity(1 + self.payload.len());
        d.push(self.kind.0);
        d.extend_from_slice(&self.payload);
        d
    }

    pub fn leaf_hash(&self) -> Hash {
        merkle::leaf_hash(&self.leaf_data())
    }
}

/// Returned by `append`: where the entry landed and the new head.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Receipt {
    pub index: u64,
    pub size: u64,
    pub root: Hash,
}

pub trait BulletinBoard {
    /// Appends an entry. There is NO way to modify or delete one.
    fn append(&mut self, entry: Entry) -> Result<Receipt, BoardError>;
    fn len(&self) -> u64;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn entry(&self, index: u64) -> Option<&Entry>;
    fn root(&self) -> Hash;
    fn inclusion_proof(&self, index: u64) -> Result<Vec<Hash>, BoardError>;
    /// Proof that the first `old_size` entries are unchanged in the current log.
    fn consistency_proof(&self, old_size: u64) -> Result<Vec<Hash>, BoardError>;
}

/// In-memory log.
#[derive(Default)]
pub struct MemoryBoard {
    entries: Vec<Entry>,
    leaves: Vec<Hash>,
    frontier: merkle::Frontier,
}

impl MemoryBoard {
    pub fn new() -> Self {
        Self::default()
    }

    fn push(&mut self, entry: Entry) -> Result<Receipt, BoardError> {
        if entry.payload.len() > MAX_PAYLOAD {
            return Err(BoardError::TooLarge);
        }
        let leaf = entry.leaf_hash();
        self.leaves.push(leaf);
        self.frontier.push(leaf);
        self.entries.push(entry);
        Ok(Receipt {
            index: self.len() - 1,
            size: self.len(),
            root: self.root(),
        })
    }
}

impl BulletinBoard for MemoryBoard {
    fn append(&mut self, entry: Entry) -> Result<Receipt, BoardError> {
        self.push(entry)
    }

    fn len(&self) -> u64 {
        self.entries.len() as u64
    }

    fn entry(&self, index: u64) -> Option<&Entry> {
        self.entries.get(usize::try_from(index).ok()?)
    }

    fn root(&self) -> Hash {
        self.frontier.root()
    }

    fn inclusion_proof(&self, index: u64) -> Result<Vec<Hash>, BoardError> {
        let i = usize::try_from(index).map_err(|_| BoardError::OutOfRange)?;
        merkle::inclusion_proof(&self.leaves, i).ok_or(BoardError::OutOfRange)
    }

    fn consistency_proof(&self, old_size: u64) -> Result<Vec<Hash>, BoardError> {
        let m = usize::try_from(old_size).map_err(|_| BoardError::BadSize)?;
        merkle::consistency_proof(&self.leaves, m).ok_or(BoardError::BadSize)
    }
}

/// File-backed log: `[u32 BE len][kind][payload]` per entry, fsync on append.
/// A corrupt or truncated file is REFUSED on open (never silently repaired):
/// a log that lost its tail must be investigated, not hidden.
pub struct FileBoard {
    path: PathBuf,
    inner: MemoryBoard,
}

impl FileBoard {
    /// STRICT open: a truncated or corrupt file is refused (a log that lost its
    /// tail must be investigated). Use this for anything that WRITES.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, BoardError> {
        let (board, partial) = Self::load(path.as_ref(), false)?;
        debug_assert!(!partial);
        Ok(board)
    }

    /// LIVE-READER open: an incomplete LAST record (a writer is mid-append) is
    /// ignored and reported (`true`) instead of failing; any other corruption is
    /// still an error. Never use it to write.
    pub fn open_tolerant(path: impl AsRef<Path>) -> Result<(Self, bool), BoardError> {
        Self::load(path.as_ref(), true)
    }

    fn load(path: &Path, tolerate_partial_tail: bool) -> Result<(Self, bool), BoardError> {
        let path = path.to_path_buf();
        let mut inner = MemoryBoard::new();
        let mut partial = false;
        if path.exists() {
            let mut bytes = Vec::new();
            File::open(&path)?.read_to_end(&mut bytes)?;
            let mut pos = 0usize;
            while pos < bytes.len() {
                let Some(hdr) = bytes.get(pos..pos + 4) else {
                    if tolerate_partial_tail {
                        partial = true;
                        break;
                    }
                    return Err(BoardError::Corrupt("truncated length"));
                };
                let len = u32::from_be_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]) as usize;
                if len == 0 || len > MAX_PAYLOAD + 1 {
                    return Err(BoardError::Corrupt("bad entry length"));
                }
                let Some(body) = bytes.get(pos + 4..pos + 4 + len) else {
                    if tolerate_partial_tail {
                        partial = true;
                        break;
                    }
                    return Err(BoardError::Corrupt("truncated entry"));
                };
                inner.push(Entry::new(EntryKind(body[0]), body[1..].to_vec()))?;
                pos += 4 + len;
            }
        } else if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        Ok((Self { path, inner }, partial))
    }

    /// Appends many entries with ONE write and ONE `fsync`.
    pub fn append_many(&mut self, entries: Vec<Entry>) -> Result<Receipt, BoardError> {
        if entries.iter().any(|e| e.payload.len() > MAX_PAYLOAD) {
            return Err(BoardError::TooLarge);
        }
        let mut buf = Vec::new();
        for e in &entries {
            let data = e.leaf_data();
            buf.extend_from_slice(&(data.len() as u32).to_be_bytes());
            buf.extend_from_slice(&data);
        }
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        f.write_all(&buf)?;
        f.sync_all()?;
        let mut last = Receipt {
            index: 0,
            size: self.len(),
            root: self.root(),
        };
        for e in entries {
            last = self.inner.push(e)?;
        }
        Ok(last)
    }
}

impl BulletinBoard for FileBoard {
    fn append(&mut self, entry: Entry) -> Result<Receipt, BoardError> {
        if entry.payload.len() > MAX_PAYLOAD {
            return Err(BoardError::TooLarge);
        }
        let data = entry.leaf_data();
        let mut rec = Vec::with_capacity(4 + data.len());
        rec.extend_from_slice(&(data.len() as u32).to_be_bytes());
        rec.extend_from_slice(&data);
        // Persist first, then expose: a crash can't leave memory ahead of disk.
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        f.write_all(&rec)?;
        f.sync_all()?;
        self.inner.push(entry)
    }

    fn len(&self) -> u64 {
        self.inner.len()
    }

    fn entry(&self, index: u64) -> Option<&Entry> {
        self.inner.entry(index)
    }

    fn root(&self) -> Hash {
        self.inner.root()
    }

    fn inclusion_proof(&self, index: u64) -> Result<Vec<Hash>, BoardError> {
        self.inner.inclusion_proof(index)
    }

    fn consistency_proof(&self, old_size: u64) -> Result<Vec<Hash>, BoardError> {
        self.inner.consistency_proof(old_size)
    }
}

// ───────────────────────── signed heads & client-side verification ─────────────────────────

/// The operator's signature over `(size, root)`. Publish it widely: if two
/// different heads for the same size ever exist, the operator is caught.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedHead {
    pub size: u64,
    pub root: Hash,
    pub operator_key: Vec<u8>,
    pub signature: Vec<u8>,
}

fn head_message(size: u64, root: &Hash) -> Vec<u8> {
    let mut m = Vec::with_capacity(8 + 32);
    m.extend_from_slice(&size.to_be_bytes());
    m.extend_from_slice(root);
    m
}

pub fn sign_head(operator: &SigningKey, size: u64, root: Hash) -> SignedHead {
    SignedHead {
        size,
        root,
        operator_key: public_key_bytes(operator),
        signature: sign_tagged(operator, tags::BULLETIN_HEAD, &head_message(size, &root)),
    }
}

/// Checks the head was signed by `expected_operator`.
pub fn verify_head(expected_operator: &[u8], head: &SignedHead) -> bool {
    head.operator_key == expected_operator
        && verify_tagged(
            expected_operator,
            tags::BULLETIN_HEAD,
            &head_message(head.size, &head.root),
            &head.signature,
        )
        .is_ok()
}

/// What a reader runs: "this exact entry is at `index` of the log with this head."
pub fn verify_entry_inclusion(
    entry: &Entry,
    index: u64,
    head: &SignedHead,
    proof: &[Hash],
) -> bool {
    merkle::verify_inclusion(&entry.leaf_hash(), index, head.size, proof, &head.root)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::merkle::verify_consistency;
    use rand::SeedableRng;
    use rand::rngs::StdRng;
    use teko_crypto::generate_signing_key;

    fn entry(i: u32) -> Entry {
        Entry::new(EntryKind::BALLOT, i.to_be_bytes().to_vec())
    }

    fn tmp(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "teko-board-{tag}-{}-{}.log",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ))
    }

    #[test]
    fn append_gives_receipts_and_provable_inclusion() {
        let mut b = MemoryBoard::new();
        let key = generate_signing_key(&mut StdRng::seed_from_u64(1));
        let mut last = None;
        for i in 0..20 {
            last = Some(b.append(entry(i)).unwrap());
        }
        let rc = last.unwrap();
        assert_eq!((rc.index, rc.size, b.len()), (19, 20, 20));
        let head = sign_head(&key, b.len(), b.root());
        assert!(verify_head(&public_key_bytes(&key), &head));
        for i in 0..20u64 {
            let p = b.inclusion_proof(i).unwrap();
            assert!(verify_entry_inclusion(b.entry(i).unwrap(), i, &head, &p));
        }
        // A different entry (even same payload, other kind) does not verify at that slot.
        let p = b.inclusion_proof(3).unwrap();
        let other_kind = Entry::new(EntryKind::RESULT, 3u32.to_be_bytes().to_vec());
        assert!(!verify_entry_inclusion(&other_kind, 3, &head, &p));
        assert!(!verify_entry_inclusion(&entry(99), 3, &head, &p));
        assert!(b.inclusion_proof(20).is_err());
    }

    #[test]
    fn head_signature_binds_size_root_and_operator() {
        let mut rng = StdRng::seed_from_u64(2);
        let (op, other) = (
            generate_signing_key(&mut rng),
            generate_signing_key(&mut rng),
        );
        let mut b = MemoryBoard::new();
        b.append(entry(1)).unwrap();
        let head = sign_head(&op, b.len(), b.root());
        assert!(verify_head(&public_key_bytes(&op), &head));
        assert!(
            !verify_head(&public_key_bytes(&other), &head),
            "wrong operator"
        );
        let mut forged = head.clone();
        forged.size += 1;
        assert!(
            !verify_head(&public_key_bytes(&op), &forged),
            "size tampered"
        );
        let mut forged = head.clone();
        forged.root[0] ^= 1;
        assert!(
            !verify_head(&public_key_bytes(&op), &forged),
            "root tampered"
        );
    }

    #[test]
    fn old_heads_stay_provable_as_the_log_grows_and_rewrites_are_caught() {
        let mut b = MemoryBoard::new();
        for i in 0..7 {
            b.append(entry(i)).unwrap();
        }
        let (old_size, old_root) = (b.len(), b.root());
        for i in 7..25 {
            b.append(entry(i)).unwrap();
        }
        let p = b.consistency_proof(old_size).unwrap();
        assert!(verify_consistency(
            old_size,
            b.len(),
            &old_root,
            &b.root(),
            &p
        ));

        // An operator that rewrote entry 3 can no longer prove consistency with the old head.
        let mut evil = MemoryBoard::new();
        for i in 0..25 {
            evil.append(if i == 3 { entry(999) } else { entry(i) })
                .unwrap();
        }
        let p = evil.consistency_proof(old_size).unwrap();
        assert!(!verify_consistency(
            old_size,
            evil.len(),
            &old_root,
            &evil.root(),
            &p
        ));
        assert!(b.consistency_proof(26).is_err());
    }

    #[test]
    fn oversized_payload_is_refused() {
        let mut b = MemoryBoard::new();
        let big = Entry::new(EntryKind::BALLOT, vec![0; MAX_PAYLOAD + 1]);
        assert!(matches!(b.append(big), Err(BoardError::TooLarge)));
        assert_eq!(b.len(), 0);
    }

    #[test]
    fn file_board_survives_restart_and_keeps_old_heads_provable() {
        let path = tmp("restart");
        let (old_size, old_root);
        {
            let mut b = FileBoard::open(&path).unwrap();
            for i in 0..9 {
                b.append(entry(i)).unwrap();
            }
            old_size = b.len();
            old_root = b.root();
        }
        let mut b = FileBoard::open(&path).unwrap();
        assert_eq!((b.len(), b.root()), (old_size, old_root));
        for i in 9..15 {
            b.append(entry(i)).unwrap();
        }
        drop(b);
        let b = FileBoard::open(&path).unwrap();
        assert_eq!(b.len(), 15);
        let p = b.consistency_proof(old_size).unwrap();
        assert!(verify_consistency(old_size, 15, &old_root, &b.root(), &p));
        assert_eq!(b.entry(4).unwrap(), &entry(4));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn truncated_or_corrupt_log_files_are_refused_not_repaired() {
        let path = tmp("corrupt");
        {
            let mut b = FileBoard::open(&path).unwrap();
            for i in 0..4 {
                b.append(entry(i)).unwrap();
            }
        }
        let good = fs::read(&path).unwrap();
        // chop the tail
        fs::write(&path, &good[..good.len() - 3]).unwrap();
        assert!(matches!(
            FileBoard::open(&path),
            Err(BoardError::Corrupt(_))
        ));
        // garbage length
        let mut bad = good.clone();
        bad[0..4].copy_from_slice(&0u32.to_be_bytes());
        fs::write(&path, &bad).unwrap();
        assert!(matches!(
            FileBoard::open(&path),
            Err(BoardError::Corrupt(_))
        ));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn a_live_reader_ignores_an_in_progress_tail_but_a_writer_still_refuses_it() {
        let path = tmp("tail");
        {
            let mut b = FileBoard::open(&path).unwrap();
            b.append_many((0..5).map(entry).collect()).unwrap();
        }
        let good = fs::read(&path).unwrap();
        fs::write(&path, &good[..good.len() - 2]).unwrap(); // a writer is mid-record
        let (r, partial) = FileBoard::open_tolerant(&path).unwrap();
        assert!(partial);
        assert_eq!(r.len(), 4, "the complete records are readable");
        assert!(
            matches!(FileBoard::open(&path), Err(BoardError::Corrupt(_))),
            "writers stay strict"
        );
        // damage in the MIDDLE is never tolerated
        let mut bad = good.clone();
        bad[0..4].copy_from_slice(&0u32.to_be_bytes());
        fs::write(&path, &bad).unwrap();
        assert!(matches!(
            FileBoard::open_tolerant(&path),
            Err(BoardError::Corrupt(_))
        ));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn append_many_equals_appending_one_by_one() {
        let (pa, pb) = (tmp("many"), tmp("one"));
        let mut a = FileBoard::open(&pa).unwrap();
        let mut b = FileBoard::open(&pb).unwrap();
        a.append_many((0..13).map(entry).collect()).unwrap();
        for i in 0..13 {
            b.append(entry(i)).unwrap();
        }
        assert_eq!((a.len(), a.root()), (b.len(), b.root()));
        assert_eq!(FileBoard::open(&pa).unwrap().root(), a.root());
        let _ = (fs::remove_file(&pa), fs::remove_file(&pb));
    }

    #[test]
    fn flipping_a_stored_byte_changes_the_root_so_pinned_heads_catch_it() {
        let path = tmp("flip");
        let head_root;
        {
            let mut b = FileBoard::open(&path).unwrap();
            for i in 0..6 {
                b.append(entry(i)).unwrap();
            }
            head_root = b.root();
        }
        let mut bytes = fs::read(&path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        fs::write(&path, bytes).unwrap();
        let b = FileBoard::open(&path).unwrap();
        assert_ne!(
            b.root(),
            head_root,
            "tampering is visible against a pinned head"
        );
        let _ = fs::remove_file(&path);
    }
}
