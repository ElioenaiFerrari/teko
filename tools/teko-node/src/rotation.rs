//! ROTATING the institutions of the vote chain, from the operator's side (ADR-0045): propose a new
//! set, collect the approvals of the institutions in force, carry the approved change to the vote
//! chain and to the economy ledger.
//!
//! The chain only checks the signatures; the rule "no election in progress" is obeyed by whoever
//! SIGNS: `check_safe` replays the verified board and refuses while a contest still depends on the
//! key ceremony of the set in force.

use std::path::Path;

use prost::Message;
use teko_bft::{ValidatorSetChange, set_hash};
use teko_bulletin::{BulletinBoard, Entry, EntryKind};
use teko_chain::Chain;
use teko_crypto::{SigningKey, generate_signing_key, hex, public_key_bytes};

use crate::client::NodeClient;
use crate::error::NodeError;
use crate::setup::{load_network, load_validator_key, node_dir};

/// How many blocks ahead a change takes effect when the operator does not say.
pub const DEFAULT_EFFECTIVE_IN: u64 = 6;

fn refuse(m: impl Into<String>) -> NodeError {
    NodeError::Refused(m.into())
}

/// The verified vote chain, downloaded from the nodes.
fn synced(dir: &Path) -> Result<Chain, NodeError> {
    let net = load_network(dir)?;
    NodeClient::new(net.peers).sync_chain(&net.profile)
}

/// May the institutions rotate NOW? Replays the verified board and asks the tally. Without the
/// election's trust file the state of the elections cannot be judged: then an election on the board
/// refuses the rotation.
pub fn check_safe(dir: &Path, chain: &Chain) -> Result<(), NodeError> {
    use teko_election::{GovernancePolicy, LogEntry, LogKind, Tallier};
    let board = chain.board();
    let mut has_election = false;
    for i in 0..board.len() {
        if board
            .entry(i)
            .is_some_and(|e| e.kind == EntryKind::MANIFEST)
        {
            has_election = true;
            break;
        }
    }
    if !has_election {
        return Ok(());
    }
    let trust_path = dir.parent().map(|p| p.join("trust.txt"));
    let trust = trust_path
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| teko_attestation::TrustPolicy::parse(&t).ok())
        .ok_or_else(|| {
            refuse("há uma eleição no quadro e não achei o trust.txt para conferir o estado dela")
        })?;
    let policy = GovernancePolicy::new(trust.gov_signers.clone(), trust.gov_threshold)
        .map_err(|_| refuse("trust.txt sem governança de eleições"))?;
    let mut t = Tallier::new(policy).with_verifier(Box::new(teko_registration::RsaRegistration));
    if !trust.chain_id.is_empty() {
        t = t.with_chain_id(trust.chain_id.clone());
    }
    for i in 0..board.len() {
        let Some(e) = board.entry(i) else { continue };
        let kind = match e.kind {
            EntryKind::MANIFEST => Some(LogKind::Manifest),
            EntryKind::TRANSITION => Some(LogKind::Transition),
            EntryKind::REGISTRATION => Some(LogKind::Registration),
            EntryKind::BALLOT => Some(LogKind::Ballot),
            EntryKind::DKG => Some(LogKind::Dkg),
            EntryKind::RESULT => Some(LogKind::Result),
            EntryKind::DRAW => Some(LogKind::Draw),
            _ => None,
        };
        match kind {
            Some(kind) => t.feed(LogEntry {
                kind,
                payload: &e.payload,
            }),
            None => t.skip(),
        }
    }
    t.rotation_safe().map_err(refuse)
}

