use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use election_admin::Admin;
use teko_election::{Phase, ResultsVisibility};

const HELP: &str = "\
election-admin — administração de eleições (PROTÓTIPO: sem prova ZK nem nullifier)

USO:
  election-admin publish [--dir dev]
      Publica dev/election.bundle no quadro (dev/board.log). Idempotente.
  election-admin simulate [--dir dev] --voters N [--seed S]
      Anexa N eleitores SINTÉTICOS de uma vez (rotula o quadro como 'dados simulados').
  election-admin simulate [--dir dev] --rate R --duration S [--seed S]
      Anexa ~R cédulas/s durante S segundos: abra a tela de Resultados da Suite e veja os gráficos andarem.
  election-admin tally [--dir dev] [--top 10] [--contest ID]
      Apura o quadro e mostra o ranking de cada pleito.
  election-admin close-due [--dir dev] [--now SEGUNDOS]
      (--now simula o relógio, só para teste: um valor no futuro fecha tudo agora.)
      Governança SIMULADA: fecha (assinado 2 de 3) os pleitos cujo prazo venceu; depois disso o quadro não conta mais cédulas deles.
  election-admin reset-board [--dir dev]
      Apaga o quadro e o marcador de simulação.
  election-admin build-election [--dir dev] [--election geral-2026] [--margin-days 30]
      Recria dev/election.bundle com os candidatos das CREDENCIAIS `candidato` ativas na chain (ADR-0040);
      pleitos sem candidaturas ficam com a lista de exemplo.
  election-admin outcomes [--dir dev]
      Mostra os resultados que as instituições assinaram no quadro (quantas assinaturas de quantas).
  election-admin claim-elected [--dir dev]
      Leva à chain de economia o resultado de cada pleito assinado por mais de 2/3 das instituições:
      o eleito recebe a credencial vitalícia `eleito` (e o anterior do mesmo cargo é sucedido).
  election-admin runoff --contest ID [--dir dev]
      Abre o 2º turno (ID-2t) de um pleito cujo resultado foi 'segundo turno'; depois rode `publish`.
";

fn flags(args: &[String]) -> Result<HashMap<String, String>, String> {
    let mut m = HashMap::new();
    let mut i = 0;
    while i < args.len() {
        let k = args[i]
            .strip_prefix("--")
            .ok_or_else(|| format!("argumento inesperado: {}", args[i]))?;
        let v = args
            .get(i + 1)
            .ok_or_else(|| format!("--{k} precisa de um valor"))?;
        m.insert(k.to_string(), v.clone());
        i += 2;
    }
    Ok(m)
}

fn num(f: &HashMap<String, String>, k: &str) -> Result<Option<u64>, String> {
    f.get(k)
        .map(|v| v.parse().map_err(|_| format!("--{k} inválido")))
        .transpose()
}

