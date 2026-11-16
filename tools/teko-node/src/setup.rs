//! Network files on disk (one folder for the whole local network):
//! ```text
//! <dir>/network.txt          public profile (chain_id, validators, threshold)
//! <dir>/peers.txt            `index=ip:port` per validator
//! <dir>/node-<i>/validator.key   SECRET (hex), mode 0600, test keys only
//! ```

use std::fs;
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use rand::rand_core::UnwrapErr;
use teko_chain::{NetworkKind, NetworkProfile};
use teko_crypto::{SigningKey, generate_signing_key, hex, public_key_bytes};

use crate::error::NodeError;

pub struct NetFiles {
    pub profile: NetworkProfile,
    pub peers: Vec<SocketAddr>,
}

fn write_private(path: &Path, content: &str) -> std::io::Result<()> {
    let mut o = fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.open(path)?.write_all(content.as_bytes())
}

/// Creates a local network of `nodes` validators with fresh TEST keys.
/// Refuses to overwrite an existing network unless `force`.
pub fn init_network(
    dir: &Path,
    kind: NetworkKind,
    nodes: usize,
    base_port: u16,
    force: bool,
) -> Result<NetFiles, NodeError> {
    if dir.join("network.txt").exists() && !force {
        return Err(NodeError::Refused(format!(
            "{} already has a network (use --force)",
            dir.display()
        )));
    }
    fs::create_dir_all(dir)?;
    let mut rng = UnwrapErr(rand::rngs::SysRng);
    let keys: Vec<SigningKey> = (0..nodes).map(|_| generate_signing_key(&mut rng)).collect();
    let pubs: Vec<Vec<u8>> = keys.iter().map(public_key_bytes).collect();
    // M-of-N: a strict majority.
    let profile = NetworkProfile::new(kind, pubs, nodes / 2 + 1)?;
    let mut peers = Vec::new();
    let mut peers_txt = String::from("# index=address (loopback: this machine only)\n");
    for (i, k) in keys.iter().enumerate() {
        let nd = dir.join(format!("node-{i}"));
        fs::create_dir_all(&nd)?;
        write_private(&nd.join("validator.key"), &hex::encode(&k.to_bytes()))?;
        let addr: SocketAddr = format!("127.0.0.1:{}", base_port + i as u16)
            .parse()
            .map_err(|_| NodeError::Protocol("bad address".into()))?;
        peers_txt.push_str(&format!("{i}={addr}\n"));
        peers.push(addr);
    }
    fs::write(dir.join("network.txt"), profile.to_text())?;
    fs::write(dir.join("peers.txt"), peers_txt)?;
    Ok(NetFiles { profile, peers })
}

pub fn load_network(dir: &Path) -> Result<NetFiles, NodeError> {
    let text = fs::read_to_string(dir.join("network.txt")).map_err(|_| {
        NodeError::Refused(format!(
            "no network in {} (run `teko-node init`)",
            dir.display()
        ))
    })?;
    let profile = NetworkProfile::parse(&text)?;
    let peers_text = fs::read_to_string(dir.join("peers.txt"))?;
    let mut peers: Vec<(usize, SocketAddr)> = Vec::new();
    for line in peers_text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (i, a) = line
            .split_once('=')
            .ok_or_else(|| NodeError::Protocol("bad peers.txt".into()))?;
        peers.push((
            i.trim()
                .parse()
                .map_err(|_| NodeError::Protocol("bad peers.txt".into()))?,
            a.trim()
                .parse()
                .map_err(|_| NodeError::Protocol("bad peers.txt".into()))?,
        ));
    }
    peers.sort_by_key(|p| p.0);
    // every genesis institution has an address; nodes that joined LATER (a rotation, ADR-0045) are
    // listed after them and only add to the list
    if peers.len() < profile.validators.len() {
        return Err(NodeError::Protocol(
            "peers.txt does not match the validators".into(),
        ));
    }
    Ok(NetFiles {
        profile,
        peers: peers.into_iter().map(|p| p.1).collect(),
    })
}

pub fn node_dir(dir: &Path, index: usize) -> PathBuf {
    dir.join(format!("node-{index}"))
}

pub(crate) fn load_key(path: &Path) -> Result<SigningKey, NodeError> {
    let text = fs::read_to_string(path)
        .map_err(|_| NodeError::Refused(format!("missing {}", path.display())))?;
    let bytes = hex::decode(text.trim()).map_err(|_| NodeError::Protocol("bad key file".into()))?;
    SigningKey::from_slice(&bytes).map_err(|_| NodeError::Protocol("bad key file".into()))
}