/// A proposal to replace the institutions by `new` (ordered consensus keys), taking effect
/// `effective_in` blocks after the chain's current height. Refuses while an election is in progress
/// or another change is pending.
pub fn propose(
    dir: &Path,
    new: Vec<Vec<u8>>,
    effective_in: u64,
) -> Result<ValidatorSetChange, NodeError> {
    let chain = synced(dir)?;
    let last = chain
        .epochs()
        .last()
        .ok_or_else(|| refuse("a chain sem época"))?;
    if last.from_height > chain.height() + 1 {
        return Err(refuse("já há uma troca de instituições pendente"));
    }
    check_safe(dir, &chain)?;
    let c = ValidatorSetChange {
        chain_id: chain.profile().chain_id.clone(),
        epoch: last.epoch + 1,
        effective_height: chain.height() + effective_in.max(2),
        prev_set_hash: set_hash(&last.set).to_vec(),
        validators: new,
        ..Default::default()
    };
    c.check_new_set().map_err(refuse)?;
    Ok(c)
}

/// The institution at `index` approves the change with ITS key. Refuses (re-checking the chain
/// itself, not trusting the proposer) while an election is in progress.
pub fn sign(dir: &Path, index: usize, change: &mut ValidatorSetChange) -> Result<(), NodeError> {
    let chain = synced(dir)?;
    let last = chain
        .epochs()
        .last()
        .ok_or_else(|| refuse("a chain sem época"))?;
    change
        .check_link(&chain.profile().chain_id, last.epoch, &last.set)
        .map_err(refuse)?;
    check_safe(dir, &chain)?;
    let key = load_validator_key(dir, index)?;
    if !last.set.contains(&public_key_bytes(&key)) {
        return Err(refuse("esta chave não é de uma instituição em vigor"));
    }
    change.sign(&key);
    Ok(())
}

/// Carries an APPROVED change to the vote chain and, if the network has one, to the economy ledger.
/// Returns what it did.
pub fn submit(dir: &Path, change: &ValidatorSetChange) -> Result<Vec<String>, NodeError> {
    let chain = synced(dir)?;
    let last = chain
        .epochs()
        .last()
        .ok_or_else(|| refuse("a chain sem época"))?;
    change
        .follow(&chain.profile().chain_id, last.epoch, &last.set)
        .map_err(refuse)?;
    let net = load_network(dir)?;
    let client = NodeClient::new(net.peers);
    let entry = Entry::new(EntryKind::VALIDATOR_SET, change.encode_to_vec());
    client.submit(std::slice::from_ref(&entry))?;
    let mut lines = vec![format!(
        "troca para a época {} enviada à chain de votos (vale a partir do bloco {})",
        change.epoch, change.effective_height
    )];
    // the economy ledger keeps its own copy of the set (ClaimElected reads it)
    if crate::setup::load_economy(dir)?.is_some() {
        use teko_assets::devnet::World;
        let mut world = crate::world::NodeWorld::open(dir)?;
        let sender = generate_signing_key(&mut rand::rand_core::UnwrapErr(rand::rngs::SysRng));
        let tx = teko_ledger::Tx::new_signed(
            &sender,
            world.chain_id(),
            0,
            teko_credentials::CREDENTIALS_MODULE,
            teko_credentials::Op::RotateVoteSet(change.clone()).encode(),
        );
        match world.submit(tx) {
            Ok(()) => lines.push("o livro de economia também passou a usar o conjunto novo".into()),
            Err(e) => lines.push(format!("a economia recusou a troca ({e})")),
        }
    }
    Ok(lines)
}

/// Who takes part in the next key ceremony and under which session (ADR-0045).
#[derive(Debug, Clone)]
pub struct CeremonyPlan {
    pub epoch: u64,
    pub session: String,
    /// The institutions in force, in the order of their set (ceremony index = position + 1).
    pub participants: Vec<crate::setup::PeerLink>,
    pub threshold: usize,
}

