//! Where the bulletin log lives: a local FILE (single operator, simple) or the
//! test NETWORK (`<dir>/net`, federated PoA). In network mode reads come from a
//! replica that this process VERIFIES block by block; it never trusts a node.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use prost::Message as _;
use teko_bulletin::{
    BoardError, BulletinBoard, Entry, EntryKind, FileBoard, Hash, MemoryBoard, Receipt,
};
use teko_chain::{Chain, NetworkProfile};
use teko_election::SubmittedBallot;
use teko_node::{NodeClient, load_network};

/// A board a caller can read: the local file or a verified network replica.
pub enum AnyBoard {
    File(FileBoard),
    Net(NetBoard),
}

/// The verified chain, handed back to the process-wide cache when dropped so the
/// next read only downloads NEW blocks.
pub struct NetBoard {
    chain: Option<Chain>,
}

static CACHE: Mutex<Option<Chain>> = Mutex::new(None);

impl Drop for NetBoard {
    fn drop(&mut self) {
        if let Some(c) = self.chain.take() {
            *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = Some(c);
        }
    }
}

impl NetBoard {
    fn board(&self) -> Option<&MemoryBoard> {
        self.chain.as_ref().map(Chain::board)
    }
    pub fn height(&self) -> u64 {
        self.chain.as_ref().map_or(0, Chain::height)
    }
    pub fn chain(&self) -> Option<&Chain> {
        self.chain.as_ref()
    }

    /// Board index of the ballot cast with `voter_key` in a contest (a device signs only one;
    /// the tally refuses duplicates anyway).
    pub fn find_ballot(
        &self,
        election_id: &str,
        contest_id: &str,
        voter_key: &[u8],
    ) -> Option<u64> {
        let board = self.board()?;
        (0..board.len()).find(|&i| {
            board.entry(i).is_some_and(|e| {
                e.kind == EntryKind::BALLOT
                    && SubmittedBallot::decode(e.payload.as_slice()).is_ok_and(|b| {
                        b.election_id == election_id
                            && b.contest_id == contest_id
                            && b.voter_key == voter_key
                    })
            })
        })
    }
}

fn err() -> BoardError {
    BoardError::Corrupt("network replica unavailable")
}

macro_rules! delegate {
    ($self:ident, $m:ident ( $($a:expr),* ) , $none:expr) => {
        match $self {
            AnyBoard::File(b) => b.$m($($a),*),
            AnyBoard::Net(n) => match n.board() { Some(b) => b.$m($($a),*), None => $none },
        }
    };
}

impl BulletinBoard for AnyBoard {
    fn append(&mut self, entry: Entry) -> Result<Receipt, BoardError> {
        match self {
            AnyBoard::File(b) => b.append(entry),
            // Writes to the network go through `NetBackend::submit`, never here.
            AnyBoard::Net(_) => Err(err()),
        }
    }
    fn len(&self) -> u64 {
        delegate!(self, len(), 0)
    }
    fn entry(&self, index: u64) -> Option<&Entry> {
        delegate!(self, entry(index), None)
    }
    fn root(&self) -> Hash {
        delegate!(self, root(), [0; 32])
    }
    fn inclusion_proof(&self, index: u64) -> Result<Vec<Hash>, BoardError> {
        delegate!(self, inclusion_proof(index), Err(err()))
    }
    fn consistency_proof(&self, old_size: u64) -> Result<Vec<Hash>, BoardError> {
        delegate!(self, consistency_proof(old_size), Err(err()))
    }
}

/// An active test network found in `<dir>/net`.
pub struct NetBackend {
    pub profile: NetworkProfile,
    pub peers: Vec<SocketAddr>,
}

impl NetBackend {
    /// `Some` only if `<dir>/net/network.txt` exists AND a node answers right now
    /// (so a network that was created but is not running falls back to the file).
    pub fn detect(dir: &Path) -> Option<Self> {
        if std::env::var("TEKO_BOARD").is_ok_and(|v| v == "file") {
            return None;
        }
        let net = load_network(&dir.join("net")).ok()?;
        let alive = net
            .peers
            .iter()
            .any(|a| std::net::TcpStream::connect_timeout(a, Duration::from_millis(250)).is_ok());
        alive.then_some(Self {
            profile: net.profile,
            peers: net.peers,
        })
    }

    pub fn client(&self) -> NodeClient {
        NodeClient::new(self.peers.clone())
    }

    /// Verified replica, downloading only the blocks this process has not seen.
    pub fn read(&self) -> Result<NetBoard, teko_node::NodeError> {
        let cached = CACHE.lock().unwrap_or_else(|e| e.into_inner()).take();
        let mut chain = match cached {
            Some(c) if c.profile() == &self.profile => c,
            _ => Chain::new(self.profile.clone()),
        };
        self.client().sync_into(&mut chain)?;
        Ok(NetBoard { chain: Some(chain) })
    }

    /// Sends entries to the network. Returns how many were new.
    pub fn submit(&self, entries: &[Entry]) -> Result<u32, teko_node::NodeError> {
        let mut total = 0;
        for chunk in entries.chunks(400) {
            total += self.client().submit(chunk)?;
        }
        Ok(total)
    }

    /// Submits ONE entry and waits until a block contains it (a few block
    /// intervals), returning where it landed.
    pub fn submit_and_wait(&self, entry: &Entry) -> Result<Receipt, teko_node::NodeError> {
        self.client().submit(std::slice::from_ref(entry))?;
        let deadline =
            Instant::now() + Duration::from_millis(self.profile.block_interval_ms * 6 + 5_000);
        while Instant::now() < deadline {
            let board = self.read()?;
            let n = board.board().map_or(0, |b| b.len());
            for i in (0..n).rev() {
                if board.board().and_then(|b| b.entry(i)) == Some(entry) {
                    let size = n;
                    let root = board.board().map_or([0; 32], |b| b.root());
                    return Ok(Receipt {
                        index: i,
                        size,
                        root,
                    });
                }
            }
            drop(board);
            std::thread::sleep(Duration::from_millis(150));
        }
        Err(teko_node::NodeError::Refused(
            "the network did not include the entry in time".into(),
        ))
    }
}