/// Writes a validator key (hex, mode 0600) into a node folder (a node that joins later, ADR-0045).
pub fn write_validator_key(node_dir: &Path, key: &SigningKey) -> Result<(), NodeError> {
    write_private(
        &node_dir.join("validator.key"),
        &hex::encode(&key.to_bytes()),
    )?;
    Ok(())
}

pub fn load_validator_key(dir: &Path, index: usize) -> Result<SigningKey, NodeError> {
    load_key(&node_dir(dir, index).join("validator.key"))
}

/// File with the economy genesis of the network (public parameters, no secrets).
pub const ECONOMY_FILE: &str = "economy.txt";

/// Writes `economy.txt` for the network in `dir` (ADR-0028, option B): the founder is
/// validator 0 (its consensus key is the node key) and the rewards go to `founder_owner`
/// (the wallet address of the product owner). Without an owner the rewards go to the
/// node's own account, which nobody can spend from (a warning for the caller).
/// `founder_target` (ypy) overrides N (local devnet default: 1.000 TEKO). `founder_alloc` (ypy) puts the
/// first coins of the network in the founder wallet, in the genesis (only with a `founder_owner`).
/// Also creates the TEST registrar keys of the asset classes `imovel` and `veiculo` (2 of 3 each).
pub fn init_economy(
    dir: &Path,
    founder_owner: Option<teko_ledger::Address>,
    founder_target: Option<u64>,
    founder_alloc: Option<u64>,
    force: bool,
) -> Result<teko_economy::EconomyGenesis, NodeError> {
    init_economy_with_issuer(
        dir,
        founder_owner,
        founder_owner,
        founder_target,
        founder_alloc,
        force,
    )
}

/// The credential types of the local test network (ADR-0037): name, public claims, contest
/// window (s) and validity (s). Each has an organ of 3 members, 2-of-3: the `issuer` wallet (when
/// given) and simulated test members whose keys live in `orgmember-<type>-<i>.key`.
pub const DEVNET_CRED_TYPES: [(&str, bool, u64, u64); 4] = [
    ("civic", false, 1_200, 315_360_000),
    ("cnh-a", false, 1_200, 315_360_000),
    ("cnh-b", false, 1_200, 315_360_000),
    ("candidato", true, 120, 31_536_000),
];

/// File of the simulated organ member `i` of a credential type (TEST key).
pub fn orgmember_file(ctype: &str, i: usize) -> String {
    format!("orgmember-{ctype}-{i}.key")
}

/// Like `init_economy`, with the wallet that is a MEMBER of the test credential organs (`issuer`;
/// the app that issues credentials signs with it, on the device). Without it all 3 members are
/// simulated.
pub fn init_economy_with_issuer(
    dir: &Path,
    founder_owner: Option<teko_ledger::Address>,
    issuer: Option<teko_ledger::Address>,
    founder_target: Option<u64>,
    founder_alloc: Option<u64>,
    force: bool,
) -> Result<teko_economy::EconomyGenesis, NodeError> {
    let path = dir.join(ECONOMY_FILE);
    if path.exists() && !force {
        return Err(NodeError::Refused(format!(
            "{} already exists (use --force)",
            path.display()
        )));
    }
    let net = load_network(dir)?;
    let founder_key = net
        .profile
        .validators
        .first()
        .cloned()
        .ok_or_else(|| NodeError::Protocol("network without validators".into()))?;
    let owner = founder_owner.unwrap_or_else(|| teko_ledger::address_of(&founder_key));
    let mut g = teko_economy::EconomyGenesis::test_defaults(
        &format!("{}-economy", net.profile.chain_id),
        founder_key,
        owner,
    );
    // the vote chain's institutions bond in this economy (ADR-0035)
    g.coin.vote_chain_id.clone_from(&net.profile.chain_id);
    if let Some(n) = founder_target {
        g.coin.founder_target = n;
    }
    // The first coins of the network, in the GENESIS, to the founder's wallet (ADR-0034: with no
    // block made on a timer, the network needs coins before the first transaction).
    if let (Some(n), Some(wallet)) = (founder_alloc.filter(|n| *n > 0), founder_owner) {
        g.allocations.push((wallet, n));
    }
    let mut rng = UnwrapErr(rand::rngs::SysRng);
    // one órgão (2-of-3, TEST keys) per asset class (ADR-0032)
    for class in teko_assets::devnet::CLASSES {
        let mut regs = Vec::new();
        for i in 0..3 {
            let p = dir.join(teko_assets::devnet::registrar_file(class, i));
            let k = if p.exists() {
                load_key(&p)?
            } else {
                let k = generate_signing_key(&mut rng);
                write_private(&p, &hex::encode(&k.to_bytes()))?;
                k
            };
            regs.push(public_key_bytes(&k));
        }
        g.classes.push(teko_economy::ClassSpec {
            name: class.into(),
            threshold: 2,
            registrars: regs,
            // the devnet window is short so a trade can be tested in minutes (ADR-0036)
            window_secs: 1_200,
        });
    }
    // one organ per credential type (ADR-0037): the issuer wallet + simulated members
    for (name, public, contest_secs, validity_secs) in DEVNET_CRED_TYPES {
        let sim = if issuer.is_some() { 2 } else { 3 };
        let mut members: Vec<teko_ledger::Address> = issuer.into_iter().collect();
        for i in 0..sim {
            let p = dir.join(orgmember_file(name, i));
            let k = if p.exists() {
                load_key(&p)?
            } else {
                let k = generate_signing_key(&mut rng);
                write_private(&p, &hex::encode(&k.to_bytes()))?;
                k
            };
            members.push(teko_ledger::address_of(&public_key_bytes(&k)));
        }
        g.credtypes.push(teko_economy::CredTypeSpec {
            name: name.into(),
            threshold: 2,
            members,
            contest_secs,
            validity_secs,
            public,
            ..Default::default()
        });
    }
    // the elected: a SYSTEM credential (no organ) created from an outcome that more than 2/3 of the
    // vote chain's institutions signed (ADR-0040)
    g.credtypes.push(teko_economy::CredTypeSpec {
        name: "eleito".into(),
        public: true,
        system: true,
        ..Default::default()
    });
    g.vote_validators = net.profile.validators.clone();
    fs::write(&path, g.to_text())?;
    sync_civic_trust(dir, &g)?;
    Ok(g)
}

