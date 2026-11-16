use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use devnet_sim::equivocate;
use teko_crypto::amount::format_amount;

const HELP: &str = "\
devnet-sim: simuladores SÓ para a rede de teste local. Nunca em produção.

USO:
  devnet-sim equivocate [--dir dev/net] [--validator 0]
    Faz o validador de teste assinar dois votos conflitantes (mesma altura e rodada, blocos
    diferentes) e envia a evidência como transação Slash. O livro penaliza o DONO do validador.

  devnet-sim veto --class imovel --asset <id> [--dir dev/net] [--reason \"motivo\"]
    O órgão SIMULADO da classe veta, dentro da janela, a negociação pendente do bem (ADR-0036): o
    preço volta ao comprador e o bem continua com o dono. O motivo é público na cadeia.

  devnet-sim cosign [--type cnh-b] [--dir dev/net]
    Os membros SIMULADOS dos órgãos de credenciais (ADR-0037) aprovam as credenciais que esperam
    aprovação, até completar o quórum. Sua carteira é um dos membros: a sua aprovação vem do app Emissor.
";

fn run_cosign(args: &[String]) -> Result<(), String> {
    let mut dir = PathBuf::from("dev/net");
    let mut ctype: Option<String> = None;
    let mut i = 1;
    while i < args.len() {
        let v = args
            .get(i + 1)
            .ok_or("faltou o valor de uma opção")?
            .clone();
        match args.get(i).map(String::as_str) {
            Some("--dir") => dir = PathBuf::from(v),
            Some("--type") => ctype = Some(v),
            _ => return Err("opção desconhecida (veja --help)".into()),
        }
        i += 2;
    }
    println!(
        "SIMULAÇÃO (rede de teste): membros simulados dos órgãos aprovam credenciais pendentes."
    );
    println!("{}", devnet_sim::cosign(&dir, ctype.as_deref())?);
    Ok(())
}

fn run_veto(args: &[String]) -> Result<(), String> {
    let mut dir = PathBuf::from("dev/net");
    let (mut class, mut asset) = (String::new(), String::new());
    let mut reason = String::from("veto de teste do órgão simulado");
    let mut i = 1;
    while i < args.len() {
        let v = args
            .get(i + 1)
            .ok_or("faltou o valor de uma opção")?
            .clone();
        match args.get(i).map(String::as_str) {
            Some("--dir") => dir = PathBuf::from(v),
            Some("--class") => class = v,
            Some("--asset") => asset = v,
            Some("--reason") => reason = v,
            _ => return Err("opção desconhecida (veja --help)".into()),
        }
        i += 2;
    }
    if class.is_empty() || asset.is_empty() {
        return Err(
            "uso: devnet-sim veto --class <classe> --asset <id> [--reason \"motivo\"]".into(),
        );
    }
    println!("SIMULAÇÃO (rede de teste): o órgão de {class} vai vetar a negociação de {asset}.");
    println!("{}", devnet_sim::veto(&dir, &class, &asset, &reason)?);
    Ok(())
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("veto") {
        return run_veto(&args);
    }
    if args.first().map(String::as_str) == Some("cosign") {
        return run_cosign(&args);
    }
    if args.first().map(String::as_str) != Some("equivocate") {
        print!("{HELP}");
        return Ok(());
    }
    let mut dir = PathBuf::from("dev/net");
    let mut validator = 0usize;
    let mut i = 1;
    while i < args.len() {
        let v = args.get(i + 1).ok_or("faltou o valor de uma opção")?;
        match args.get(i).map(String::as_str) {
            Some("--dir") => dir = PathBuf::from(v),
            Some("--validator") => validator = v.parse().map_err(|_| "--validator inválido")?,
            _ => return Err("opção desconhecida (veja --help)".into()),
        }
        i += 2;
    }
    println!(
        "SIMULAÇÃO (rede de teste): o validador {validator} vai assinar dois votos conflitantes."
    );
    let r = equivocate(&dir, validator, Duration::from_secs(20))?;
    println!("Rede {} · dono do validador: {}", r.chain_id, r.owner);
    println!(
        "1) Evidência: dois prevotes do validador {} na altura {} (blocos diferentes), enviados como Slash",
        r.validator_index, r.offence_height
    );
    println!(
        "   altura da rede: {} → {}",
        r.height_before, r.height_after
    );
    println!(
        "2) Stake do dono: {} → {}  (em saída: {} → {})",
        format_amount(r.stake_before),
        format_amount(r.stake_after),
        format_amount(r.unbonding_before),
        format_amount(r.unbonding_after)
    );
    println!(
        "3) Queimado: {} → {}  (penalidade: {}, {}% do stake em risco)",
        format_amount(r.burned_before),
        format_amount(r.burned_after),
        format_amount(r.penalty()),
        r.slash_bps / 100
    );
    if r.penalty() == 0 {
        println!(
            "\nA ofensa foi registrada, mas a penalidade é 0: o dono não tem stake nem stake em saída. Trave stake na aba Validadores da Suite e rode de novo."
        );
    } else {
        println!("\nVeja no explorador: Eventos → tópico `slashed` (make explorer).");
    }
    if !r.applied {
        return Err("a evidência não foi incluída em um bloco a tempo (a rede está parada ou a evidência é antiga)".into());
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
