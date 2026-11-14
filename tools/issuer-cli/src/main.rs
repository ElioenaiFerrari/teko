use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use issuer_cli::{FW_SIGNERS, Issuer, IssuerError, Subject, make_firmware_blob};
use prost::Message;
use teko_crypto::hex;
use teko_protocol::IdentityRequest;

const HELP: &str = "\
issuer-cli — autoridade simulada do Teko (CHAVES DE TESTE)

USO:
  issuer-cli dev-setup [--dir dev]
      Cria tudo para testar: chaves, trust.txt, factory.bin e firmware v1/v2.
  issuer-cli init [--dir dev] [--force]
  issuer-cli make-firmware --version N --out ARQ [--size 49152]
  issuer-cli sign-firmware [--dir dev] --version N --blob ARQ --signers 0,1 --out ARQ.manifest
  issuer-cli provision [--dir dev] --out factory.bin
  issuer-cli nonce
  issuer-cli dev-election [--dir dev]
      Recria dev/election.bundle (eleição de exemplo com 4 pleitos, datas relativas a agora).
  issuer-cli issue [--dir dev] --nonce HEX --request ARQ --name NOME --birth AAAA-MM-DD --state SP --municipality 3550308
                   --nationality BR --out credencial.bin
  issuer-cli reissue [--dir dev] --nonce HEX --request ARQ --old-civic-id ID [--reason lost|stolen|replaced]
                     --name NOME --birth AAAA-MM-DD --state SP --municipality 3550308 --nationality BR --out credencial.bin
      Reemissão PRESENCIAL (não há seed): reverifica a pessoa, REVOGA a credencial anterior e emite a nova.
  issuer-cli revoke [--dir dev] --civic-id ID [--reason lost|stolen|replaced]
      Bloqueia já um dispositivo perdido/roubado (a pessoa ainda pode pedir reemissão depois).
  issuer-cli dev-forget [--dir dev]
      TESTE: esquece cidadãos/registros/revogações (use junto com `make reset`).
  issuer-cli economy-fund [--dir dev] --address tekow1… --amount N
      TESTE (devnet): aloca N moedas de TESTE ao endereço da carteira (grava dev/economy.txt).
  issuer-cli trust [--dir dev]
  issuer-cli verify-firmware [--dir dev] --manifest ARQ --blob ARQ [--installed N]
      Verificação INDEPENDENTE da Suite: assinaturas M-de-N, hash e anti-downgrade.

--dir padrão: ./dev
";

fn parse_flags(args: &[String]) -> Result<HashMap<String, String>, String> {
    let mut m = HashMap::new();
    let mut i = 0;
    while i < args.len() {
        let key = args[i]
            .strip_prefix("--")
            .ok_or_else(|| format!("argumento inesperado: {}", args[i]))?;
        if key == "force" {
            m.insert(key.to_string(), "1".to_string());
            i += 1;
        } else {
            let val = args
                .get(i + 1)
                .ok_or_else(|| format!("--{key} precisa de um valor"))?;
            m.insert(key.to_string(), val.clone());
            i += 2;
        }
    }
    Ok(m)
}

fn need<'a>(f: &'a HashMap<String, String>, k: &str) -> Result<&'a String, String> {
    f.get(k).ok_or_else(|| format!("faltou --{k}"))
}

