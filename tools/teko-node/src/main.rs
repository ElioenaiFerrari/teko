//! `teko-node`: run and inspect a local federated PoA test network.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;
use std::thread::sleep;
use std::time::Duration;

use teko_chain::NetworkKind;
use teko_node::{Node, NodeClient, NodeConfig, init_network, load_network};

const HELP: &str = "\
teko-node: rede de teste federada (PoA). PROTÓTIPO, só chaves de teste.

USO:
  teko-node init   [--dir dev/net] [--network devnet|testnet] [--nodes 3] [--port 7901] [--founder tekow1…] [--issuer tekow1…] [--alloc TEKO] [--founder-target TEKO] [--force]
                   --issuer: carteira MEMBRO dos órgãos de credenciais de teste (padrão: o --founder)
                   --founder: endereço da CARTEIRA (do dispositivo) que recebe as recompensas do fundador (opção B, ADR-0028)
  teko-node keygen [--dir node-X] [--force]       cria a chave de consenso DESTA instituição (nesta máquina) e imprime a pública
  teko-node genesis --dir NET --validators ARQUIVO [--network testnet] [--force]   monta network.txt e peers.txt (linhas: CHAVE_PUBLICA_HEX peer=HOST:PORTA public=HOST:PORTA)
  teko-node economy-init --dir NET --founder tekow1… [--issuer …] [--alloc TEKO] [--founder-target TEKO] [--force]   cria economy.txt
  teko-node genesis-hash [--dir NET]               hash do genesis (todas as instituições devem ver o MESMO; entra no handshake)
  teko-node run    [--dir dev/net] --index I [--listen-peer ADDR] [--listen-public ADDR] [--advertise-peer HOST:PORT] [--advertise-public HOST:PORT] [--allow-remote]
                   sobe o validador I; por padrão escuta em loopback; --allow-remote libera outros endereços (a porta de pares exige a chave da instituição; a pública é só leitura e envio, com limites)
  teko-node stop   [--dir dev/net]                  para os validadores iniciados com `run` (pelos arquivos .pid)
  teko-node reset  [--dir dev/net]                  apaga SÓ os blocos (votes.blocks, economy.blocks) de cada nó; mantém as chaves e volta aos nós da gênese
  teko-node status [--dir dev/net]                  altura, raiz e pares de cada nó
  teko-node dkg    [--dir dev/net] [--threshold M]  cerimônia de chave da eleição entre os nós EM EXECUÇÃO (ninguém guarda a chave inteira)
  teko-node blocks [--dir dev/net]                  blocos verificados (cliente que não confia em nós)
  teko-node founder [--dir dev/net] --address tekow1…   troca a carteira do fundador (apaga SÓ a cadeia de economia; pare os nós antes)
  teko-node economy [--dir dev/net]                 livro de economia (BFT): altura, fase de fundador, validadores; verifica a cadeia
  teko-node rotation-propose --new <hex>,<hex>,… [--effective-in 6]   propõe um conjunto NOVO de instituições (ordenado); grava rotation.pending; recusa com eleição em andamento
  teko-node rotation-sign    --index I                                  a instituição I aprova a troca pendente com a SUA chave (recusa com eleição em andamento)
  teko-node rotation-submit                                             envia a troca aprovada (mais de 2/3) à chain de votos e à economia
  teko-node rotate --out K [--port P]                                   SÓ DEV: troca a instituição K por uma nova (cria a chave, a pasta e o endereço, aprova e envia); depois: teko-node run --index <novo>
A mainnet está TRAVADA nesta fase.
";

fn flags(args: &[String]) -> Result<HashMap<String, String>, String> {
    let mut m = HashMap::new();
    let mut i = 0;
    while i < args.len() {
        let k = args[i].strip_prefix("--").ok_or("argumento inesperado")?;
        if k == "force" || k == "allow-remote" {
            m.insert(k.to_string(), "1".to_string());
            i += 1;
        } else {
            m.insert(
                k.to_string(),
                args.get(i + 1).ok_or("faltou valor")?.clone(),
            );
            i += 2;
        }
    }
    Ok(m)
}

