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

/// Which chain `blocks` makes blocks on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlocksKind {
    Economy,
    Votes,
}

/// The board entry kind used ONLY by the block simulator: the tally ignores it (`log_kind` has no
/// mapping for it), so it never changes a count, but it does grow the board (`make reset-board`).
pub const FILLER_KIND: u8 = 200;

/// DEV ONLY: makes `count` blocks on the running test network, one at a time, waiting for each
/// (blocks only exist for activity, ADR-0034). Economy: a harmless transaction from a throwaway key
/// (it only links a fresh validator key, needs no balance). Votes: one entry of the reserved filler
/// kind. Refuses a locked network like the other simulators.
pub fn blocks(dir: &Path, kind: BlocksKind, count: u32) -> Result<String, String> {
    if count == 0 || count > 1000 {
        return Err("a quantidade tem de ser de 1 a 1000".into());
    }
    let net = load_network(dir).map_err(|e| e.to_string())?;
    if net.profile.kind.name() == "mainnet" {
        return Err("a mainnet está travada: simulações só em devnet e testnet".into());
    }
    let client = NodeClient::new(net.peers);
    let height = |c: &NodeClient| -> Result<u64, String> {
        let a = *c.addrs().first().ok_or("a rede não tem nós")?;
        match kind {
            BlocksKind::Economy => c.econ_status(a).map(|s| s.height),
            BlocksKind::Votes => c.status(a).map(|s| s.height),
        }
        .map_err(|e| format!("nenhum nó respondeu ({e}): a devnet está de pé?"))
    };
    let chain_id = match kind {
        BlocksKind::Economy => {
            load_economy(dir)
                .map_err(|e| e.to_string())?
                .ok_or("esta rede não tem economia (rode `make devnet` com FOUNDER=…)")?
                .genesis
                .chain_id
        }
        BlocksKind::Votes => String::new(),
    };
    let start = height(&client)?;
    let mut rng = rand::rand_core::UnwrapErr(rand::rngs::SysRng);
    for n in 0..count {
        let before = height(&client)?;
        match kind {
            BlocksKind::Economy => {
                let sk = generate_signing_key(&mut rng);
                let tx = Tx::new_signed(
                    &sk,
                    &chain_id,
                    0,
                    COIN_MODULE,
                    Op::SetValidatorKey {
                        key: public_key_bytes(&sk),
                    }
                    .encode(),
                );
                client
                    .econ_submit(&[tx])
                    .map_err(|e| format!("a rede recusou a transação: {e}"))?;
            }
            BlocksKind::Votes => {
                let mut payload = format!("teko-dev filler {n} ").into_bytes();
                payload.extend_from_slice(&unix_secs().to_be_bytes());
                payload.extend_from_slice(&generate_signing_key(&mut rng).to_bytes());
                client
                    .submit(&[teko_bulletin::Entry::new(
                        teko_bulletin::EntryKind(FILLER_KIND),
                        payload,
                    )])
                    .map_err(|e| format!("a rede recusou a entrada: {e}"))?;
            }
        }
        let t = Instant::now();
        loop {
            if height(&client)? > before {
                break;
            }
            if t.elapsed() > Duration::from_secs(30) {
                return Err(format!(
                    "o bloco {} não saiu em 30 s (a rede tem quórum?)",
                    n + 1
                ));
            }
            thread::sleep(Duration::from_millis(200));
        }
    }
    let end = height(&client)?;
    Ok(format!(
        "{count} bloco(s) de {}: altura {start} -> {end}",
        match kind {
            BlocksKind::Economy => "economia",
            BlocksKind::Votes =>
                "votos (entradas de teste que a apuração ignora; `make reset-board` limpa)",
        }
    ))
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

/// Simulates the SIMULATED members of the credential organs (ADR-0037) APPROVING every credential
/// still waiting for approvals (of `ctype`, or of every type), until the quorum is reached. The
/// member keys are the TEST keys of the network folder. Returns what it did, in Portuguese.
pub fn cosign(dir: &Path, ctype: Option<&str>) -> Result<String, String> {
    use teko_assets::devnet::World;
    use teko_credentials::{CREDENTIALS_MODULE, Op, Status, VoteOp};
    let net = load_network(dir).map_err(|e| e.to_string())?;
    if net.profile.kind.name() == "mainnet" {
        return Err("a mainnet está travada: simulações só em devnet e testnet".into());
    }
    let mut world = teko_node::NodeWorld::open(dir).map_err(|e| e.to_string())?;
    let mut lines = Vec::new();
    let types = teko_credentials::types(World::ledger(&world));
    for (name, t) in types {
        if ctype.is_some_and(|c| c.as_bytes() != name.as_slice()) {
            continue;
        }
        let tname = String::from_utf8_lossy(&name).to_string();
        let sims = teko_node::load_orgmember_keys(dir, &tname);
        for (id, rec) in teko_credentials::credentials_of_type(World::ledger(&world), &name) {
            if rec.status_at(0) != Status::Proposed {
                continue;
            }
            let mut approvals = rec.approvals.clone();
            let mut done = 0;
            for k in &sims {
                if approvals.len() >= t.threshold as usize {
                    break;
                }
                let me = teko_ledger::address_of(&teko_crypto::public_key_bytes(k));
                if !t.members.iter().any(|m| m.as_slice() == me) || approvals.contains(&me.to_vec())
                {
                    continue;
                }
                let tx = teko_ledger::Tx::new_signed(
                    k,
                    world.chain_id(),
                    world.nonce(&me),
                    CREDENTIALS_MODULE,
                    Op::Approve(VoteOp {
                        ctype: name.clone(),
                        id: id.clone(),
                        reason: String::new(),
                    })
                    .encode(),
                );
                world.submit(tx)?;
                approvals.push(me.to_vec());
                done += 1;
            }
            if done > 0 {
                lines.push(format!(
                    "{tname}: {done} membro(s) simulado(s) aprovou(aram) {}… ({}/{} aprovações)",
                    teko_crypto::hex::encode(&id[..4.min(id.len())]),
                    approvals.len(),
                    t.threshold
                ));
            }
        }
    }
    if lines.is_empty() {
        return Err(
            "nenhuma credencial esperando aprovação (ou os membros simulados já aprovaram)".into(),
        );
    }
    Ok(lines.join("\n"))
}

/// The sample candidacies: a name and a number per contest (the numbers are the ballot numbers).
fn sample_candidates(contest: &str) -> &'static [(&'static str, &'static str)] {
    match contest {
        "presidente" => &[
            ("Candidata Onça-Pintada", "10"),
            ("Candidato Boto-Cor-de-Rosa", "11"),
            ("Candidata Arara-Azul", "12"),
        ],
        "governador-sp" => &[
            ("Candidato Guará", "20"),
            ("Candidata Vitória-Régia", "21"),
            ("Candidato Lobo-Guará", "22"),
        ],
        "governador-rj" => &[
            ("Candidato Mico-Leão-Dourado", "40"),
            ("Candidata Capivara", "41"),
            ("Candidato Boto-Tucuxi", "42"),
        ],
        "governador-mg" => &[
            ("Candidato Lobo-Guará", "50"),
            ("Candidata Onça-Parda", "51"),
        ],
        "governador-ba" => &[
            ("Candidata Arara-Vermelha", "60"),
            ("Candidato Tartaruga-Marinha", "61"),
        ],
        "governador-rs" => &[("Candidato Quero-Quero", "70"), ("Candidata Graxaim", "71")],
        "prefeito-3550308" => &[("Candidata Jaguatirica", "30"), ("Candidato Tucano", "31")],
        // the Senate (ADR-0044): SP elects 2, RJ elects 1; always more candidates than seats
        "senador-sp" => &[
            ("Candidato Peixe-Boi", "450"),
            ("Candidata Ariranha", "451"),
            ("Candidato Cervo-do-Pantanal", "452"),
            ("Candidata Harpia", "453"),
        ],
        "senador-rj" => &[
            ("Candidata Muriqui", "460"),
            ("Candidato Preguiça-de-Coleira", "461"),
            ("Candidata Sagui-da-Serra-Escuro", "462"),
        ],
        _ => &[],
    }
}

