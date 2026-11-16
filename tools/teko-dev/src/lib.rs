//! `teko-dev`: a menu (arrow keys) over the `make` targets, for DEVELOPMENT only. It is a shell
//! around the Makefile, which stays the single source of truth for every command; this crate holds
//! the testable part (the catalog, parameter validation, command assembly and the test network
//! state). The terminal code lives in `main.rs`.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

/// One `make` variable a command takes.
#[derive(Debug)]
pub struct Param {
    pub key: &'static str,
    pub label: &'static str,
    /// Pre-filled value; an empty value on an optional parameter is simply not passed.
    pub default: &'static str,
    pub required: bool,
    pub numeric: bool,
    /// When non-empty, only these values are accepted (the form cycles through them).
    pub choices: &'static [&'static str],
}

const fn p(key: &'static str, label: &'static str, default: &'static str) -> Param {
    Param {
        key,
        label,
        default,
        required: false,
        numeric: false,
        choices: &[],
    }
}

const fn req(key: &'static str, label: &'static str, default: &'static str) -> Param {
    Param {
        required: true,
        ..p(key, label, default)
    }
}

const fn num(key: &'static str, label: &'static str, default: &'static str) -> Param {
    Param {
        numeric: true,
        ..p(key, label, default)
    }
}

/// When a command makes sense, given the state of the test network.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum When {
    Any,
    /// The network is not running (not created, or stopped).
    NetNotUp,
    NetUp,
}

/// How a command is run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Output goes to a scrollable panel.
    Capture,
    /// The command owns the terminal (PIN prompts, GUI apps, servers); the menu returns after it.
    Terminal,
}

#[derive(Debug)]
pub struct Item {
    pub group: &'static str,
    pub label: &'static str,
    /// `make` targets, run in order (every parameter is passed to every step).
    pub steps: &'static [&'static str],
    pub params: &'static [Param],
    pub when: When,
    pub mode: Mode,
    pub note: &'static str,
}

const fn item(
    group: &'static str,
    label: &'static str,
    steps: &'static [&'static str],
    params: &'static [Param],
    when: When,
    mode: Mode,
    note: &'static str,
) -> Item {
    Item {
        group,
        label,
        steps,
        params,
        when,
        mode,
        note,
    }
}

use Mode::{Capture as C, Terminal as T};
use When::{Any, NetNotUp, NetUp};

const KINDS: &[&str] = &["economy", "votes"];

static P_DEVNET: [Param; 3] = [
    p("FOUNDER", "Carteira do fundador (tekow1…, na criação)", ""),
    p("ISSUER", "Carteira do emissor (opcional)", ""),
    num("NODES", "Instituições (só vale na criação; padrão 4)", ""),
];
static P_FOUNDER: [Param; 2] = [
    req("FOUNDER", "Carteira do fundador (tekow1…)", ""),
    p("ALLOC", "Alocação inicial (opcional)", ""),
];
static P_BLOCKS: [Param; 2] = [
    Param {
        choices: KINDS,
        ..req("KIND", "Chain (economy ou votes; ←/→)", "economy")
    },
    Param {
        numeric: true,
        ..req("COUNT", "Quantos blocos", "5")
    },
];
static P_SIMULATE: [Param; 1] = [num("N", "Eleitores sintéticos", "5000")];
static P_LIVE: [Param; 2] = [
    num("RATE", "Cédulas por segundo", "15"),
    num("SECS", "Duração (s)", "120"),
];
static P_ELECTION: [Param; 1] = [p("ELECTION", "Id da eleição (opcional)", "")];
static P_RUNOFF: [Param; 1] = [req("CONTEST", "Pleito (ex.: governador-sp)", "")];
static P_ROTATE: [Param; 1] = [num(
    "OUT",
    "Instituição a trocar (posição no conjunto, padrão 3)",
    "3",
)];
static P_TIE: [Param; 2] = [
    req("CONTEST", "Pleito aberto (ex.: governador-sp)", ""),
    p(
        "VOTES",
        "Votos por opção, separados por vírgula (5,5 empata as 2 primeiras)",
        "5,5",
    ),
];
static P_COSIGN: [Param; 1] = [p("TYPE", "Tipo de credencial (opcional, ex.: cnh-b)", "")];
static P_VETO: [Param; 2] = [
    req("CLASS", "Classe do bem", "imovel"),
    req("ASSET", "Id do bem", ""),
];
static P_EQUIV: [Param; 1] = [num("VALIDATOR", "Índice do validador", "0")];
static P_FUND: [Param; 2] = [
    req("ADDR", "Carteira (tekow1…)", ""),
    req("AMOUNT", "TEKO (decimais com , ou .)", "1000"),
];
static P_PIN: [Param; 1] = [num("PIN", "PIN automático", "482913")];

