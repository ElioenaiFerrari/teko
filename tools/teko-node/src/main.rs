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
  teko-node init   [--dir dev/net] [--network devnet|testnet] [--nodes 3] [--port 7901] [--founder tekow1…] [--founder-target TEKO] [--force]
                   --founder: endereço da CARTEIRA (do dispositivo) que recebe as recompensas do fundador (opção B, ADR-0028)
  teko-node run    [--dir dev/net] --index I        sobe o validador I (só loopback)
  teko-node stop   [--dir dev/net]                  para os validadores iniciados com `run` (pelos arquivos .pid)
  teko-node reset  [--dir dev/net]                  apaga SÓ os blocos (blocks.log) de cada nó; mantém as chaves
  teko-node status [--dir dev/net]                  altura, raiz e pares de cada nó
  teko-node dkg    [--dir dev/net] [--threshold M]  cerimônia de chave da eleição entre os nós EM EXECUÇÃO (ninguém guarda a chave inteira)
  teko-node blocks [--dir dev/net]                  blocos verificados (cliente que não confia em nós)
  teko-node founder [--dir dev/net] --address tekow1…   troca a carteira do fundador (apaga SÓ a cadeia de economia; pare os nós antes)
  teko-node economy [--dir dev/net]                 livro de economia (BFT): altura, fase de fundador, validadores; verifica a cadeia
A mainnet está TRAVADA nesta fase.
";

fn flags(args: &[String]) -> Result<HashMap<String, String>, String> {
    let mut m = HashMap::new();
    let mut i = 0;
    while i < args.len() {
        let k = args[i].strip_prefix("--").ok_or("argumento inesperado")?;
        if k == "force" {
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
            let owner = match f.get("founder") {
                Some(a) => Some(
                    teko_crypto::parse_wallet_address(a)
                        .map_err(|_| "--founder: endereço tekow1… inválido (confira o checksum)")?,
                ),
                None => None,
            };
            let target = match f.get("founder-target") {
                Some(v) => Some(
                    teko_crypto::amount::parse_amount(v)
                        .map_err(|x| format!("--founder-target: {x}"))?,
                ),
                None => None,
            };
            let g =
                teko_node::init_economy(&dir, owner, target, f.contains_key("force")).map_err(e)?;
            match owner {
                Some(a) => println!(
                    "Economia {}: o fundador (validador 0) manda as recompensas para a carteira {} · a fase de fundador termina com {} de stake total",
                    g.chain_id,
                    teko_crypto::wallet_address(&a).unwrap_or_default(),
                    teko_crypto::amount::format_amount(g.coin.founder_target)
                ),
                None => println!(
                    "AVISO: sem --founder, as recompensas vão para uma conta do nó que ninguém movimenta. Use: make devnet-reset FOUNDER=<seu endereço tekow1…> e make devnet."
                ),
            }
            println!(
                "Rede {} criada em {}: {} validadores, {} de {} assinam cada bloco, chain_id={}",
                n.profile.kind.name(),
                dir.display(),
                n.profile.validators.len(),
                n.profile.threshold,
                n.profile.validators.len(),
                n.profile.chain_id
            );
        }
        "run" => {
            let net = load_network(&dir).map_err(e)?;
            let index: usize = f
                .get("index")
                .ok_or("faltou --index")?
                .parse()
                .map_err(|_| "--index inválido")?;
            let addr = *net.peers.get(index).ok_or("índice fora da rede")?;
            let key = teko_node::load_validator_key(&dir, index).map_err(e)?;
            let cfg = NodeConfig {
                dir: teko_node::node_dir(&dir, index),
                profile: net.profile.clone(),
                key,
                index,
                peers: net.peers,
                allow_remote: false,
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
            let t = f
                .get("threshold")
                .map_or(Ok(net.profile.threshold), |v| v.parse())
                .map_err(|_| "--threshold inválido")?;
            let session = format!("{}|tally-key", net.profile.chain_id);
            let rec =
                teko_node::run_ceremony(&NodeClient::new(net.peers), &dir, session.as_bytes(), t)
                    .map_err(e)?;
            println!(
                "Cerimônia concluída: chave da eleição de {} validadores, {} de {} decifram.\nChave pública (registrada na cadeia e em {}/tally-key.pub): {}",
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
                let _ = std::fs::remove_file(nd.join("blocks.log"));
                let _ = std::fs::remove_file(nd.join("economy.blocks"));
                let _ = std::fs::remove_file(nd.join("economy.wal"));
                // The key ceremony lives IN the chain being wiped: its shares and the
                // public key mean nothing without it (and would poison a new election).
                let _ = std::fs::remove_file(nd.join("tally-share.key"));
            }
            let _ = std::fs::remove_file(dir.join("tally-key.pub"));
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
        "founder" => {
            let net = load_network(&dir).map_err(e)?;
            let a = f.get("address").ok_or("faltou --address")?;
            let owner = teko_crypto::parse_wallet_address(a)
                .map_err(|_| "endereço tekow1… inválido (confira o checksum)")?;
            teko_node::init_economy(&dir, Some(owner), None, true).map_err(e)?;
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
                "{} blocos verificados (assinaturas, encadeamento e raiz Merkle):",
                chain.height()
            );
            for b in chain.blocks() {
                println!(
                    "  #{} · {} entradas · {} assinaturas · rodada {}",
                    b.height,
                    b.entries.len(),
                    b.signatures.len(),
                    b.round
                );
            }
        }
        _ => print!("{HELP}"),
    }
    Ok(())
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
