//! Teko Emissor: o app em que órgãos e comunidades autorizados emitem credenciais na chain
//! (ADR-0037). O computador é NÃO CONFIÁVEL: este app nunca vê o PIN nem guarda chaves, e cada
//! operação é montada, mostrada e assinada NO DISPOSITIVO (emulador) do membro do órgão.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;

use std::sync::{Arc, Mutex};

use commands::{Issuer, IssuerInner};

fn main() {
    let addr =
        std::env::var("TEKO_DEVICE_ADDR").unwrap_or_else(|_| teko_client::DEFAULT_ADDR.to_string());
    let issuer: Issuer = Arc::new(IssuerInner {
        client: Mutex::new(None),
        addr,
    });
    let result = tauri::Builder::default()
        .manage(issuer)
        .invoke_handler(tauri::generate_handler![
            commands::device_status,
            commands::unlock,
            commands::overview,
            commands::holder_key,
            commands::issue,
            commands::vote,
        ])
        .run(tauri::generate_context!());
    if let Err(e) = result {
        eprintln!("erro ao iniciar o Teko Emissor: {e}");
        std::process::exit(1);
    }
}
