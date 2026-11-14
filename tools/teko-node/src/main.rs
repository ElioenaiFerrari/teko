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
  teko-node init   [--dir dev/net] [--network devnet|testnet] [--nodes 3] [--port 7901] [--force]
  teko-node run    [--dir dev/net] --index I        sobe o validador I (só loopback)
  teko-node stop   [--dir dev/net]                  para os validadores iniciados com `run` (pelos arquivos .pid)
  teko-node reset  [--dir dev/net]                  apaga SÓ os blocos (blocks.log) de cada nó; mantém as chaves
  teko-node status [--dir dev/net]                  altura, raiz e pares de cada nó
  teko-node dkg    [--dir dev/net] [--threshold M]  cerimônia de chave da eleição entre os nós EM EXECUÇÃO (ninguém guarda a chave inteira)
  teko-node blocks [--dir dev/net]                  blocos verificados (cliente que não confia em nós)
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