fn load_pending(dir: &std::path::Path) -> Result<teko_bft::ValidatorSetChange, String> {
    let text = std::fs::read_to_string(dir.join("rotation.pending"))
        .map_err(|_| "não há troca pendente: rode teko-node rotation-propose")?;
    let bytes = teko_crypto::hex::decode(text.trim()).map_err(|_| "rotation.pending corrompido")?;
    <teko_bft::ValidatorSetChange as prost::Message>::decode(bytes.as_slice())
        .map_err(|_| "rotation.pending corrompido".to_string())
}

fn save_pending(dir: &std::path::Path, c: &teko_bft::ValidatorSetChange) -> Result<(), String> {
    std::fs::write(
        dir.join("rotation.pending"),
        format!(
            "{}\n",
            teko_crypto::hex::encode(&prost::Message::encode_to_vec(c))
        ),
    )
    .map_err(|x| x.to_string())
}

/// The economy genesis from the `--founder`, `--issuer`, `--alloc`, `--founder-target`, `--force` flags.
fn economy_from_flags(
    dir: &std::path::Path,
    f: &HashMap<String, String>,
) -> Result<(teko_economy::EconomyGenesis, Option<teko_ledger::Address>), String> {
    let e = |x: teko_node::NodeError| x.to_string();
    let owner = match f.get("founder") {
        Some(a) => Some(
            teko_crypto::parse_wallet_address(a)
                .map_err(|_| "--founder: endereço tekow1… inválido (confira o checksum)")?,
        ),
        None => None,
    };
    let target = match f.get("founder-target") {
        Some(v) => Some(
            teko_crypto::amount::parse_amount(v).map_err(|x| format!("--founder-target: {x}"))?,
        ),
        None => None,
    };
    let alloc = founder_alloc(f, owner.is_some())?;
    // the wallet that is a MEMBER of the credential organs (default: the founder's)
    let issuer = match f.get("issuer") {
        Some(a) => Some(
            teko_crypto::parse_wallet_address(a)
                .map_err(|_| "--issuer: endereço tekow1… inválido (confira o checksum)")?,
        ),
        None => owner,
    };
    let g = teko_node::init_economy_with_issuer(
        dir,
        owner,
        issuer,
        target,
        alloc,
        f.contains_key("force"),
    )
    .map_err(e)?;
    Ok((g, owner))
}

