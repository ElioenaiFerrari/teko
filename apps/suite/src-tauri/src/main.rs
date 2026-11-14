//! Teko Suite — desktop companion. The host is UNTRUSTED by design: this app
//! never sees the PIN, never holds keys, and every sensitive action is
//! confirmed on the device (emulator).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;

use std::sync::{Arc, Mutex};

use commands::{Suite, SuiteInner};

fn main() {
    let addr =
        std::env::var("TEKO_DEVICE_ADDR").unwrap_or_else(|_| teko_client::DEFAULT_ADDR.to_string());
    let suite: Suite = Arc::new(SuiteInner {
        client: Mutex::new(None),
        addr,
    });
    let result = tauri::Builder::default()
        .manage(suite)
        .invoke_handler(tauri::generate_handler![
            commands::device_status,
            commands::dev_info,
            commands::verify_authenticity,
            commands::firmware_catalog,
            commands::install_firmware,
            commands::setup_pin,
            commands::unlock,
            commands::civic_id,
            commands::issue_credential,
            commands::reveal_identity,
            commands::pairwise_id,
            commands::hash_file,
            commands::sign_document,
            commands::save_signed_document,
            commands::verify_document,
            commands::prove_attribute,
            commands::cast_vote,
            commands::wipe,
        ])
        .run(tauri::generate_context!());
    if let Err(e) = result {
        eprintln!("erro ao iniciar a Teko Suite: {e}");
        std::process::exit(1);
    }
}
