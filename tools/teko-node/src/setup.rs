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
    if peers.len() != profile.validators.len() {
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
    fs::write(&path, g.to_text())?;
    Ok(g)
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