fn phase_label(p: Phase) -> &'static str {
    match p {
        Phase::Draft => "em preparação",
        Phase::CandidatesPublished => "candidatos publicados",
        Phase::RegistrationOpen => "registro aberto",
        Phase::VotingOpen => "VOTAÇÃO ABERTA",
        Phase::VotingClosed => "votação encerrada",
        Phase::Tallied => "apurada",
        Phase::Audited => "auditada",
        Phase::Cancelled => "cancelada",
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    let Some(cmd) = args.first().map(String::as_str) else {
        print!("{HELP}");
        return Ok(());
    };
    let f = flags(&args[1..])?;
    let dir: PathBuf = f
        .get("dir")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("dev"));
    let e = |x: election_admin::AdminError| x.to_string();
    let admin = || Admin::open(&dir).map_err(|x| x.to_string());

    match cmd {
        "help" | "--help" | "-h" => print!("{HELP}"),
        "build-election" => {
            let election = f.get("election").map_or("geral-2026", String::as_str);
            let days = num(&f, "margin-days")?.unwrap_or(30);
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            for l in election_admin::elected::build_election(&dir, election, days * 86_400, now)? {
                println!("  {l}");
            }
            println!(
                "Eleição `{election}` gravada em {}/election.bundle. Rode: make publish",
                dir.display()
            );
        }
        "outcomes" => {
            let a = admin()?;
            let list = election_admin::elected::outcomes(&a)?;
            if list.is_empty() {
                println!("Nenhum resultado assinado no quadro ainda.");
            }
            for c in list {
                let o = &c.outcome;
                println!(
                    "{} (turno {}): {:?} · {} de {} instituições assinaram{} · votos {:?}",
                    o.contest_id,
                    o.round,
                    o.kind(),
                    c.signers,
                    c.institutions,
                    if c.is_certified() {
                        " (CERTIFICADO)"
                    } else {
                        ""
                    },
                    o.totals
                );
            }
        }
        "claim-elected" => {
            for l in election_admin::elected::claim_elected(&dir)? {
                println!("  {l}");
            }
        }
        "runoff" => {
            let contest = f.get("contest").ok_or("faltou --contest")?;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            let id = election_admin::elected::start_runoff(&dir, contest, now)?;
            println!("2º turno `{id}` criado (revisão do manifesto assinada). Rode: make publish");
        }
        "publish" => {
            let r = admin()?.publish().map_err(e)?;
            println!(
                "Quadro: {} entradas novas, {} já estavam lá.",
                r.appended, r.already_there
            );
        }
        "close-due" => {
            // `--now SECS` SIMULATES the clock (dev only): a far-future value closes
            // every contest at once so the results can be seen without waiting for the deadline.
            let now = num(&f, "now")?.unwrap_or_else(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs())
            });
            let n = admin()?.close_due(now).map_err(e)?;
            println!("{n} pleito(s) fechado(s) pela governança (prazo vencido).");
        }
        "reset-board" => {
            admin()?.reset_board();
            println!("Quadro apagado.");
        }
        "simulate" => {
            let a = admin()?;
            let seed = num(&f, "seed")?.unwrap_or(2026);
            if let Some(rate) = num(&f, "rate")? {
                let secs = num(&f, "duration")?.ok_or("--rate precisa de --duration")?;
                println!(
                    "Simulando ~{rate} cédulas/s por {secs}s (DADOS SIMULADOS). Veja a tela de Resultados da Suite."
                );
                let r = a
                    .simulate_live(rate, Duration::from_secs(secs), seed, |n| {
                        print!("\r  cédulas anexadas: {n}   ");
                        let _ = std::io::Write::flush(&mut std::io::stdout());
                    })
                    .map_err(e)?;
                println!("\nPronto: {} cédulas em {} pleitos.", r.ballots, r.contests);
            } else {
                let voters = num(&f, "voters")?.ok_or("use --voters N ou --rate R --duration S")?;
                let r = a.simulate_batch(voters, seed).map_err(e)?;
                println!(
                    "{} cédulas SINTÉTICAS anexadas em {} pleitos abertos (DADOS SIMULADOS).",
                    r.ballots, r.contests
                );
            }
        }
        "tally" => {
            let a = admin()?;
            let top = num(&f, "top")?.unwrap_or(10) as usize;
            let only = f.get("contest");
            let t = a.tally().map_err(e)?;
            println!("{}", t.title);
            if a.is_simulated() {
                println!("*** DADOS SIMULADOS: nada disto é resultado real ***");
            }
            println!(
                "entradas: {}  ·  governança ignorada: {}  ·  política de resultados: {:?}\n",
                t.entries_seen, t.ignored_governance, t.visibility
            );
            for c in t
                .contests
                .iter()
                .filter(|c| only.is_none_or(|o| o == &c.contest_id))
            {
                let view = c.public_view(t.visibility);
                println!("▌ {}  [{}]", c.title, phase_label(c.phase));
                match (view.accepted, view.distribution) {
                    (None, _) => println!("   (oculto até o fechamento da votação)\n"),
                    (Some(n), false) => {
                        println!("   {n} cédulas (distribuição oculta até o fechamento)\n")
                    }
                    (Some(n), true) => {
                        println!("   {n} cédulas aceitas · {} rejeitadas", c.rejected.total());
                        let r = c.ranked(top);
                        // the percentage is over the votes IN CANDIDATES (ADR-0042)
                        let valid = c.valid_votes();
                        let max = r.top.iter().map(|o| o.votes).max().unwrap_or(0).max(1);
                        for o in &r.top {
                            let bar = "█".repeat((o.votes * 40 / max) as usize);
                            let pct = if valid > 0 {
                                o.votes as f64 * 100.0 / valid as f64
                            } else {
                                0.0
                            };
                            println!("   {:<34} {:>7}  {:>5.1}%  {bar}", o.name, o.votes, pct);
                        }
                        if r.others_count > 0 {
                            println!(
                                "   {:<34} {:>7}",
                                format!("Outros ({})", r.others_count),
                                r.others_votes
                            );
                        }
                        if c.options
                            .iter()
                            .any(|o| !o.kind.eq(&teko_election::OptionKind::Candidate))
                        {
                            println!(
                                "   {:<34} {:>7}   (fora da base da maioria)",
                                format!("Branco {} · Nulo {}", c.blank_votes(), c.null_votes()),
                                c.blank_votes() + c.null_votes()
                            );
                        }
                        println!();
                    }
                }
            }
            let _ = ResultsVisibility::Live;
        }
        other => return Err(format!("comando desconhecido: {other}\n\n{HELP}")),
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