/// Every command, by group. A test keeps this in sync with the Makefile.
pub static CATALOG: &[Item] = &[
    item(
        "Devnet",
        "Subir a devnet",
        &["devnet"],
        &P_DEVNET,
        NetNotUp,
        C,
        "cria a rede na 1ª vez; os nós rodam em segundo plano",
    ),
    item(
        "Devnet",
        "Definir o fundador",
        &["devnet-founder"],
        &P_FOUNDER,
        NetNotUp,
        C,
        "zera a economia",
    ),
    item(
        "Devnet",
        "Resetar os blocos (mantém as chaves)",
        &["devnet-reset"],
        &[],
        NetNotUp,
        C,
        "",
    ),
    item(
        "Devnet",
        "Derrubar a devnet",
        &["devnet-stop"],
        &[],
        NetUp,
        C,
        "",
    ),
    item(
        "Devnet",
        "Reiniciar a devnet",
        &["devnet-stop", "devnet"],
        &[],
        NetUp,
        C,
        "derruba e sobe de novo",
    ),
    item(
        "Devnet",
        "Trocar uma instituição (rotação, ADR-0045)",
        &["devnet-rotate"],
        &P_ROTATE,
        NetUp,
        C,
        "só dev: a nova entra com a aprovação das outras; recusa com eleição aberta",
    ),
    item(
        "Devnet",
        "Status (altura, mempool, pares)",
        &["devnet-status"],
        &[],
        NetUp,
        C,
        "",
    ),
    item(
        "Devnet",
        "Economia (altura, fundador, validadores)",
        &["devnet-economy"],
        &[],
        NetUp,
        C,
        "",
    ),
    item(
        "Devnet",
        "Listar blocos verificados",
        &["devnet-blocks"],
        &[],
        NetUp,
        C,
        "",
    ),
    item(
        "Devnet",
        "Cerimônia de chave (DKG)",
        &["devnet-dkg"],
        &[],
        NetUp,
        C,
        "chave de decifração da eleição",
    ),
    item(
        "Devnet",
        "Explorador web (:8545)",
        &["explorer"],
        &[],
        NetUp,
        T,
        "Ctrl-C volta ao menu",
    ),
    item(
        "Simular blocos",
        "Criar N blocos",
        &["devnet-blocks-sim"],
        &P_BLOCKS,
        NetUp,
        C,
        "só existem blocos com atividade (ADR-0034); em votos o quadro cresce",
    ),
    item(
        "Eleição",
        "Candidaturas de exemplo (órgão simulado)",
        &["candidates"],
        &P_ELECTION,
        NetUp,
        C,
        "só dev",
    ),
    item(
        "Eleição",
        "Montar a eleição",
        &["election"],
        &P_ELECTION,
        Any,
        C,
        "",
    ),
    item(
        "Eleição",
        "Publicar no quadro",
        &["publish"],
        &[],
        Any,
        C,
        "idempotente",
    ),
    item(
        "Eleição",
        "Simular N eleitores",
        &["simulate"],
        &P_SIMULATE,
        Any,
        C,
        "compila em release na 1ª vez",
    ),
    item(
        "Eleição",
        "Simular votos ao vivo",
        &["simulate-live"],
        &P_LIVE,
        Any,
        C,
        "",
    ),
    item(
        "Eleição",
        "Forçar um empate (plantar votos)",
        &["tie"],
        &P_TIE,
        Any,
        C,
        "só dev: depois feche e veja o sorteio verificável nos resultados",
    ),
    item(
        "Eleição",
        "Fechar todos os pleitos agora",
        &["close"],
        &[],
        Any,
        C,
        "",
    ),
    item("Eleição", "Apurar (top 10)", &["tally"], &[], Any, C, ""),
    item(
        "Eleição",
        "Apurar com tempos (release)",
        &["tally-time"],
        &[],
        Any,
        C,
        "",
    ),
    item(
        "Eleição",
        "Eleitos recebem a credencial",
        &["claim"],
        &[],
        NetUp,
        C,
        "",
    ),
    item(
        "Eleição",
        "Abrir o 2º turno",
        &["runoff"],
        &P_RUNOFF,
        NetUp,
        C,
        "",
    ),
    item(
        "Eleição",
        "Zerar o quadro",
        &["reset-board"],
        &[],
        Any,
        C,
        "",
    ),
    item(
        "Credenciais e bens",
        "Membros simulados aprovam credenciais",
        &["devnet-cosign"],
        &P_COSIGN,
        NetUp,
        C,
        "só dev",
    ),
    item(
        "Credenciais e bens",
        "Órgão simulado veta uma negociação",
        &["devnet-veto"],
        &P_VETO,
        NetUp,
        C,
        "só dev",
    ),
    item(
        "Credenciais e bens",
        "Equivocação do validador de teste",
        &["devnet-equivocate"],
        &P_EQUIV,
        NetUp,
        C,
        "só dev: aplica a penalidade",
    ),
    item(
        "Credenciais e bens",
        "TEKO de teste para uma carteira",
        &["fund"],
        &P_FUND,
        Any,
        C,
        "edita a gênese; vale ao (re)criar a rede",
    ),
    item(
        "Dispositivo e apps",
        "Preparar chaves de teste (1x)",
        &["setup"],
        &[],
        Any,
        C,
        "",
    ),
    item(
        "Dispositivo e apps",
        "Emulador do dispositivo",
        &["emulator"],
        &[],
        Any,
        T,
        "o PIN é digitado no terminal",
    ),
    item(
        "Dispositivo e apps",
        "Emulador com botões automáticos",
        &["demo"],
        &P_PIN,
        Any,
        T,
        "NUNCA em produção",
    ),
    item(
        "Dispositivo e apps",
        "Suite (cidadão)",
        &["suite"],
        &[],
        Any,
        T,
        "",
    ),
    item(
        "Dispositivo e apps",
        "Emissor (órgãos)",
        &["issuer"],
        &[],
        Any,
        T,
        "",
    ),
    item(
        "Dispositivo e apps",
        "Suite no navegador (backend falso)",
        &["preview"],
        &[],
        Any,
        T,
        "",
    ),
    item(
        "Dispositivo e apps",
        "Dispositivo de volta a virgem",
        &["reset"],
        &[],
        Any,
        C,
        "",
    ),
    item(
        "Dispositivo e apps",
        "Apagar dev/ (pergunta antes)",
        &["clean-dev"],
        &[],
        Any,
        T,
        "",
    ),
    item(
        "Qualidade",
        "Tudo: fmt, clippy, testes, docs",
        &["check"],
        &[],
        Any,
        C,
        "",
    ),
    item("Qualidade", "Formatar", &["fmt"], &[], Any, C, ""),
    item("Qualidade", "Clippy", &["lint"], &[], Any, C, ""),
    item("Qualidade", "Testes", &["test"], &[], Any, C, ""),
    item(
        "Qualidade",
        "Compilar o workspace",
        &["build"],
        &[],
        Any,
        C,
        "",
    ),
    item(
        "Qualidade",
        "Conferir a documentação",
        &["docs-check"],
        &[],
        Any,
        C,
        "",
    ),
    item(
        "Qualidade",
        "Conferir o release",
        &["release-check"],
        &[],
        Any,
        C,
        "",
    ),
    item(
        "Qualidade",
        "Build reproduzível",
        &["repro"],
        &[],
        Any,
        C,
        "",
    ),
    item(
        "Qualidade",
        "Verificar o firmware",
        &["verify-fw"],
        &[],
        Any,
        C,
        "",
    ),
    item(
        "Qualidade",
        "Regenerar os diagramas",
        &["diagrams"],
        &[],
        Any,
        C,
        "",
    ),
    item("Demos", "Demo da moeda", &["coin-demo"], &[], Any, C, ""),
    item("Demos", "Demo dos bens", &["assets-demo"], &[], Any, C, ""),
];