fn run(args: Vec<String>) -> Result<(), String> {
    let Some(cmd) = args.first().map(String::as_str) else {
        print!("{HELP}");
        return Ok(());
    };
    let flags = parse_flags(&args[1..])?;
    let dir: PathBuf = flags
        .get("dir")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("dev"));
    let issuer = Issuer::open(&dir);
    let e = |x: IssuerError| x.to_string();
    let io = |x: std::io::Error| x.to_string();

    match cmd {
        "help" | "--help" | "-h" => print!("{HELP}"),
        "dev-setup" => {
            let t = issuer.dev_setup().map_err(e)?;
            println!("Autoridade de teste pronta em {}", dir.display());
            println!("  raiz:         {}", hex::encode(&t.authority_root));
            println!(
                "  firmware:     {}-de-{} (v1 assinado por 0,1; v2 por 1,2)",
                t.fw_threshold,
                t.fw_signers.len()
            );
            println!(
                "  governança:   {}-de-{} (eleições)",
                t.gov_threshold,
                t.gov_signers.len()
            );
            println!(
                "  arquivos:     trust.txt factory.bin firmware-v{{1,2}}.{{blob,manifest}} election.bundle"
            );
            println!("Agora rode: cargo run -p device-emulator   e   cargo run -p teko-suite");
        }
        "init" => {
            let t = issuer.init(flags.contains_key("force")).map_err(e)?;
            println!(
                "Inicializado em {} (raiz {})",
                dir.display(),
                hex::encode(&t.authority_root)
            );
        }
        "trust" => print!("{}", issuer.trust().map_err(e)?.to_text()),
        "make-firmware" => {
            let version: u32 = need(&flags, "version")?
                .parse()
                .map_err(|_| "versão inválida")?;
            let size: usize = flags
                .get("size")
                .map(|s| s.parse().map_err(|_| "tamanho inválido"))
                .transpose()?
                .unwrap_or(48 * 1024);
            fs::write(need(&flags, "out")?, make_firmware_blob(version, size)).map_err(io)?;
        }
        "sign-firmware" => {
            let version: u32 = need(&flags, "version")?
                .parse()
                .map_err(|_| "versão inválida")?;
            let blob = fs::read(need(&flags, "blob")?).map_err(io)?;
            let signers: Vec<usize> = need(&flags, "signers")?
                .split(',')
                .map(|s| {
                    s.trim()
                        .parse::<usize>()
                        .map_err(|_| "signers inválidos".to_string())
                })
                .collect::<Result<_, _>>()?;
            if signers.iter().any(|i| *i >= FW_SIGNERS) {
                return Err(format!("signers devem estar entre 0 e {}", FW_SIGNERS - 1));
            }
            let m = issuer.sign_firmware(version, &blob, &signers).map_err(e)?;
            fs::write(need(&flags, "out")?, m.encode_to_vec()).map_err(io)?;
            println!("Manifesto v{version} assinado por {signers:?}");
        }
        "provision" => {
            let f = issuer.provision_device().map_err(e)?;
            fs::write(need(&flags, "out")?, f.encode_to_vec()).map_err(io)?;
            println!("Material de fábrica gravado (SECRETO: contém a chave de lote)");
        }
        "verify-firmware" => {
            let trust = issuer.trust().map_err(e)?;
            let manifest = teko_attestation::FirmwareManifest::decode(
                fs::read(need(&flags, "manifest")?).map_err(io)?.as_slice(),
            )
            .map_err(|_| "manifesto inválido")?;
            let blob = fs::read(need(&flags, "blob")?).map_err(io)?;
            let installed: u32 = flags
                .get("installed")
                .map(|s| s.parse().map_err(|_| "--installed inválido"))
                .transpose()?
                .unwrap_or(0);
            let valid = teko_attestation::count_valid_signers(&trust, &manifest);
            println!("versão:            v{}", manifest.version);
            println!(
                "impressão digital: {}",
                teko_attestation::manifest_fingerprint(&manifest)
            );
            println!(
                "assinaturas:       {valid} válidas de {} exigidas ({} instituições)",
                trust.fw_threshold,
                trust.fw_signers.len()
            );
            teko_attestation::verify_blob(&manifest, &teko_attestation::blob_hash(&blob))
                .map_err(|_| "FALHOU: o arquivo não corresponde ao hash do manifesto")?;
            teko_attestation::verify_manifest(&trust, &manifest, installed)
                .map_err(|x| format!("FALHOU: {x}"))?;
            println!("OK: firmware aceitável (M-de-N válido, hash confere, sem downgrade)");
        }
        "economy-fund" => {
            let address = flags
                .get("address")
                .ok_or_else(|| "--address é obrigatório".to_string())?;
            let amount: u64 = flags
                .get("amount")
                .ok_or_else(|| "--amount é obrigatório".to_string())?
                .parse()
                .map_err(|_| "--amount inválido".to_string())?;
            let path = dir.join(teko_coin::devnet::ECONOMY_FILE);
            teko_coin::devnet::append_allocation(&path, address, amount)?;
            println!(
                "Alocadas {amount} moedas de TESTE (devnet) para {address} em {}",
                path.display()
            );
        }
        "dev-election" => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            issuer.dev_election(now).map_err(e)?;
            println!(
                "Eleição de exemplo gravada em {}/election.bundle",
                dir.display()
            );
        }
        "nonce" => println!("{}", hex::encode(&Issuer::new_nonce())),
        "issue" => {
            let nonce = hex::decode(need(&flags, "nonce")?).map_err(|_| "nonce inválido")?;
            let req =
                IdentityRequest::decode(fs::read(need(&flags, "request")?).map_err(io)?.as_slice())
                    .map_err(|_| "arquivo de pedido inválido")?;
            let subject = Subject {
                name: need(&flags, "name")?.clone(),
                birth_date: need(&flags, "birth")?.clone(),
                nationality: need(&flags, "nationality")?.clone(),
                electoral_state: need(&flags, "state")?.clone(),
                electoral_municipality: need(&flags, "municipality")?.clone(),
            };
            let issued = issuer.issue_credential(&nonce, &req, &subject).map_err(e)?;
            fs::write(need(&flags, "out")?, issued.encode_to_vec()).map_err(io)?;
            println!("Credencial emitida e vinculada à chave de identidade do dispositivo");
        }
        "reissue" => {
            let nonce = hex::decode(need(&flags, "nonce")?).map_err(|_| "nonce inválido")?;
            let req =
                IdentityRequest::decode(fs::read(need(&flags, "request")?).map_err(io)?.as_slice())
                    .map_err(|_| "arquivo de pedido inválido")?;
            let subject = Subject {
                name: need(&flags, "name")?.clone(),
                birth_date: need(&flags, "birth")?.clone(),
                nationality: need(&flags, "nationality")?.clone(),
                electoral_state: need(&flags, "state")?.clone(),
                electoral_municipality: need(&flags, "municipality")?.clone(),
            };
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            let reason = flags.get("reason").map_or("lost", String::as_str);
            let issued = issuer
                .reissue_credential_at(
                    &nonce,
                    &req,
                    &subject,
                    need(&flags, "old-civic-id")?,
                    reason,
                    now,
                )
                .map_err(e)?;
            fs::write(need(&flags, "out")?, issued.encode_to_vec()).map_err(io)?;
            println!(
                "Reemissão feita: a credencial anterior foi REVOGADA e a nova está vinculada ao mesmo cidadão"
            );
        }
        "revoke" => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            let reason = flags.get("reason").map_or("lost", String::as_str);
            let n = issuer
                .revoke_civic_id(need(&flags, "civic-id")?, reason, now)
                .map_err(e)?;
            println!("{n} credencial(is) revogada(s); lista assinada atualizada");
        }
        "dev-forget" => {
            issuer.dev_forget_citizens();
            println!("Cadastro de cidadãos, registros e revogações de TESTE apagados.");
        }
        other => return Err(format!("comando desconhecido: {other}\n\n{HELP}")),
    }
    Ok(())
}

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("erro: {msg}");
            ExitCode::FAILURE
        }
    }
}
