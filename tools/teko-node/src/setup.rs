//! Network files on disk (one folder for the whole local network):
//! ```text
//! <dir>/network.txt          public profile (chain_id, validators, threshold)
//! <dir>/peers.txt            `index peer=HOST:PORT public=HOST:PORT key=HEX` per validator (ADR-0048)
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

/// One line of `peers.txt`: where to reach a validator and which consensus key it must prove.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerLink {
    /// The institution's consensus key (SEC1 compressed).
    pub key: Vec<u8>,
    /// The authenticated peer port (`host:port`, an IP or a DNS name).
    pub peer: String,
    /// The public read/submit port clients use.
    pub public: String,
}

pub struct NetFiles {
    pub profile: NetworkProfile,
    /// The PUBLIC addresses of the nodes, by index (what clients dial).
    pub peers: Vec<SocketAddr>,
    /// The peer links, by index (what nodes dial; carries the keys).
    pub links: Vec<PeerLink>,
}

/// How far the public port sits above the peer port in the local test network.
pub const PUBLIC_PORT_OFFSET: u16 = 8;

impl PeerLink {
    /// The line of `peers.txt`.
    pub fn to_line(&self, index: usize) -> String {
        format!(
            "{index} peer={} public={} key={}\n",
            self.peer,
            self.public,
            hex::encode(&self.key)
        )
    }
}

fn resolve(a: &str) -> Result<SocketAddr, NodeError> {
    use std::net::ToSocketAddrs;
    a.to_socket_addrs()
        .ok()
        .and_then(|mut it| it.next())
        .ok_or_else(|| NodeError::Protocol(format!("peers.txt: cannot resolve {a}")))
}

/// Parses `peers.txt`: `index peer=… public=… key=…` per line, `#` comments.
pub fn parse_peers(text: &str) -> Result<Vec<(usize, PeerLink)>, NodeError> {
    let bad = |m: &str| NodeError::Protocol(format!("peers.txt: {m}"));
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut it = line.split_whitespace();
        let index: usize = it
            .next()
            .and_then(|i| i.parse().ok())
            .ok_or_else(|| bad("a line must start with the index (old format? recreate the network: make devnet-reset)"))?;
        let (mut peer, mut public, mut key) = (None, None, None);
        for f in it {
            match f.split_once('=') {
                Some(("peer", v)) => peer = Some(v.to_string()),
                Some(("public", v)) => public = Some(v.to_string()),
                Some(("key", v)) => key = hex::decode(v).ok(),
                _ => return Err(bad("unknown field")),
            }
        }
        let (Some(peer), Some(public), Some(key)) = (peer, public, key) else {
            return Err(bad("each line needs peer=, public= and key="));
        };
        if !teko_p2p::valid_addr(&peer) || !teko_p2p::valid_addr(&public) || key.len() != 33 {
            return Err(bad("bad address or key"));
        }
        out.push((index, PeerLink { key, peer, public }));
    }
    out.sort_by_key(|p| p.0);
    Ok(out)
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
    if nodes > PUBLIC_PORT_OFFSET as usize {
        return Err(NodeError::Refused(format!(
            "a local network has at most {PUBLIC_PORT_OFFSET} nodes"
        )));
    }
    let mut peers = Vec::new();
    let mut links = Vec::new();
    let mut peers_txt = String::from(
        "# index peer=HOST:PORT public=HOST:PORT key=HEX (loopback: this machine only)\n",
    );
    for (i, k) in keys.iter().enumerate() {
        let nd = dir.join(format!("node-{i}"));
        fs::create_dir_all(&nd)?;
        write_private(&nd.join("validator.key"), &hex::encode(&k.to_bytes()))?;
        let link = PeerLink {
            key: public_key_bytes(k),
            peer: format!("127.0.0.1:{}", base_port + i as u16),
            public: format!("127.0.0.1:{}", base_port + PUBLIC_PORT_OFFSET + i as u16),
        };
        peers_txt.push_str(&link.to_line(i));
        peers.push(resolve(&link.public)?);
        links.push(link);
    }
    fs::write(dir.join("network.txt"), profile.to_text())?;
    fs::write(dir.join("peers.txt"), peers_txt)?;
    Ok(NetFiles {
        profile,
        peers,
        links,
    })
}