/// The ceremony of the institutions IN FORCE: epoch 0 keeps the session of always; after a rotation
/// the session names the epoch and the set, so a ceremony of an earlier set is another ceremony.
pub fn ceremony_plan(dir: &Path) -> Result<CeremonyPlan, NodeError> {
    let net = load_network(dir)?;
    let chain = NodeClient::new(net.peers.clone()).sync_chain(&net.profile)?;
    let epoch = chain.epoch_at(chain.height() + 1).clone();
    let mut participants = Vec::new();
    for member in &epoch.set {
        // the institution's link (key and addresses) from `peers.txt`; no key folder is needed here
        let link = net
            .links
            .iter()
            .find(|l| &l.key == member)
            .cloned()
            .ok_or_else(|| refuse("uma instituição do conjunto em vigor não está no peers.txt"))?;
        participants.push(link);
    }
    let session = if epoch.epoch == 0 {
        format!("{}|tally-key", net.profile.chain_id)
    } else {
        format!(
            "{}|tally-key|e{}|{}",
            net.profile.chain_id,
            epoch.epoch,
            &hex::encode(&set_hash(&epoch.set))[..16]
        )
    };
    let threshold = participants.len() / 2 + 1;
    Ok(CeremonyPlan {
        epoch: epoch.epoch,
        session,
        participants,
        threshold,
    })
}

/// What `dev_rotate` did.
#[derive(Debug, Clone)]
pub struct DevRotation {
    pub change: ValidatorSetChange,
    /// Index (in `peers.txt`) and key file of the NEW node, to be started with `run --index`.
    pub new_index: usize,
    pub new_key: Vec<u8>,
    pub lines: Vec<String>,
}

/// DEV ONLY: replaces the institution at set position `out` with a brand-new one: creates its key
/// and node folder, adds its address to `peers.txt` (`port`), has the OTHER institutions approve
/// (their keys are in the node folders of this machine) and submits. The new node still has to be
/// started: `teko-node run --index <new_index>`.
pub fn dev_rotate(dir: &Path, out: usize, port: u16) -> Result<DevRotation, NodeError> {
    let chain = synced(dir)?;
    let last = chain
        .epochs()
        .last()
        .ok_or_else(|| refuse("a chain sem época"))?
        .clone();
    if out >= last.set.len() {
        return Err(refuse("não existe essa instituição no conjunto em vigor"));
    }
    let net = load_network(dir)?;
    let new_index = net.links.len();
    let key: SigningKey = generate_signing_key(&mut rand::rand_core::UnwrapErr(rand::rngs::SysRng));
    let new_pub = public_key_bytes(&key);
    // the new node's folder, key and address (the key file format of `init`)
    let nd = node_dir(dir, new_index);
    std::fs::create_dir_all(&nd)?;
    crate::setup::write_validator_key(&nd, &key)?;
    let mut peers = std::fs::read_to_string(dir.join("peers.txt"))?;
    if !peers.ends_with('\n') {
        peers.push('\n');
    }
    let link = crate::setup::PeerLink {
        key: new_pub.clone(),
        peer: format!("127.0.0.1:{port}"),
        public: format!("127.0.0.1:{}", port + crate::setup::PUBLIC_PORT_OFFSET),
    };
    peers.push_str(&link.to_line(new_index));
    std::fs::write(dir.join("peers.txt"), peers)?;
    // the proposal: the same order, the newcomer in the outgoing slot
    let mut set = last.set.clone();
    set[out] = new_pub.clone();
    let mut change = propose(dir, set, DEFAULT_EFFECTIVE_IN)?;
    // every institution in force EXCEPT the outgoing one approves (more than 2/3 of the old set)
    for (pos, member) in last.set.iter().enumerate() {
        if pos == out {
            continue;
        }
        let Some(i) = (0..net.links.len())
            .find(|i| load_validator_key(dir, *i).is_ok_and(|k| &public_key_bytes(&k) == member))
        else {
            continue;
        };
        sign(dir, i, &mut change)?;
    }
    let mut lines = submit(dir, &change)?;
    lines.push(format!(
        "novo nó {new_index}: chave {} em 127.0.0.1:{port} (pública {})",
        hex::encode(&new_pub[..8]),
        port + crate::setup::PUBLIC_PORT_OFFSET
    ));
    Ok(DevRotation {
        change,
        new_index,
        new_key: new_pub,
        lines,
    })
}