/// Tells the DEVICES which organ issues the civic identity credential (ADR-0038): the members and
/// the quorum of the `civic` credential type, written into `dev/trust.txt` (the folder above the
/// network). The emulator reads it at start, so restart it after creating the network. Without a
/// `trust.txt` there is nothing to update (a test folder).
pub fn sync_civic_trust(dir: &Path, g: &teko_economy::EconomyGenesis) -> Result<(), NodeError> {
    let Some(trust_path) = dir.parent().map(|p| p.join("trust.txt")) else {
        return Ok(());
    };
    let Ok(text) = fs::read_to_string(&trust_path) else {
        return Ok(());
    };
    let mut trust = teko_attestation::TrustPolicy::parse(&text)
        .map_err(|e| NodeError::Protocol(format!("trust.txt: {e}")))?;
    if let Some(civic) = g.credtypes.iter().find(|c| c.name == "civic") {
        trust.civic_chain_id.clone_from(&g.chain_id);
        trust.civic_threshold = civic.threshold as usize;
        trust.civic_members = civic.members.iter().map(|a| a.to_vec()).collect();
    }
    // the vote-chain institutions: the device checks its ballot's certificate against them (ADR-0041)
    if !g.vote_validators.is_empty() && !g.coin.vote_chain_id.is_empty() {
        trust.vote_chain_id.clone_from(&g.coin.vote_chain_id);
        trust.vote_validators = g.vote_validators.clone();
    }
    fs::write(&trust_path, trust.to_text())?;
    Ok(())
}

/// The simulated members of a credential organ (TEST keys), in file order.
pub fn load_orgmember_keys(dir: &Path, ctype: &str) -> Vec<SigningKey> {
    (0..3)
        .filter_map(|i| load_key(&dir.join(orgmember_file(ctype, i))).ok())
        .collect()
}

/// Loads the economy configuration of the network, if it has one.
pub fn load_economy(dir: &Path) -> Result<Option<crate::EconomyConfig>, NodeError> {
    let Ok(text) = fs::read_to_string(dir.join(ECONOMY_FILE)) else {
        return Ok(None);
    };
    let genesis = teko_economy::EconomyGenesis::parse(&text).map_err(NodeError::Economy)?;
    Ok(Some(crate::EconomyConfig { genesis }))
}

/// The TEST registrar keys of the órgão of `class` (next to the network).
pub fn load_class_registrar_keys(dir: &Path, class: &str) -> Vec<SigningKey> {
    (0..3)
        .filter_map(|i| load_key(&dir.join(teko_assets::devnet::registrar_file(class, i))).ok())
        .collect()
}

/// The TEST registrar keys of the `imovel` class (kept for older callers).
pub fn load_registrar_keys(dir: &Path) -> Vec<SigningKey> {
    load_class_registrar_keys(dir, "imovel")
}