pub fn load_network(dir: &Path) -> Result<NetFiles, NodeError> {
    let text = fs::read_to_string(dir.join("network.txt")).map_err(|_| {
        NodeError::Refused(format!(
            "no network in {} (run `teko-node init`)",
            dir.display()
        ))
    })?;
    let profile = NetworkProfile::parse(&text)?;
    let listed = parse_peers(&fs::read_to_string(dir.join("peers.txt"))?)?;
    // every genesis institution has an address; nodes that joined LATER (a rotation, ADR-0045) are
    // listed after them and only add to the list
    if listed.len() < profile.validators.len()
        || listed.iter().enumerate().any(|(i, (n, _))| *n != i)
        || listed
            .iter()
            .zip(&profile.validators)
            .any(|((_, l), v)| &l.key != v)
    {
        return Err(NodeError::Protocol(
            "peers.txt does not match the validators".into(),
        ));
    }
    let mut peers = Vec::new();
    let mut links = Vec::new();
    for (_, l) in listed {
        peers.push(resolve(&l.public)?);
        links.push(l);
    }
    Ok(NetFiles {
        profile,
        peers,
        links,
    })
}

/// Assembles `network.txt` and `peers.txt` from the institutions' PUBLIC keys and addresses
/// (`HEXKEY peer=HOST:PORT public=HOST:PORT` per line, the order is the validator order). Nothing
/// secret is involved: every institution made its own key with `keygen` (ADR-0048).
pub fn genesis_network(
    dir: &Path,
    kind: NetworkKind,
    validators_text: &str,
    force: bool,
) -> Result<NetFiles, NodeError> {
    if dir.join("network.txt").exists() && !force {
        return Err(NodeError::Refused(format!(
            "{} already has a network (use --force)",
            dir.display()
        )));
    }
    let bad = |m: &str| NodeError::Protocol(format!("validators: {m}"));
    let mut links = Vec::new();
    for line in validators_text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, rest) = line
            .split_once(char::is_whitespace)
            .ok_or_else(|| bad("line: KEY peer=… public=…"))?;
        let parsed = parse_peers(&format!("{} {} key={}", links.len(), rest.trim(), key))?;
        let (_, link) = parsed.into_iter().next().ok_or_else(|| bad("empty line"))?;
        // the key must be a real SEC1 point, and no key may appear twice
        if teko_crypto::verifying_key_from_bytes(&link.key).is_err() {
            return Err(bad("a public key is not a valid P-256 point"));
        }
        if links.iter().any(|l: &PeerLink| l.key == link.key) {
            return Err(bad("an institution appears twice"));
        }
        links.push(link);
    }
    if links.len() < 2 {
        return Err(bad("at least two institutions"));
    }
    let profile = NetworkProfile::new(
        kind,
        links.iter().map(|l| l.key.clone()).collect(),
        links.len() / 2 + 1,
    )?;
    fs::create_dir_all(dir)?;
    let mut peers_txt = String::from("# index peer=HOST:PORT public=HOST:PORT key=HEX\n");
    let mut peers = Vec::new();
    for (i, l) in links.iter().enumerate() {
        peers_txt.push_str(&l.to_line(i));
        peers.push(resolve(&l.public)?);
    }
    fs::write(dir.join("network.txt"), profile.to_text())?;
    fs::write(dir.join("peers.txt"), peers_txt)?;
    Ok(NetFiles {
        profile,
        peers,
        links,
    })
}

/// Makes a new consensus key in `node_dir` (hex, mode 0600) and returns it.
pub fn generate_validator_key(node_dir: &Path) -> Result<SigningKey, NodeError> {
    fs::create_dir_all(node_dir)?;
    let key = generate_signing_key(&mut UnwrapErr(rand::rngs::SysRng));
    write_validator_key(node_dir, &key)?;
    Ok(key)
}

/// Reads the consensus key of `node_dir`.
pub fn read_validator_key(node_dir: &Path) -> Result<SigningKey, NodeError> {
    load_key(&node_dir.join("validator.key"))
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
pub const DEVNET_CRED_TYPES: [(&str, bool, u64, u64); 6] = [
    ("civic", false, 1_200, 315_360_000),
    ("cnh-a", false, 1_200, 315_360_000),
    ("cnh-b", false, 1_200, 315_360_000),
    ("candidato", true, 120, 31_536_000),
    // civil registry (ADR-0047): private, never expire (validity 0); a divorce revokes the marriage
    ("certidao-casamento", false, 1_200, 0),
    ("certidao-divorcio", false, 1_200, 0),
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
