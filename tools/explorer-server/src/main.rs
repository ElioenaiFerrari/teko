use std::collections::HashMap;
use std::net::IpAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use explorer_server::{Shared, handle, spawn_sync, sync_once};
use teko_explorer::Index;
use teko_node::{NodeClient, load_economy, load_network};

const HELP: &str = "\
teko-explorer: explorador da chain (somente leitura). PROTÓTIPO.

USO:
  teko-explorer [--dir dev/net] [--port 8545] [--bind 127.0.0.1] [--public] [--metadata-dir PASTA]

  --dir     pasta da rede (a mesma de `teko-node`)
  --bind    endereço de escuta; só loopback por padrão
  --metadata-dir  pasta com os registros de metadados dos bens (padrão em devnet: <dir>/assets-metadata; fora da devnet, nenhum)
  --public  permite --bind fora de loopback (use atrás de um proxy com HTTPS: a API é só leitura, mas pública)
A chain é baixada dos nós e VERIFICADA aqui (certificados e raízes de estado): o explorador não confia nos nós.
";

fn flags(args: &[String]) -> Result<HashMap<String, String>, String> {
    let mut m = HashMap::new();
    let mut i = 0;
    while i < args.len() {
        let k = args
            .get(i)
            .and_then(|a| a.strip_prefix("--"))
            .ok_or("argumento inesperado")?;
        if k == "public" || k == "help" {
            m.insert(k.to_string(), "1".to_string());
            i += 1;
        } else {
            let v = args
                .get(i + 1)
                .ok_or_else(|| format!("--{k} precisa de valor"))?;
            m.insert(k.to_string(), v.clone());
            i += 2;
        }
    }
    Ok(m)
}

fn header(name: &str, value: &str) -> Option<tiny_http::Header> {
    tiny_http::Header::from_bytes(name.as_bytes(), value.as_bytes()).ok()
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let f = flags(&args)?;
    if f.contains_key("help") {
        print!("{HELP}");
        return Ok(());
    }
    let dir = PathBuf::from(f.get("dir").map_or("dev/net", String::as_str));
    let port: u16 = f
        .get("port")
        .map_or(Ok(8545), |v| v.parse())
        .map_err(|_| "--port inválido")?;
    let bind: IpAddr = f
        .get("bind")
        .map_or("127.0.0.1", String::as_str)
        .parse()
        .map_err(|_| "--bind inválido")?;
    if !bind.is_loopback() && !f.contains_key("public") {
        return Err(
            "--bind fora de loopback exige --public (e um proxy com HTTPS na frente)".into(),
        );
    }

    let net = load_network(&dir).map_err(|e| e.to_string())?;
    let ec = load_economy(&dir)
        .map_err(|e| e.to_string())?
        .ok_or("esta rede não tem economia (rode `teko-node init`)")?;
    let index = Index::new(ec.genesis).map_err(|e| e.to_string())?;
    let mut shared = Shared::new(index, net.profile.kind.name())
        .with_votes(net.profile.clone(), net.links.clone());
    // the records hold the owners' data: only a local devnet (or an explicit folder) shows them
    let meta_dir = f
        .get("metadata-dir")
        .map(PathBuf::from)
        .or_else(|| (shared.network == "devnet").then(|| dir.join("assets-metadata")));
    if let Some(m) = meta_dir {
        shared = shared.with_store(teko_assets::MetadataStore::new(m));
    }
    let shared = Arc::new(shared);
    let client = NodeClient::new(net.peers);
    sync_once(&shared, &client);
    spawn_sync(Arc::clone(&shared), client, Duration::from_secs(2));

    let server = Arc::new(
        tiny_http::Server::http((bind, port))
            .map_err(|e| format!("não foi possível escutar: {e}"))?,
    );
    println!(
        "Teko Explorer em http://{bind}:{port}  (rede {}; Ctrl+C para sair)",
        shared.network
    );
    let mut workers = Vec::new();
    for _ in 0..4 {
        let (server, shared) = (Arc::clone(&server), Arc::clone(&shared));
        workers.push(thread::spawn(move || {
            while let Ok(rq) = server.recv() {
                let r = handle(&shared, rq.method().as_str(), rq.url());
                let mut resp = tiny_http::Response::from_data(r.body).with_status_code(r.status);
                let api = rq.url().starts_with("/api/");
                for (n, v) in [
                    ("Content-Type", r.content_type),
                    ("X-Content-Type-Options", "nosniff"),
                    ("Referrer-Policy", "no-referrer"),
                    ("X-Frame-Options", "DENY"),
                    ("Content-Security-Policy", "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'"),
                    ("Cache-Control", if api { "no-store" } else { "no-cache" }),
                ] {
                    if let Some(h) = header(n, v) {
                        resp.add_header(h);
                    }
                }
                let _ = rq.respond(resp);
            }
        }));
    }
    for w in workers {
        let _ = w.join();
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("erro: {e}");
            ExitCode::FAILURE
        }
    }
}
