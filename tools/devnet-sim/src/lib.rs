//! Development-only simulators for the LOCAL test network. This is not a validator and
//! not part of any release: it exists to see the penalty path work on a devnet.
//!
//! **Equivocation.** A validator that signs two conflicting votes (same height, round
//! and kind, different blocks) is penalized. The BFT engine detects it live, but with the
//! single founder validator a height is decided the instant it starts, so forging votes
//! over the wire cannot be done reliably. The ledger accepts the evidence of ANY recent
//! height from ANYONE, which is how a third party denounces a validator, so this tool
//! signs the two votes with the test validator's consensus key and submits them as a
//! `Slash` transaction from a throwaway account. It exercises the ledger's penalty
//! (checks, burn, event); live detection by the engine is covered by the `teko-bft`
//! simulator and the `teko-economy` and `teko-node` tests.

use std::path::Path;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use prost::Message;
use teko_bft::{Evidence, Vote, VoteKind};
use teko_coin::{COIN_MODULE, Op};
use teko_crypto::{generate_signing_key, public_key_bytes, wallet_address};
use teko_ledger::{Address, Tx};
use teko_node::{NodeClient, load_economy, load_network, load_validator_key};

/// What the penalty did, read from the chain the tool verified itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub chain_id: String,
    pub validator_index: usize,
    /// Wallet address that owns the validator (it bears the penalty).
    pub owner: String,
    pub height_before: u64,
    pub height_after: u64,
    /// The height the forged votes claim (a recent one).
    pub offence_height: u64,
    pub stake_before: u64,
    pub stake_after: u64,
    pub unbonding_before: u64,
    pub unbonding_after: u64,
    pub burned_before: u64,
    pub burned_after: u64,
    pub slash_bps: u32,
    /// The ledger executed the `Slash` (the burned total went up or the offence was
    /// recorded).
    pub applied: bool,
}

impl Report {
    /// Coins taken from the owner (stake plus what was unbonding).
    pub fn penalty(&self) -> u64 {
        self.burned_after.saturating_sub(self.burned_before)
    }
}

fn unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Builds the evidence: two prevotes of `key` for the same height and round, naming
/// different blocks. `round` makes each simulation a NEW offence (the ledger counts one
/// penalty per offence).
pub fn forge_evidence(
    chain_id: &str,
    key: &teko_crypto::SigningKey,
    height: u64,
    round: u32,
) -> Vec<u8> {
    let a = Vote::new(
        chain_id,
        key,
        VoteKind::Prevote,
        height,
        round,
        Some(&[0xAA; 32]),
    );
    let b = Vote::new(
        chain_id,
        key,
        VoteKind::Prevote,
        height,
        round,
        Some(&[0xBB; 32]),
    );
    Evidence {
        a: Some(a),
        b: Some(b),
    }
    .encode_to_vec()
}

fn read(
    client: &NodeClient,
    genesis: &teko_economy::EconomyGenesis,
    consensus_key: &[u8],
) -> Result<(u64, Address, u64, u64, u64, u32), String> {
    let chain = client.econ_sync(genesis).map_err(|e| e.to_string())?;
    let l = chain.ledger();
    let owner = chain.owner_of(consensus_key);
    let unb: u64 = teko_coin::unbonding_of(l, &owner)
        .iter()
        .map(|(_, a)| *a)
        .sum();
    let bps = teko_coin::config(l).map_or(0, |c| c.slash_bps);
    Ok((
        chain.height(),
        owner,
        teko_coin::stake_of(l, &owner),
        unb,
        teko_coin::burned(l),
        bps,
    ))
}

/// Simulates the equivocation of validator `index` of the network in `dir` and waits
/// (up to `wait`) for the penalty to be included in a block.
pub fn equivocate(dir: &Path, index: usize, wait: Duration) -> Result<Report, String> {
    let net = load_network(dir).map_err(|e| e.to_string())?;
    if net.profile.kind.name() == "mainnet" {
        return Err("a mainnet está travada: simulações só em devnet e testnet".into());
    }
    let ec = load_economy(dir)
        .map_err(|e| e.to_string())?
        .ok_or("esta rede não tem economia (rode `make devnet` com FOUNDER=…)")?;
    let key = load_validator_key(dir, index).map_err(|e| format!("validador {index}: {e}"))?;
    let consensus = public_key_bytes(&key);
    let client = NodeClient::new(net.peers);

    let (h0, owner, stake0, unb0, burned0, bps) = read(&client, &ec.genesis, &consensus)?;
    if h0 == 0 {
        return Err("a rede ainda não produziu blocos: espere alguns segundos".into());
    }
    // the votes claim the last decided height: recent enough for the penalty window
    let offence_height = h0;
    let round = (unix_secs() % u64::from(u32::MAX)) as u32;
    let evidence = forge_evidence(&ec.genesis.chain_id, &key, offence_height, round);

    // anyone may submit evidence: a throwaway account sends it (devnet fees are 0)
    let mut rng = rand::rand_core::UnwrapErr(rand::rngs::SysRng);
    let sender = generate_signing_key(&mut rng);
    let tx = Tx::new_signed(
        &sender,
        &ec.genesis.chain_id,
        0,
        COIN_MODULE,
        Op::Slash { evidence }.encode(),
    );
    client
        .econ_submit(&[tx])
        .map_err(|e| format!("a rede recusou a transação: {e}"))?;

    let start = Instant::now();
    let mut last = (h0, stake0, unb0, burned0);
    let mut applied = false;
    while start.elapsed() < wait {
        thread::sleep(Duration::from_millis(500));
        let (h, _, s, u, b, _) = read(&client, &ec.genesis, &consensus)?;
        last = (h, s, u, b);
        if b > burned0 {
            applied = true;
            break;
        }
        // the offence may be recorded with a zero penalty (owner without stake): then the
        // chain advances but nothing is burned; stop after a few blocks
        if h >= h0 + 4 {
            applied = true;
            break;
        }
    }
    Ok(Report {
        chain_id: ec.genesis.chain_id.clone(),
        validator_index: index,
        owner: wallet_address(&owner).unwrap_or_default(),
        height_before: h0,
        height_after: last.0,
        offence_height,
        stake_before: stake0,
        stake_after: last.1,
        unbonding_before: unb0,
        unbonding_after: last.2,
        burned_before: burned0,
        burned_after: last.3,
        slash_bps: bps,
        applied,
    })
}

/// Simulates the registrar quorum (órgão) of `class` VETOING the pending trade of `asset` inside its
/// veto window (ADR-0036), with a public `reason`. The simulated órgão keys are the TEST keys of the
/// network folder; the buyer's price returns from the escrow and the asset stays with its owner.
pub fn veto(dir: &Path, class: &str, asset: &str, reason: &str) -> Result<String, String> {
    use teko_assets::devnet::{self, METADATA_DIR};
    let net = load_network(dir).map_err(|e| e.to_string())?;
    if net.profile.kind.name() == "mainnet" {
        return Err("a mainnet está travada: simulações só em devnet e testnet".into());
    }
    let mut world = teko_node::NodeWorld::open(dir).map_err(|e| e.to_string())?;
    let mut cast = devnet::Cast::load(dir)
        .map_err(|e| e.to_string())?
        .with_store(dir.join(METADATA_DIR));
    for c in devnet::CLASSES {
        cast = cast.with_registrars(c, teko_node::load_class_registrar_keys(dir, c));
    }
    devnet::veto_pending(&mut world, &cast, class, asset, reason)?;
    Ok(format!(
        "O órgão simulado de {class} vetou a negociação de {asset}: o preço voltou ao comprador e o bem continua com o dono."
    ))
}