fn run(args: Vec<String>) -> Result<(), String> {
    let Some(cmd) = args.first().map(String::as_str) else {
        print!("{HELP}");
        return Ok(());
    };
    let f = flags(&args[1..])?;
    let dir = PathBuf::from(f.get("dir").map_or("dev/net", String::as_str));
    let e = |x: teko_node::NodeError| x.to_string();
    match cmd {
        "init" => {
            let kind = match f.get("network").map_or("devnet", String::as_str) {
                "devnet" => NetworkKind::Devnet,
                "testnet" => NetworkKind::Testnet,
                "mainnet" => return Err("a mainnet está travada nesta fase".into()),
                _ => return Err("--network: devnet ou testnet".into()),
            };
            let nodes = f
                .get("nodes")
                .map_or(Ok(3), |v| v.parse())
                .map_err(|_| "--nodes inválido")?;
            let port = f
                .get("port")
                .map_or(Ok(7901), |v| v.parse())
                .map_err(|_| "--port inválido")?;
            let n = init_network(&dir, kind, nodes, port, f.contains_key("force")).map_err(e)?;
            let (g, owner) = economy_from_flags(&dir, &f)?;
            match owner {
                Some(a) => {
                    println!(
                        "Economia {}: o fundador (validador 0) manda as recompensas para a carteira {} · a fase de fundador termina com {} de stake total",
                        g.chain_id,
                        teko_crypto::wallet_address(&a).unwrap_or_default(),
                        teko_crypto::amount::format_amount(g.coin.founder_target)
                    );
                    let first: u64 = g.allocations.iter().map(|(_, n)| *n).sum();
                    println!(
                        "Primeiras moedas da rede (TESTE, na gênese): {} para a carteira do fundador",
                        teko_crypto::amount::format_amount(first)
                    );
                }
                None => println!(
                    "AVISO: sem --founder, as recompensas vão para uma conta do nó que ninguém movimenta. Use: make devnet-founder FOUNDER=<seu endereço tekow1…> e make devnet."
                ),
            }
            println!(
                "Rede {} criada em {}: {} validadores (o bloco é final com mais de 2/3: {} de {}), chain_id={}",
                n.profile.kind.name(),
                dir.display(),
                n.profile.validators.len(),
                n.profile.validators.len() * 2 / 3 + 1,
                n.profile.validators.len(),
                n.profile.chain_id
            );
        }
        "keygen" => {
            // each institution makes ITS consensus key on ITS machine; only the public key travels
            let key_file = dir.join("validator.key");
            if key_file.exists() && !f.contains_key("force") {
                let key = teko_node::read_validator_key(&dir).map_err(e)?;
                println!(
                    "A chave já existe em {} (use --force para trocar).\nChave pública (envie esta ao coordenador): {}",
                    key_file.display(),
                    teko_crypto::hex::encode(&teko_crypto::public_key_bytes(&key))
                );
            } else {
                let key = teko_node::generate_validator_key(&dir).map_err(e)?;
                println!(
                    "Chave criada em {} (SECRETA: não saia desta máquina).\nChave pública (envie esta ao coordenador): {}",
                    key_file.display(),
                    teko_crypto::hex::encode(&teko_crypto::public_key_bytes(&key))
                );
            }
        }
        "genesis" => {
            // the coordinator assembles network.txt and peers.txt from the institutions' PUBLIC keys
            let kind = match f.get("network").map_or("testnet", String::as_str) {
                "devnet" => NetworkKind::Devnet,
                "testnet" => NetworkKind::Testnet,
                "mainnet" => return Err("a mainnet está travada nesta fase".into()),
                _ => return Err("--network: devnet ou testnet".into()),
            };
            let file = f.get("validators").ok_or("faltou --validators ARQUIVO")?;
            let text = std::fs::read_to_string(file).map_err(|x| format!("{file}: {x}"))?;
            let n = teko_node::genesis_network(&dir, kind, &text, f.contains_key("force"))
                .map_err(e)?;
            println!(
                "Rede {} montada em {}: {} instituições, o bloco é final com mais de 2/3, chain_id={}.\nDistribua network.txt, peers.txt (e economy.txt, se houver) e CONFIRA o hash do genesis com cada instituição: teko-node genesis-hash --dir {}",
                n.profile.kind.name(),
                dir.display(),
                n.profile.validators.len(),
                n.profile.chain_id,
                dir.display()
            );
        }
        "economy-init" => {
            let (g, owner) = economy_from_flags(&dir, &f)?;
            println!(
                "Economia {} criada (fundador: {}). Distribua economy.txt junto com network.txt e peers.txt e confira o hash do genesis.",
                g.chain_id,
                owner
                    .and_then(|a| teko_crypto::wallet_address(&a).ok())
                    .unwrap_or_else(|| "conta do nó 0".into())
            );
        }
        "genesis-hash" => {
            println!(
                "{}",
                teko_crypto::hex::encode(&teko_node::genesis_hash_of(&dir).map_err(e)?)
            );
        }
        "run" => {
            let net = load_network(&dir).map_err(e)?;
            let index: usize = f
                .get("index")
                .ok_or("faltou --index")?
                .parse()
                .map_err(|_| "--index inválido")?;
            let addr = f
                .get("listen-public")
                .map(|a| {
                    a.parse::<std::net::SocketAddr>()
                        .map_err(|_| "--listen-public inválido")
                })
                .transpose()?
                .or_else(|| net.peers.get(index).copied())
                .ok_or("índice fora da rede")?;
            let listen_peer = f
                .get("listen-peer")
                .map(|a| {
                    a.parse::<std::net::SocketAddr>()
                        .map_err(|_| "--listen-peer inválido")
                })
                .transpose()?;
            let advertise = match (f.get("advertise-peer"), f.get("advertise-public")) {
                (Some(p), Some(q)) => Some((p.clone(), q.clone())),
                (None, None) => None,
                _ => return Err("use --advertise-peer e --advertise-public juntos".into()),
            };
            let key = teko_node::load_validator_key(&dir, index).map_err(e)?;
            let cfg = NodeConfig {
                dir: teko_node::node_dir(&dir, index),
                profile: net.profile.clone(),
                key,
                index,
                links: net.links,
                listen_peer,
                advertise,
                allow_remote: f.contains_key("allow-remote"),
                economy: teko_node::load_economy(&dir).map_err(e)?,
                // `dev/net` sits next to `dev/trust.txt` (governance keys + bound network).
                trust: dir
                    .parent()
                    .map(|p| p.join("trust.txt"))
                    .and_then(|f| std::fs::read_to_string(f).ok())
                    .and_then(|t| teko_attestation::TrustPolicy::parse(&t).ok()),
            };
            let h = Node::start(cfg, addr).map_err(e)?;
            let _ = std::fs::write(
                teko_node::node_dir(&dir, index).join("node.pid"),
                std::process::id().to_string(),
            );
            println!(
                "validador {index} de {} em {} (chain_id {})",
                net.profile.validators.len(),
                h.addr,
                net.profile.chain_id
            );
            loop {
                sleep(Duration::from_secs(3600));
            }
        }
        "dkg" => {
            let net = load_network(&dir).map_err(e)?;
            // the ceremony is among the institutions IN FORCE (after a rotation: the new set in its
            // order, with a session that names the epoch, ADR-0045)
            let plan = teko_node::rotation::ceremony_plan(&dir).map_err(e)?;
            let t = f
                .get("threshold")
                .map(|v| v.parse::<usize>().map_err(|_| "--threshold inválido"))
                .transpose()?
                .unwrap_or(plan.threshold);
            let client = NodeClient::new(net.peers.clone());
            let rec = teko_node::run_ceremony_among(
                &client,
                &plan.participants,
                &dir,
                plan.session.as_bytes(),
                t,
            )
            .map_err(e)?;
            println!(
                "Cerimônia concluída (época {}): chave da eleição de {} validadores, {} de {} decifram.\nChave pública (registrada na cadeia e em {}/tally-key.pub): {}",
                plan.epoch,
                rec.n,
                rec.threshold,
                rec.n,
                dir.display(),
                teko_crypto::hex::encode(&rec.group_key)
            );
        }
        "stop" => {
            let net = load_network(&dir).map_err(e)?;
            for i in 0..net.peers.len() {
                let pid = teko_node::node_dir(&dir, i).join("node.pid");
                if let Ok(text) = std::fs::read_to_string(&pid) {
                    let text = text.trim();
                    if text.chars().all(|c| c.is_ascii_digit()) && !text.is_empty() {
                        let _ = std::process::Command::new("kill").arg(text).status();
                        println!("nó {i} parado");
                    }
                    let _ = std::fs::remove_file(&pid);
                }
            }
        }
        "reset" => {
            let net = load_network(&dir).map_err(e)?;
            for i in 0..net.peers.len() {
                let nd = teko_node::node_dir(&dir, i);
                // the vote chain writes `votes.blocks` and `votes.wal`; `blocks.log` is the old name
                for f in ["blocks.log", "votes.blocks", "votes.wal"] {
                    let _ = std::fs::remove_file(nd.join(f));
                }
                let _ = std::fs::remove_file(nd.join("economy.blocks"));
                let _ = std::fs::remove_file(nd.join("economy.wal"));
                // The key ceremony lives IN the chain being wiped: its shares and the
                // public key mean nothing without it (and would poison a new election).
                let _ = std::fs::remove_file(nd.join("tally-share.key"));
            }
            let _ = std::fs::remove_file(dir.join("tally-key.pub"));
            let _ = std::fs::remove_file(dir.join("rotation.pending"));
            // nodes that joined by a rotation belong to the chain being wiped: back to the genesis set
            let genesis = net.profile.validators.len();
            if net.peers.len() > genesis {
                if let Ok(text) = std::fs::read_to_string(dir.join("peers.txt")) {
                    let kept: Vec<&str> = text
                        .lines()
                        .filter(|l| {
                            l.split_whitespace()
                                .next()
                                .and_then(|i| i.parse::<usize>().ok())
                                .is_none_or(|i| i < genesis)
                        })
                        .collect();
                    let _ = std::fs::write(dir.join("peers.txt"), format!("{}\n", kept.join("\n")));
                }
                for i in genesis..net.peers.len() {
                    let _ = std::fs::remove_dir_all(teko_node::node_dir(&dir, i));
                }
            }
            println!(
                "Cadeia de teste apagada, com a cerimônia da chave (as chaves dos validadores ficaram). Pare os nós antes (`teko-node stop`). Depois: make devnet, make devnet-dkg, make election."
            );
        }
        "status" => {
            let net = load_network(&dir).map_err(e)?;
            let c = NodeClient::new(net.peers);
            println!(
                "rede {} ({})",
                net.profile.kind.name(),
                net.profile.chain_id
            );
            // the institutions in force (verified chain): the epoch and how many
            if let Ok(chain) = c.sync_chain(&net.profile) {
                let e = chain.epoch_at(chain.height() + 1);
                let pending = chain
                    .epochs()
                    .last()
                    .filter(|x| x.from_height > chain.height() + 1)
                    .map_or(String::new(), |x| {
                        format!(
                            " · troca PENDENTE para a época {} no bloco {}",
                            x.epoch, x.from_height
                        )
                    });
                println!(
                    "  instituições em vigor: época {} · {} instituições{}",
                    e.epoch,
                    e.set.len(),
                    pending
                );
            }
            for (i, (a, s)) in c.statuses().into_iter().enumerate() {
                match s {
                    Some(s) => println!(
                        "  nó {i} {a}: altura {} · {} entradas · mempool {} · pares {}/{} · equívocos {}",
                        s.height,
                        s.log_size,
                        s.mempool,
                        s.peers_online,
                        s.peers_total,
                        s.equivocations
                    ),
                    None => println!("  nó {i} {a}: FORA DO AR"),
                }
            }
        }
        "rotation-propose" => {
            let list = f.get("new").ok_or("faltou --new <hex>,<hex>,…")?;
            let keys = list
                .split(',')
                .map(|k| {
                    teko_crypto::hex::decode(k.trim()).map_err(|_| "--new: chave hex inválida")
                })
                .collect::<Result<Vec<_>, _>>()?;
            let n = f
                .get("effective-in")
                .map_or(Ok(teko_node::rotation::DEFAULT_EFFECTIVE_IN), |v| v.parse())
                .map_err(|_| "--effective-in inválido")?;
            let change = teko_node::rotation::propose(&dir, keys, n).map_err(e)?;
            std::fs::write(
                dir.join("rotation.pending"),
                format!(
                    "{}\n",
                    teko_crypto::hex::encode(&prost::Message::encode_to_vec(&change))
                ),
            )
            .map_err(|x| x.to_string())?;
            println!(
                "Proposta da época {} gravada em {}/rotation.pending (vale a partir do bloco {}). Cada instituição aprova: teko-node rotation-sign --index I",
                change.epoch,
                dir.display(),
                change.effective_height
            );
        }
        "rotation-sign" => {
            let i: usize = f
                .get("index")
                .ok_or("faltou --index")?
                .parse()
                .map_err(|_| "--index inválido")?;
            let mut change = load_pending(&dir)?;
            teko_node::rotation::sign(&dir, i, &mut change).map_err(e)?;
            save_pending(&dir, &change)?;
            println!(
                "Instituição {i} aprovou ({} aprovações até agora).",
                change.signers.len()
            );
        }
        "rotation-submit" => {
            let change = load_pending(&dir)?;
            for l in teko_node::rotation::submit(&dir, &change).map_err(e)? {
                println!("  {l}");
            }
            let _ = std::fs::remove_file(dir.join("rotation.pending"));
        }
        "rotate" => {
            let out: usize = f
                .get("out")
                .ok_or("faltou --out K")?
                .parse()
                .map_err(|_| "--out inválido")?;
            let net = load_network(&dir).map_err(e)?;
            let last_port = net.peers.iter().map(|a| a.port()).max().unwrap_or(7900);
            let port: u16 = f
                .get("port")
                .map_or(Ok(last_port + 1), |v| v.parse())
                .map_err(|_| "--port inválido")?;
            let r = teko_node::rotation::dev_rotate(&dir, out, port).map_err(e)?;
            for l in &r.lines {
                println!("  {l}");
            }
            println!(
                "NOVO={} (suba com: teko-node run --index {})",
                r.new_index, r.new_index
            );
        }
        "founder" => {
            let net = load_network(&dir).map_err(e)?;
            let a = f.get("address").ok_or("faltou --address")?;
            let owner = teko_crypto::parse_wallet_address(a)
                .map_err(|_| "endereço tekow1… inválido (confira o checksum)")?;
            let alloc = founder_alloc(&f, true)?;
            teko_node::init_economy(&dir, Some(owner), None, alloc, true).map_err(e)?;
            for i in 0..net.peers.len() {
                let nd = teko_node::node_dir(&dir, i);
                let _ = std::fs::remove_file(nd.join("economy.blocks"));
                let _ = std::fs::remove_file(nd.join("economy.wal"));
            }
            println!(
                "Fundador agora é a carteira {a}. A cadeia de economia foi apagada; suba os nós: make devnet"
            );
        }
        "economy" => {
            let net = load_network(&dir).map_err(e)?;
            let Some(ec) = teko_node::load_economy(&dir).map_err(e)? else {
                return Err("esta rede não tem economia (rode init de novo)".into());
            };
            let c = NodeClient::new(net.peers);
            for (i, a) in c.addrs().iter().enumerate() {
                match c.econ_status(*a) {
                    Ok(s) => println!(
                        "  nó {i} {a}: altura {} · mempool {} · {} · validador: {} · rodada {} · equívocos {}",
                        s.height,
                        s.mempool,
                        if s.founder_phase_active {
                            "FASE DE FUNDADOR"
                        } else {
                            "validadores por stake"
                        },
                        if s.is_validator {
                            "sim"
                        } else {
                            "não (só replica)"
                        },
                        s.round,
                        s.evidence
                    ),
                    Err(_) => println!("  nó {i} {a}: FORA DO AR"),
                }
            }
            let chain = c.econ_sync(&ec.genesis).map_err(e)?;
            let l = chain.ledger();
            println!(
                "cadeia VERIFICADA aqui: {} blocos · stake total {} de {} para encerrar a fase de fundador · {} criadas (só recompensas)",
                chain.height(),
                teko_crypto::amount::format_amount(teko_coin::total_stake(l)),
                teko_crypto::amount::format_amount(ec.genesis.coin.founder_target),
                teko_crypto::amount::format_amount(teko_coin::minted(l))
            );
            for v in teko_coin::validator_set(l) {
                println!(
                    "  validador {}… · dono {}",
                    &teko_crypto::hex::encode(&v.key)[..12],
                    teko_crypto::wallet_address(&v.owner).unwrap_or_default()
                );
            }
        }
        "blocks" => {
            let net = load_network(&dir).map_err(e)?;
            let chain = NodeClient::new(net.peers)
                .sync_chain(&net.profile)
                .map_err(e)?;
            println!(
                "{} blocos verificados (certificado de commit, encadeamento e raiz Merkle):",
                chain.height()
            );
            for d in chain.blocks() {
                let Ok(b) = d.block() else { continue };
                let (precommits, round) = d
                    .commit
                    .as_ref()
                    .map_or((0, 0), |c| (c.precommits.len(), c.round));
                println!(
                    "  #{} · {} entradas · {} precommits · rodada {}",
                    b.height,
                    b.entries.len(),
                    precommits,
                    round
                );
            }
        }
        _ => print!("{HELP}"),
    }
    Ok(())
}

/// Default first coins of a TEST network for the founder wallet (1.000 TEKO, the founder-phase target).
const DEFAULT_FOUNDER_ALLOC: u64 = 1_000 * 1_000_000;

/// `--alloc TEKO`: the first coins of the network, in the genesis, to the founder wallet. Without
/// the flag a founder wallet gets the default (a test network needs coins before any transaction
/// because no block is made on a timer); `--alloc 0` means none.
fn founder_alloc(
    f: &std::collections::HashMap<String, String>,
    has_founder: bool,
) -> Result<Option<u64>, String> {
    match f.get("alloc") {
        Some(v) => teko_crypto::amount::parse_amount(v)
            .map(Some)
            .map_err(|x| format!("--alloc: {x}")),
        None if has_founder => Ok(Some(DEFAULT_FOUNDER_ALLOC)),
        None => Ok(None),
    }
}

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(m) => {
            eprintln!("erro: {m}");
            ExitCode::FAILURE
        }
    }
}
