//! Request handling, independent of the HTTP library so it can be tested directly.

use serde::Serialize;

use crate::state::Shared;

const INDEX_HTML: &str = include_str!("../ui/index.html");
const APP_JS: &str = include_str!("../ui/app.js");
const STYLES_CSS: &str = include_str!("../ui/styles.css");

pub struct Response {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

fn json<T: Serialize>(status: u16, v: &T) -> Response {
    Response {
        status,
        content_type: "application/json; charset=utf-8",
        body: serde_json::to_vec(v).unwrap_or_else(|_| b"{}".to_vec()),
    }
}

fn error(status: u16, msg: &str) -> Response {
    #[derive(Serialize)]
    struct E<'a> {
        error: &'a str,
    }
    json(status, &E { error: msg })
}

fn asset(content_type: &'static str, body: &str) -> Response {
    Response {
        status: 200,
        content_type,
        body: body.as_bytes().to_vec(),
    }
}

/// Percent-decoding of a query value (`%C3%A7`, `+`).
fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b.get(i) {
            Some(b'+') => {
                out.push(b' ');
                i += 1;
            }
            Some(b'%') => {
                let hex = s
                    .get(i + 1..i + 3)
                    .and_then(|h| u8::from_str_radix(h, 16).ok());
                match hex {
                    Some(v) => {
                        out.push(v);
                        i += 3;
                    }
                    None => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            Some(c) => {
                out.push(*c);
                i += 1;
            }
            None => break,
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn query_of(q: &str) -> Vec<(String, String)> {
    q.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| match p.split_once('=') {
            Some((k, v)) => (decode(k), decode(v)),
            None => (decode(p), String::new()),
        })
        .collect()
}

fn param<'a>(q: &'a [(String, String)], k: &str) -> Option<&'a str> {
    q.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str())
}

fn limit(q: &[(String, String)]) -> Result<usize, Response> {
    match param(q, "limit") {
        None => Ok(20),
        Some(v) => v
            .parse::<usize>()
            .ok()
            .filter(|n| (1..=100).contains(n))
            .ok_or_else(|| error(400, "limit deve estar entre 1 e 100")),
    }
}

/// Routes one request. Only `GET` exists: the API never changes anything.
pub fn handle(shared: &Shared, method: &str, url: &str) -> Response {
    if method != "GET" && method != "HEAD" {
        return error(405, "só GET: a API é somente leitura");
    }
    let (path, query) = url.split_once('?').unwrap_or((url, ""));
    let q = query_of(query);
    let segs: Vec<&str> = path.trim_matches('/').split('/').collect();
    match segs.as_slice() {
        [""] | ["index.html"] => asset("text/html; charset=utf-8", INDEX_HTML),
        ["app.js"] => asset("text/javascript; charset=utf-8", APP_JS),
        ["styles.css"] => asset("text/css; charset=utf-8", STYLES_CSS),
        ["api", "v1", rest @ ..] => api(shared, rest, &q),
        _ => error(404, "rota não encontrada"),
    }
}

fn api(shared: &Shared, segs: &[&str], q: &[(String, String)]) -> Response {
    match segs {
        ["status"] => {
            #[derive(Serialize)]
            struct S {
                chain: teko_explorer::StatusView,
                network: String,
                meta: crate::state::Meta,
            }
            let chain = shared.index().status();
            json(
                200,
                &S {
                    chain,
                    network: shared.network.clone(),
                    meta: shared.meta().clone(),
                },
            )
        }
        ["blocks"] => {
            let n = match limit(q) {
                Ok(n) => n,
                Err(r) => return r,
            };
            let before = match param(q, "before").map(str::parse::<u64>) {
                None => None,
                Some(Ok(b)) => Some(b),
                Some(Err(_)) => return error(400, "before deve ser uma altura"),
            };
            json(200, &shared.index().blocks(before, n))
        }
        ["blocks", id] => {
            let ix = shared.index();
            let b = id
                .parse::<u64>()
                .ok()
                .and_then(|h| ix.block(h))
                .or_else(|| ix.block_by_hash(id));
            match b {
                Some(b) => json(200, b),
                None => error(404, "bloco não encontrado"),
            }
        }
        ["txs"] => {
            let n = match limit(q) {
                Ok(n) => n,
                Err(r) => return r,
            };
            json(200, &shared.index().latest_txs(n))
        }
        ["txs", id] => match shared.index().tx(id) {
            Some(t) => json(200, t),
            None => error(404, "transação não encontrada"),
        },
        ["accounts", address] => match shared.index().account(address) {
            Some(a) => json(200, &a),
            None => error(400, "endereço inválido (esperado tekow1…, com checksum)"),
        },
        ["votes"] => {
            let v = shared.votes();
            json(200, &*v)
        }
        ["votes", "blocks", h] => {
            let v = shared.votes();
            match h
                .parse::<u64>()
                .ok()
                .and_then(|h| v.blocks.iter().find(|b| b.height == h))
            {
                Some(b) => json(200, b),
                None => error(404, "bloco de votos não encontrado"),
            }
        }
        ["validators"] => json(200, &shared.index().validators()),
        ["assets"] => {
            #[derive(Serialize)]
            struct A {
                classes: Vec<teko_explorer::ClassView>,
                assets: Vec<teko_explorer::AssetView>,
            }
            let ix = shared.index();
            json(
                200,
                &A {
                    classes: ix.classes(),
                    assets: ix.assets(),
                },
            )
        }
        ["events"] => {
            let n = match limit(q) {
                Ok(n) => n,
                Err(r) => return r,
            };
            json(200, &shared.index().events(param(q, "topic"), n))
        }
        ["search"] => match param(q, "q") {
            Some(v) if v.len() <= 200 => json(200, &shared.index().search(v)),
            Some(_) => error(400, "busca longa demais"),
            None => error(400, "informe ?q="),
        },
        _ => error(404, "rota não encontrada"),
    }
}