/// Makefile targets the menu deliberately does not offer.
pub const NOT_IN_MENU: &[&str] = &["help", "dev", "issuer-help", "testnet-help"];

impl Item {
    /// `Ok` if the item makes sense now, else the reason to show (the item stays visible, dimmed).
    pub fn available(&self, net: &NetState) -> Result<(), &'static str> {
        match (self.when, net.is_up()) {
            (NetNotUp, true) => Err("a devnet já está de pé (derrube antes)"),
            (NetUp, false) => Err("precisa da devnet de pé (suba primeiro)"),
            _ => Ok(()),
        }
    }
}

/// Checks the value of one parameter.
pub fn validate(p: &Param, value: &str) -> Result<(), String> {
    let v = value.trim();
    if v.is_empty() {
        return if p.required {
            Err(format!("{} é obrigatório", p.key))
        } else {
            Ok(())
        };
    }
    if !p.choices.is_empty() && !p.choices.contains(&v) {
        return Err(format!(
            "{} deve ser um de: {}",
            p.key,
            p.choices.join(", ")
        ));
    }
    if p.numeric && !v.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("{} precisa ser um número", p.key));
    }
    // the value goes to `make` as an argument (never through a shell), but keep it plain anyway
    if !v
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"_.,:/@+-".contains(&b))
    {
        return Err(format!("{} tem caracteres não permitidos", p.key));
    }
    Ok(())
}

