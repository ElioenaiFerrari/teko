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
";

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
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
