//! Peer discovery (ADR-0048): each validator publishes a signed address record; nodes keep the newest
//! valid record of every validator in force and learn new addresses from each other. Only keys of the
//! validator set count, so discovery cannot add a stranger to the network.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use prost::Message;
use teko_crypto::{SigningKey, public_key_bytes, sign_tagged, tagged_hash, tags, verify_tagged};

/// A record may not claim to live longer than this (a lost key cannot pin an address forever).
pub const MAX_LIFETIME_SECS: u64 = 7 * 86_400;
const MAX_ADDR: usize = 255;
const MAX_RECORDS: usize = 256;

/// Where to reach one validator: its address for peers and for the public API, signed by its
/// consensus key. `seq` only goes up; `expires_at` is unix seconds.
#[derive(Clone, PartialEq, Message)]
pub struct PeerRecord {
    #[prost(bytes = "vec", tag = "1")]
    pub consensus_key: Vec<u8>,
    #[prost(string, tag = "2")]
    pub peer_addr: String,
    #[prost(string, tag = "3")]
    pub public_addr: String,
    #[prost(uint64, tag = "4")]
    pub seq: u64,
    #[prost(uint64, tag = "5")]
    pub expires_at: u64,
    #[prost(bytes = "vec", tag = "6")]
    pub signature: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
struct Book {
    #[prost(message, repeated, tag = "1")]
    records: Vec<PeerRecord>,
}

fn signing_bytes(chain_id: &str, r: &PeerRecord) -> [u8; 32] {
    tagged_hash(
        tags::NET_RECORD,
        &[
            chain_id.as_bytes(),
            &r.consensus_key,
            r.peer_addr.as_bytes(),
            r.public_addr.as_bytes(),
            &r.seq.to_be_bytes(),
            &r.expires_at.to_be_bytes(),
        ],
    )
}

/// `host:port` with a numeric port and a plain host (an IP or a DNS name).
pub fn valid_addr(a: &str) -> bool {
    let Some((host, port)) = a.rsplit_once(':') else {
        return false;
    };
    a.len() <= MAX_ADDR
        && !host.is_empty()
        && port.parse::<u16>().is_ok_and(|p| p != 0)
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'))
}

impl PeerRecord {
    pub fn sign(
        key: &SigningKey,
        chain_id: &str,
        peer_addr: &str,
        public_addr: &str,
        seq: u64,
        expires_at: u64,
    ) -> Self {
        let mut r = PeerRecord {
            consensus_key: public_key_bytes(key),
            peer_addr: peer_addr.to_string(),
            public_addr: public_addr.to_string(),
            seq,
            expires_at,
            signature: Vec::new(),
        };
        r.signature = sign_tagged(key, tags::NET_RECORD, &signing_bytes(chain_id, &r));
        r
    }

    pub fn verify(&self, chain_id: &str) -> bool {
        verify_tagged(
            &self.consensus_key,
            tags::NET_RECORD,
            &signing_bytes(chain_id, self),
            &self.signature,
        )
        .is_ok()
    }
}

/// What a merge did with a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeOutcome {
    Added,
    Updated,
    /// Not newer than what we hold.
    Stale,
    /// The key is not in the validator set.
    NotValidator,
    Expired,
    BadSignature,
    /// Malformed addresses, or a lifetime beyond the maximum.
    Invalid,
}

/// The newest valid record of each validator.
pub struct PeerBook {
    chain_id: String,
    records: BTreeMap<Vec<u8>, PeerRecord>,
}

impl PeerBook {
    pub fn new(chain_id: &str) -> Self {
        Self {
            chain_id: chain_id.to_string(),
            records: BTreeMap::new(),
        }
    }

    /// Considers one record. `is_validator` says whether a key belongs to the set in force.
    pub fn merge(
        &mut self,
        r: PeerRecord,
        is_validator: &dyn Fn(&[u8]) -> bool,
        now: u64,
    ) -> MergeOutcome {
        if !is_validator(&r.consensus_key) {
            return MergeOutcome::NotValidator;
        }
        if !valid_addr(&r.peer_addr)
            || (!r.public_addr.is_empty() && !valid_addr(&r.public_addr))
            || r.expires_at > now.saturating_add(MAX_LIFETIME_SECS)
        {
            return MergeOutcome::Invalid;
        }
        if r.expires_at <= now {
            return MergeOutcome::Expired;
        }
        if !r.verify(&self.chain_id) {
            return MergeOutcome::BadSignature;
        }
        match self.records.get(&r.consensus_key) {
            Some(old) if old.seq >= r.seq => MergeOutcome::Stale,
            Some(_) => {
                self.records.insert(r.consensus_key.clone(), r);
                MergeOutcome::Updated
            }
            None => {
                if self.records.len() >= MAX_RECORDS {
                    return MergeOutcome::Invalid;
                }
                self.records.insert(r.consensus_key.clone(), r);
                MergeOutcome::Added
            }
        }
    }

    pub fn get(&self, key: &[u8]) -> Option<&PeerRecord> {
        self.records.get(key)
    }

    /// The records still valid at `now`.
    pub fn live(&self, now: u64) -> Vec<PeerRecord> {
        self.records
            .values()
            .filter(|r| r.expires_at > now)
            .cloned()
            .collect()
    }