/// Simulates the ORGAN of `candidato` (its simulated members) issuing the sample candidacies of
/// `election_id` (ADR-0040): one proposal and one approval each, valid for 200 days. The holders'
/// credential keys are derived from fixed seeds (a real candidate gives their own). Returns what it did.
pub fn seed_candidates(dir: &Path, election_id: &str) -> Result<String, String> {
    use rand::SeedableRng;
    use teko_assets::devnet::World;
    use teko_credentials::candidate::{Candidacy, Cargo, candidate_claims};
    use teko_credentials::{
        CREDENTIALS_MODULE, Op, ProposeOp, VoteOp, credential_id, public_commitment,
    };
    let net = load_network(dir).map_err(|e| e.to_string())?;
    if net.profile.kind.name() == "mainnet" {
        return Err("a mainnet está travada: simulações só em devnet e testnet".into());
    }
    let sims = teko_node::load_orgmember_keys(dir, "candidato");
    if sims.len() < 2 {
        return Err("faltam os membros simulados do órgão `candidato` (recrie a rede)".into());
    }
    let mut world = teko_node::NodeWorld::open(dir).map_err(|e| e.to_string())?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let valid_until = now + 200 * 86_400;
    let mut n = 0;
    for p in issuer_cli::dev_contest_plan(now) {
        let cargo = Cargo::parse(p.cargo).ok_or("cargo")?;
        for (i, (nome, numero)) in sample_candidates(p.id).iter().enumerate() {
            let uf = if p.cargo == "governador" || p.cargo == "senador" {
                p.scope_code
            } else {
                ""
            };
            let mun = if p.cargo == "prefeito" {
                p.scope_code
            } else {
                ""
            };
            let claims = candidate_claims(&Candidacy {
                cargo,
                uf: uf.into(),
                municipio: mun.into(),
                numero: (*numero).into(),
                nome: (*nome).into(),
                eleicao: election_id.into(),
            })?
            .into_bytes();
            // the candidate's credential key: fixed per (election, contest, number)
            let seed = u64::from_be_bytes(
                teko_crypto::tagged_hash(
                    "teko/sim/candidate",
                    &[election_id.as_bytes(), p.id.as_bytes(), &[i as u8]],
                )[..8]
                    .try_into()
                    .map_err(|_| "seed")?,
            );
            let holder = teko_crypto::public_key_bytes(&teko_crypto::generate_signing_key(
                &mut rand::rngs::StdRng::seed_from_u64(seed),
            ));
            let commitment = public_commitment(&claims);
            let id = credential_id(b"candidato", &holder, &commitment, &claims, valid_until);
            let send = |world: &mut teko_node::NodeWorld,
                        k: &teko_crypto::SigningKey,
                        op: Op|
             -> Result<(), String> {
                let me = teko_ledger::address_of(&teko_crypto::public_key_bytes(k));
                let tx = teko_ledger::Tx::new_signed(
                    k,
                    World::chain_id(world),
                    world.nonce(&me),
                    CREDENTIALS_MODULE,
                    op.encode(),
                );
                world.submit(tx)
            };
            send(
                &mut world,
                &sims[0],
                Op::Propose(ProposeOp {
                    ctype: b"candidato".to_vec(),
                    subject: holder,
                    commitment: commitment.to_vec(),
                    claims,
                    valid_until,
                    sealed: vec![],
                }),
            )?;
            send(
                &mut world,
                &sims[1],
                Op::Approve(VoteOp {
                    ctype: b"candidato".to_vec(),
                    id: id.to_vec(),
                    reason: String::new(),
                }),
            )?;
            n += 1;
        }
    }
    Ok(format!(
        "{n} candidaturas emitidas para a eleição `{election_id}` (vigentes em ~2 min, a janela de contestação de teste); depois rode: make election"
    ))
}
