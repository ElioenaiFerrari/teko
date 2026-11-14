use std::net::{TcpListener, ToSocketAddrs};
use std::path::PathBuf;
use std::process::ExitCode;

use device_emulator::{FileStorage, TerminalHal};
use device_emulator::{LinkSlot, Linked, serve};
use prost::Message;
use rand::rngs::OsRng;
use teko_attestation::{FactoryData, TrustPolicy};
use teko_device_core::{Device, DeviceConfig};

const HELP: &str = "\
device-emulator — dispositivo Teko emulado (tela e botões neste terminal)

USO: device-emulator [--dir dev] [--listen 127.0.0.1:7878] [--label NOME]
  --dir     pasta criada por `issuer-cli dev-setup` (padrão: ./dev)
  --listen  endereço do 'USB' (padrão: 127.0.0.1:7878)
  --state   arquivo de estado do dispositivo (padrão: DIR/device-state.bin)
  --reset   apaga o estado salvo (volta a Virgem) e sai
O PIN e as confirmações são digitados AQUI, nunca na Suite.
";

struct Args {
    dir: PathBuf,
    listen: String,
    label: Option<String>,
    state: Option<PathBuf>,
    reset: bool,
    #[cfg(feature = "test-buttons")]
    auto_pin: Option<String>,
}

fn parse() -> Result<Args, String> {
    let mut a = Args {
        dir: PathBuf::from("dev"),
        listen: "127.0.0.1:7878".into(),
        label: None,
        state: None,
        reset: false,
        #[cfg(feature = "test-buttons")]
        auto_pin: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        let mut val = || it.next().ok_or_else(|| format!("{k} precisa de um valor"));
        match k.as_str() {
            "--dir" => a.dir = PathBuf::from(val()?),
            "--listen" => a.listen = val()?,
            "--label" => a.label = Some(val()?),
            "--state" => a.state = Some(PathBuf::from(val()?)),
            "--reset" => a.reset = true,
            #[cfg(feature = "test-buttons")]
            "--auto-pin" => a.auto_pin = Some(val()?),
            "-h" | "--help" => return Err(HELP.to_string()),
            other => return Err(format!("argumento desconhecido: {other}\n\n{HELP}")),
        }
    }
    Ok(a)
}

fn run() -> Result<(), String> {
    let args = parse()?;
    let state_path = args
        .state
        .clone()
        .unwrap_or_else(|| args.dir.join("device-state.bin"));
    if args.reset {
        let _ = std::fs::remove_file(&state_path);
        println!("Estado apagado: dispositivo virgem.");
        return Ok(());
    }
    let trust_text = std::fs::read_to_string(args.dir.join("trust.txt")).map_err(|_| {
        format!(
            "sem {}/trust.txt — rode `issuer-cli dev-setup` primeiro",
            args.dir.display()
        )
    })?;
    let trust = TrustPolicy::parse(&trust_text).map_err(|e| e.to_string())?;
    let factory = std::fs::read(args.dir.join("factory.bin"))
        .ok()
        .and_then(|b| FactoryData::decode(b.as_slice()).ok());
    if factory.is_none() {
        eprintln!("aviso: sem factory.bin — a atestação não funcionará");
    }
    let mut cfg = DeviceConfig::default();
    if let Some(l) = args.label.clone() {
        cfg.label = l;
    }

    // The device's "USB port" must never be reachable from the network.
    let addrs: Vec<_> = args
        .listen
        .to_socket_addrs()
        .map_err(|e| format!("--listen inválido ({}): {e}", args.listen))?
        .collect();
    if addrs.is_empty() || addrs.iter().any(|a| !a.ip().is_loopback()) {
        return Err(format!(
            "recusado: --listen {} não é um endereço de loopback (use 127.0.0.1)",
            args.listen
        ));
    }
    let listener = TcpListener::bind(&args.listen)
        .map_err(|e| format!("não consegui abrir {}: {e}", args.listen))?;
    let slot = LinkSlot::default();
    let emu = device_emulator::EmuHal {
        term: TerminalHal::new(),
        storage: FileStorage::new(&state_path),
    };

    println!("TEKO — emulador de dispositivo");
    println!(
        "  'USB' em {}   estado: {}",
        args.listen,
        state_path.display()
    );
    println!("  PIN e confirmações são feitos NESTE terminal.\n");

    macro_rules! launch {
        ($hal:expr) => {{
            let hal = Linked::new($hal, slot.clone());
            let mut dev =
                Device::new(hal, OsRng, cfg, trust, factory).map_err(|e| e.to_string())?;
            serve(&listener, &mut dev, &slot, |m| println!("[usb] {m}")).map_err(|e| e.to_string())
        }};
    }

    #[cfg(feature = "test-buttons")]
    if let Some(pin) = args.auto_pin {
        println!("*** MODO TESTE: botões automáticos (test-buttons) ***");
        return launch!(device_emulator::AutoHal { inner: emu, pin });
    }
    launch!(emu)
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(m) => {
            eprintln!("{m}");
            ExitCode::FAILURE
        }
    }
}