    /// Forgets the keys that left the validator set and the records that expired.
    pub fn prune(&mut self, is_validator: &dyn Fn(&[u8]) -> bool, now: u64) {
        self.records
            .retain(|k, r| is_validator(k) && r.expires_at > now);
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let book = Book {
            records: self.records.values().cloned().collect(),
        };
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, book.encode_to_vec())?;
        fs::rename(tmp, path)
    }

    /// Loads a saved book; a record that no longer verifies is dropped. A missing file is an empty book.
    pub fn load(chain_id: &str, path: &Path) -> Self {
        let mut me = Self::new(chain_id);
        if let Ok(raw) = fs::read(path)
            && let Ok(book) = Book::decode(raw.as_slice())
        {
            for r in book.records.into_iter().take(MAX_RECORDS) {
                if r.verify(chain_id) && valid_addr(&r.peer_addr) {
                    me.records.insert(r.consensus_key.clone(), r);
                }
            }
        }
        me
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use teko_crypto::generate_signing_key;

    fn key(seed: u64) -> SigningKey {
        generate_signing_key(&mut rand::rngs::StdRng::seed_from_u64(seed))
    }

    const NOW: u64 = 1_900_000_000;

    fn rec(seed: u64, addr: &str, seq: u64, ttl: u64) -> PeerRecord {
        PeerRecord::sign(&key(seed), "c", addr, "", seq, NOW + ttl)
    }

    fn only(seeds: &[u64]) -> impl Fn(&[u8]) -> bool {
        let keys: Vec<Vec<u8>> = seeds.iter().map(|s| public_key_bytes(&key(*s))).collect();
        move |k: &[u8]| keys.iter().any(|x| x == k)
    }

    #[test]
    fn only_the_newest_valid_record_of_a_validator_is_kept() {
        let mut b = PeerBook::new("c");
        let v = only(&[1, 2]);
        assert_eq!(
            b.merge(rec(1, "a.example:7001", 1, 100), &v, NOW),
            MergeOutcome::Added
        );
        assert_eq!(
            b.merge(rec(1, "b.example:7001", 1, 100), &v, NOW),
            MergeOutcome::Stale
        );
        assert_eq!(
            b.merge(rec(1, "b.example:7001", 2, 100), &v, NOW),
            MergeOutcome::Updated
        );
        let k = public_key_bytes(&key(1));
        assert_eq!(b.get(&k).unwrap().peer_addr, "b.example:7001");
        // an older record cannot take the address back
        assert_eq!(
            b.merge(rec(1, "a.example:7001", 1, 100), &v, NOW),
            MergeOutcome::Stale
        );
    }

    #[test]
    fn strangers_forgeries_expired_and_malformed_records_are_ignored() {
        let mut b = PeerBook::new("c");
        let v = only(&[1]);
        assert_eq!(
            b.merge(rec(9, "x:1", 1, 100), &v, NOW),
            MergeOutcome::NotValidator
        );
        // signed by someone else but naming validator 1
        let mut forged = rec(9, "evil:7001", 5, 100);
        forged.consensus_key = public_key_bytes(&key(1));
        assert_eq!(b.merge(forged, &v, NOW), MergeOutcome::BadSignature);
        // another chain
        let other = PeerRecord::sign(&key(1), "other", "a:7001", "", 1, NOW + 100);
        assert_eq!(b.merge(other, &v, NOW), MergeOutcome::BadSignature);
        // expired, too long-lived, bad addresses
        assert_eq!(
            b.merge(rec(1, "a:7001", 1, 0), &v, NOW),
            MergeOutcome::Expired
        );
        assert_eq!(
            b.merge(rec(1, "a:7001", 1, 8 * 86_400), &v, NOW),
            MergeOutcome::Invalid
        );
        for bad in ["nohost", ":7001", "a:0", "a:99999", "a b:7001", "a:7001\n"] {
            assert_eq!(
                b.merge(rec(1, bad, 1, 100), &v, NOW),
                MergeOutcome::Invalid,
                "{bad}"
            );
        }
        assert!(b.live(NOW).is_empty());
        // editing any signed field breaks the signature
        let mut r = rec(1, "a:7001", 1, 100);
        r.seq = 9;
        assert_eq!(b.merge(r, &v, NOW), MergeOutcome::BadSignature);
    }

    #[test]
    fn the_book_forgets_who_left_the_set_and_survives_a_restart() {
        let dir = std::env::temp_dir().join(format!("teko-book-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("peers.book");
        let mut b = PeerBook::new("c");
        let v = only(&[1, 2]);
        b.merge(rec(1, "a:7001", 1, 100), &v, NOW);
        b.merge(rec(2, "b:7002", 1, 100), &v, NOW);
        b.save(&path).unwrap();
        let again = PeerBook::load("c", &path);
        assert_eq!(again.live(NOW).len(), 2);
        // another chain's book is worth nothing; so is garbage
        assert!(PeerBook::load("other", &path).live(NOW).is_empty());
        std::fs::write(&path, b"garbage").unwrap();
        assert!(PeerBook::load("c", &path).live(NOW).is_empty());
        // key 2 leaves the set, and records expire
        let mut b = again;
        b.prune(&only(&[1]), NOW);
        assert_eq!(b.live(NOW).len(), 1);
        b.prune(&only(&[1]), NOW + 200);
        assert!(b.live(NOW + 200).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