/// The `make` command lines for an item with the given values (one per parameter, in order).
pub fn command_lines(item: &Item, values: &[String]) -> Result<Vec<Vec<String>>, String> {
    if values.len() != item.params.len() {
        return Err("quantidade de valores diferente da de parâmetros".into());
    }
    let mut vars = Vec::new();
    for (p, v) in item.params.iter().zip(values) {
        validate(p, v)?;
        let v = v.trim();
        if !v.is_empty() {
            vars.push(format!("{}={}", p.key, v));
        }
    }
    Ok(item
        .steps
        .iter()
        .map(|t| {
            let mut c = vec!["make".to_string(), (*t).to_string()];
            c.extend(vars.iter().cloned());
            c
        })
        .collect())
}

/// What the probe found about the test network.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum NetState {
    /// Still asking (first probe).
    #[default]
    Unknown,
    /// `network.txt` does not exist: `make devnet` creates it.
    NotCreated,
    /// Created, but no node answers.
    Down { nodes: usize },
    Up {
        up: usize,
        total: usize,
        votes_height: u64,
        board_size: u64,
        econ_height: Option<u64>,
        mempool: u32,
        equivocations: u32,
    },
}

impl NetState {
    pub fn is_up(&self) -> bool {
        matches!(self, NetState::Up { .. })
    }

    /// One line for the header.
    pub fn headline(&self) -> String {
        match self {
            NetState::Unknown => "verificando a devnet…".into(),
            NetState::NotCreated => "devnet: NÃO CRIADA".into(),
            NetState::Down { nodes } => format!("devnet: PARADA ({nodes} nós)"),
            NetState::Up {
                up,
                total,
                votes_height,
                board_size,
                econ_height,
                mempool,
                equivocations,
            } => format!(
                "devnet: DE PÉ {up}/{total} nós · votos: bloco {votes_height}, quadro {board_size} · economia: {} · mempool {mempool}{}",
                econ_height.map_or("—".to_string(), |h| format!("bloco {h}")),
                if *equivocations > 0 {
                    format!(" · equivocações {equivocations}")
                } else {
                    String::new()
                }
            ),
        }
    }
}

/// Asks the network (every node, in parallel, 3 s timeout each). Safe to call from a thread.
pub fn probe(dir: &Path) -> NetState {
    if !dir.join("network.txt").exists() {
        return NetState::NotCreated;
    }
    let Ok(net) = teko_node::load_network(dir) else {
        return NetState::NotCreated;
    };
    let total = net.peers.len();
    let client = teko_node::NodeClient::new(net.peers.clone());
    let (tx, rx) = mpsc::channel();
    for a in net.peers {
        let tx = tx.clone();
        let c = teko_node::NodeClient::new(vec![a]);
        std::thread::spawn(move || {
            let _ = tx.send((a, c.status(a).ok()));
        });
    }
    drop(tx);
    let mut answers: Vec<(SocketAddr, teko_node::RemoteStatus)> = Vec::new();
    while let Ok((a, s)) = rx.recv_timeout(Duration::from_secs(4)) {
        if let Some(s) = s {
            answers.push((a, s));
        }
    }
    let Some((first, st)) = answers.first().cloned() else {
        return NetState::Down { nodes: total };
    };
    let econ = client.econ_status(first).ok();
    NetState::Up {
        up: answers.len(),
        total,
        votes_height: st.height,
        board_size: st.log_size,
        econ_height: econ.map(|e| e.height),
        mempool: st.mempool,
        equivocations: answers.iter().map(|(_, s)| s.equivocations).sum(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn find(label: &str) -> &'static Item {
        CATALOG.iter().find(|i| i.label == label).unwrap()
    }

    fn up() -> NetState {
        NetState::Up {
            up: 4,
            total: 4,
            votes_height: 3,
            board_size: 9,
            econ_height: Some(5),
            mempool: 0,
            equivocations: 0,
        }
    }

    #[test]
    fn every_makefile_target_is_in_the_menu_or_deliberately_left_out() {
        let mk = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Makefile"))
            .unwrap();
        let targets: Vec<&str> = mk
            .lines()
            .filter_map(|l| {
                let (t, rest) = l.split_once(':')?;
                (rest.contains("##") && !t.contains(' ') && !t.starts_with('.')).then_some(t)
            })
            .collect();
        assert!(targets.len() > 30, "parsed {} targets", targets.len());
        for t in &targets {
            let in_menu = CATALOG.iter().any(|i| i.steps.contains(t));
            assert!(
                in_menu || NOT_IN_MENU.contains(t),
                "o alvo `{t}` do Makefile não está no menu do teko-dev"
            );
        }
        // and the menu does not name a target that does not exist
        for i in CATALOG {
            for s in i.steps {
                assert!(
                    targets.contains(s),
                    "`{s}` ({}) não existe no Makefile",
                    i.label
                );
            }
        }
    }

    #[test]
    fn a_command_line_has_the_target_then_only_the_filled_variables() {
        let it = find("Criar N blocos");
        let l = command_lines(it, &["votes".into(), "7".into()]).unwrap();
        assert_eq!(l, [["make", "devnet-blocks-sim", "KIND=votes", "COUNT=7"]]);
        // optional empty values are not passed
        let it = find("Subir a devnet");
        let l = command_lines(it, &["".into(), "".into(), "".into()]).unwrap();
        assert_eq!(l, [["make", "devnet"]]);
        let l = command_lines(it, &["tekow1abc".into(), "".into(), "5".into()]).unwrap();
        assert_eq!(l, [["make", "devnet", "FOUNDER=tekow1abc", "NODES=5"]]);
        // a restart is two steps, in order
        let l = command_lines(find("Reiniciar a devnet"), &[]).unwrap();
        assert_eq!(
            l.iter().map(|c| c[1].as_str()).collect::<Vec<_>>(),
            ["devnet-stop", "devnet"]
        );
    }

    #[test]
    fn bad_values_are_refused_before_anything_runs() {
        let blocks = find("Criar N blocos");
        assert!(
            command_lines(blocks, &["votes".into(), "x".into()]).is_err(),
            "not a number"
        );
        assert!(
            command_lines(blocks, &["other".into(), "5".into()]).is_err(),
            "not a choice"
        );
        assert!(
            command_lines(blocks, &["votes".into(), "".into()]).is_err(),
            "required"
        );
        let fund = find("TEKO de teste para uma carteira");
        assert!(
            command_lines(fund, &["tekow1abc".into(), "0,5".into()]).is_ok(),
            "decimals"
        );
        for bad in ["a b", "a;rm -rf /", "$(x)", "a\nb", "a&b", "\"x\""] {
            assert!(
                command_lines(fund, &[bad.into(), "1".into()]).is_err(),
                "{bad:?}"
            );
        }
        assert!(
            command_lines(fund, &["".into(), "1".into()]).is_err(),
            "address required"
        );
        assert!(
            command_lines(fund, &["a".into()]).is_err(),
            "wrong number of values"
        );
    }

    #[test]
    fn the_menu_follows_the_state_of_the_network() {
        let stop = find("Derrubar a devnet");
        let start = find("Subir a devnet");
        let blocks = find("Criar N blocos");
        let tally = find("Apurar (top 10)");
        for down in [
            NetState::NotCreated,
            NetState::Down { nodes: 4 },
            NetState::Unknown,
        ] {
            assert!(start.available(&down).is_ok());
            assert!(stop.available(&down).is_err());
            assert!(blocks.available(&down).is_err());
            assert!(tally.available(&down).is_ok(), "tally does not need nodes");
        }
        assert!(start.available(&up()).is_err());
        assert!(stop.available(&up()).is_ok());
        assert!(blocks.available(&up()).is_ok());
    }

    #[test]
    fn the_header_says_what_state_the_network_is_in() {
        assert!(NetState::NotCreated.headline().contains("NÃO CRIADA"));
        assert!(NetState::Down { nodes: 3 }.headline().contains("PARADA"));
        let h = up().headline();
        assert!(
            h.contains("DE PÉ 4/4") && h.contains("bloco 3") && h.contains("bloco 5"),
            "{h}"
        );
    }

    #[test]
    fn probing_a_folder_without_a_network_says_not_created() {
        let d = std::env::temp_dir().join(format!("teko-dev-probe-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        assert_eq!(probe(&d), NetState::NotCreated);
        let _ = std::fs::remove_dir_all(&d);
    }
}
