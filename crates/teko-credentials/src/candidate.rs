//! The candidacy credential (ADR-0040): a PUBLIC credential of type `candidato` whose claims are
//! `cargo;uf;municipio;[partido;legenda;]numero;nome;eleicao` (`partido` and `legenda` only in the
//! proportional offices, ADR-0046). An election's candidates are the ACTIVE candidacies
//! that match a contest; this module holds the pure rules (claims, matching, selection), used by
//! the organ's app that issues them, by the builder of the election manifest and by auditors.

/// The offices that are elected automatically: the single-seat ones (ADR-0040), the senator (1 or 2
/// seats, ADR-0044) and the proportional ones, deputies and councillors (ADR-0046).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Cargo {
    #[default]
    Presidente,
    Governador,
    Prefeito,
    Senador,
    DeputadoFederal,
    DeputadoEstadual,
    Vereador,
}

/// Where an office is contested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    National,
    State,
    Municipality,
}

impl Cargo {
    pub fn parse(s: &str) -> Option<Cargo> {
        match s {
            "presidente" => Some(Cargo::Presidente),
            "governador" => Some(Cargo::Governador),
            "prefeito" => Some(Cargo::Prefeito),
            "senador" => Some(Cargo::Senador),
            "deputado-federal" => Some(Cargo::DeputadoFederal),
            "deputado-estadual" => Some(Cargo::DeputadoEstadual),
            "vereador" => Some(Cargo::Vereador),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Cargo::Presidente => "presidente",
            Cargo::Governador => "governador",
            Cargo::Prefeito => "prefeito",
            Cargo::Senador => "senador",
            Cargo::DeputadoFederal => "deputado-federal",
            Cargo::DeputadoEstadual => "deputado-estadual",
            Cargo::Vereador => "vereador",
        }
    }

    pub fn level(self) -> Level {
        match self {
            Cargo::Presidente => Level::National,
            Cargo::Governador
            | Cargo::Senador
            | Cargo::DeputadoFederal
            | Cargo::DeputadoEstadual => Level::State,
            Cargo::Prefeito | Cargo::Vereador => Level::Municipality,
        }
    }

    /// Deputies and councillors: voted by party list (ADR-0046).
    pub fn is_proportional(self) -> bool {
        matches!(
            self,
            Cargo::DeputadoFederal | Cargo::DeputadoEstadual | Cargo::Vereador
        )
    }

    /// Digits of a candidate number in a proportional office (2 of the party + 2 or 3 of the
    /// candidate, as in Brazil).
    fn number_len(self) -> Option<usize> {
        match self {
            Cargo::DeputadoFederal => Some(4),
            Cargo::DeputadoEstadual | Cargo::Vereador => Some(5),
            _ => None,
        }
    }
}

/// What the organ verified about a candidate.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Candidacy {
    pub cargo: Cargo,
    /// 2 letters; empty for the President.
    pub uf: String,
    /// 7-digit IBGE code; only for a mayor.
    pub municipio: String,
    /// Two digits of the party; only in the proportional offices (ADR-0046).
    pub partido: String,
    /// The party's name or acronym (all candidates of one party agree on it).
    pub legenda: String,
    pub numero: String,
    pub nome: String,
    pub eleicao: String,
}

impl Candidacy {
    /// The jurisdiction code of the contest this candidacy is for: empty (national), the UF or the
    /// IBGE code.
    pub fn scope_code(&self) -> &str {
        match self.cargo.level() {
            Level::National => "",
            Level::State => &self.uf,
            Level::Municipality => &self.municipio,
        }
    }
}

fn clean_text(s: &str, max: usize) -> bool {
    let t = s.trim();
    !t.is_empty()
        && t.chars().count() <= max
        && !t.chars().any(|c| c.is_control() || c == ';' || c == '=')
}

/// Validates a candidacy and writes its claims in the canonical order.
pub fn candidate_claims(c: &Candidacy) -> Result<String, String> {
    if !clean_text(&c.nome, 80) {
        return Err("nome deve ter de 1 a 80 caracteres (sem ; nem =)".into());
    }
    if c.numero.is_empty() || c.numero.len() > 8 || !c.numero.chars().all(|d| d.is_ascii_digit()) {
        return Err("número: de 1 a 8 dígitos".into());
    }
    if !c.eleicao.is_empty()
        && !(c.eleicao.len() <= 64
            && c.eleicao
                .chars()
                .all(|x| x.is_ascii_lowercase() || x.is_ascii_digit() || x == '-'))
    {
        return Err("eleição: letras minúsculas, dígitos e hífen".into());
    }
    if c.eleicao.is_empty() {
        return Err("informe a eleição".into());
    }
    let uf_ok = c.uf.len() == 2 && c.uf.chars().all(|x| x.is_ascii_uppercase());
    let mun_ok = c.municipio.len() == 7 && c.municipio.chars().all(|x| x.is_ascii_digit());
    match c.cargo.level() {
        Level::National => {
            if !c.uf.is_empty() || !c.municipio.is_empty() {
                return Err("presidente é nacional: sem UF nem município".into());
            }
        }
        Level::State => {
            if !uf_ok || !c.municipio.is_empty() {
                return Err("governador: UF com 2 letras maiúsculas, sem município".into());
            }
        }
        Level::Municipality => {
            if !mun_ok || !(c.uf.is_empty() || uf_ok) {
                return Err("prefeito: município com 7 dígitos (IBGE)".into());
            }
        }
    }
    if c.cargo.is_proportional() {
        let party_ok = c.partido.len() == 2 && c.partido.chars().all(|d| d.is_ascii_digit());
        if !party_ok || !clean_text(&c.legenda, 40) {
            return Err("partido: 2 dígitos e legenda de 1 a 40 caracteres (sem ; nem =)".into());
        }
        if Some(c.numero.len()) != c.cargo.number_len() || !c.numero.starts_with(&c.partido) {
            return Err(format!(
                "número de {}: {} dígitos, começando pelo partido",
                c.cargo.name(),
                c.cargo.number_len().unwrap_or(0)
            ));
        }
    } else if !c.partido.is_empty() || !c.legenda.is_empty() {
        return Err("partido só nos cargos proporcionais".into());
    }
    let mut parts = vec![format!("cargo={}", c.cargo.name())];
    if !c.uf.is_empty() {
        parts.push(format!("uf={}", c.uf));
    }
    if !c.municipio.is_empty() {
        parts.push(format!("municipio={}", c.municipio));
    }
    if c.cargo.is_proportional() {
        parts.push(format!("partido={}", c.partido));
        parts.push(format!("legenda={}", c.legenda.trim()));
    }
    parts.push(format!("numero={}", c.numero));
    parts.push(format!("nome={}", c.nome.trim()));
    parts.push(format!("eleicao={}", c.eleicao));
    Ok(parts.join(";"))
}

/// Reads the claims of a candidacy; `None` if they are not exactly a valid candidacy.
pub fn parse_candidate(claims: &[u8]) -> Option<Candidacy> {
    let pairs = teko_crypto::civic::parse_claims(claims)?;
    let get = |k: &str| {
        pairs
            .iter()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };
    // no claim outside the known ones (a stray claim would be ignored by a reader and not by another)
    if pairs.iter().any(|(n, _)| {
        ![
            "cargo",
            "uf",
            "municipio",
            "partido",
            "legenda",
            "numero",
            "nome",
            "eleicao",
        ]
        .contains(&n.as_str())
    }) {
        return None;
    }
    let c = Candidacy {
        cargo: Cargo::parse(&get("cargo"))?,
        uf: get("uf"),
        municipio: get("municipio"),
        partido: get("partido"),
        legenda: get("legenda"),
        numero: get("numero"),
        nome: get("nome"),
        eleicao: get("eleicao"),
    };
    // canonical: it must be exactly what `candidate_claims` would write
    (candidate_claims(&c).ok()?.as_bytes() == claims).then_some(c)
}

/// One credential of type `candidato` as read from the chain.
#[derive(Debug, Clone)]
pub struct CandidateEntry {
    pub id: Vec<u8>,
    pub subject: Vec<u8>,
    pub claims: Vec<u8>,
    pub valid_until: u64,
    /// ACTIVE at the reference time (past the contest window, not vetoed, revoked or expired).
    pub active: bool,
}

/// A candidate chosen for a contest, in ballot order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selected {
    pub id: Vec<u8>,
    pub subject: Vec<u8>,
    pub numero: String,
    pub nome: String,
    /// Party number and name, in the proportional offices (empty otherwise).
    pub partido: String,
    pub legenda: String,
}

/// The candidates of ONE contest: the ACTIVE candidacies for this election, office and
/// jurisdiction, whose validity outlasts the voting (plus `margin` seconds for tally and audit).
/// The contest is REFUSED, instead of silently picking one, when two candidates share a number or
/// a holder, or there are fewer than two. Ordered by number.
pub fn select_candidates(
    entries: &[CandidateEntry],
    eleicao: &str,
    cargo: Cargo,
    scope_code: &str,
    voting_closes: u64,
    margin: u64,
) -> Result<Vec<Selected>, String> {
    let mut out: Vec<Selected> = Vec::new();
    for e in entries {
        let Some(c) = parse_candidate(&e.claims) else {
            continue;
        };
        if !e.active
            || c.eleicao != eleicao
            || c.cargo != cargo
            || c.scope_code() != scope_code
            || (e.valid_until != 0 && e.valid_until <= voting_closes.saturating_add(margin))
        {
            continue;
        }
        if out.iter().any(|o| o.numero == c.numero) {
            return Err(format!("número {} repetido entre os candidatos", c.numero));
        }
        if out.iter().any(|o| o.subject == e.subject) {
            return Err(format!("o mesmo titular concorre duas vezes ({})", c.nome));
        }
        if out
            .iter()
            .any(|o| o.partido == c.partido && o.legenda != c.legenda)
        {
            return Err(format!(
                "o partido {} aparece com dois nomes ({})",
                c.partido, c.legenda
            ));
        }
        out.push(Selected {
            id: e.id.clone(),
            subject: e.subject.clone(),
            numero: c.numero,
            nome: c.nome,
            partido: c.partido,
            legenda: c.legenda,
        });
    }
    if out.len() < 2 {
        return Err(format!(
            "são precisos ao menos 2 candidatos ativos ({} encontrado(s))",
            out.len()
        ));
    }
    out.sort_by(|a, b| {
        a.numero
            .len()
            .cmp(&b.numero.len())
            .then_with(|| a.numero.cmp(&b.numero))
    });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(cargo: Cargo, uf: &str, mun: &str, numero: &str, nome: &str) -> Candidacy {
        Candidacy {
            cargo,
            uf: uf.into(),
            municipio: mun.into(),
            partido: String::new(),
            legenda: String::new(),
            numero: numero.into(),
            nome: nome.into(),
            eleicao: "geral-2026".into(),
        }
    }

    fn entry(id: u8, subject: u8, c: &Candidacy, valid_until: u64, active: bool) -> CandidateEntry {
        CandidateEntry {
            id: vec![id; 32],
            subject: vec![subject; 33],
            claims: candidate_claims(c).unwrap().into_bytes(),
            valid_until,
            active,
        }
    }

    #[test]
    fn claims_round_trip_in_the_canonical_form_and_reject_anything_else() {
        let c = cand(Cargo::Governador, "SP", "", "13", "Ana Souza");
        let text = candidate_claims(&c).unwrap();
        assert_eq!(
            text,
            "cargo=governador;uf=SP;numero=13;nome=Ana Souza;eleicao=geral-2026"
        );
        assert_eq!(parse_candidate(text.as_bytes()), Some(c));
        // not canonical (order), unknown claim, missing claim, bad cargo: no candidacy
        assert!(
            parse_candidate(b"uf=SP;cargo=governador;numero=13;nome=Ana;eleicao=geral-2026")
                .is_none()
        );
        assert!(parse_candidate(format!("{text};extra=1").as_bytes()).is_none());
        assert!(parse_candidate(b"cargo=governador;uf=SP;numero=13;eleicao=geral-2026").is_none());
        assert!(parse_candidate(b"cargo=rei;numero=1;nome=A;eleicao=e").is_none());
    }

    #[test]
    fn the_jurisdiction_follows_the_office() {
        assert!(candidate_claims(&cand(Cargo::Presidente, "", "", "10", "A")).is_ok());
        assert!(candidate_claims(&cand(Cargo::Presidente, "SP", "", "10", "A")).is_err());
        assert!(candidate_claims(&cand(Cargo::Governador, "", "", "10", "A")).is_err());
        assert!(candidate_claims(&cand(Cargo::Governador, "sp", "", "10", "A")).is_err());
        assert!(candidate_claims(&cand(Cargo::Prefeito, "", "3550308", "10", "A")).is_ok());
        assert!(candidate_claims(&cand(Cargo::Prefeito, "", "35503", "10", "A")).is_err());
        assert!(candidate_claims(&cand(Cargo::Prefeito, "SP", "3550308", "10", "A")).is_ok());
        for bad in ["", "x1", "123456789"] {
            assert!(
                candidate_claims(&cand(Cargo::Presidente, "", "", bad, "A")).is_err(),
                "{bad}"
            );
        }
        for bad in ["", "A;B", "A=B", "A\nB"] {
            assert!(
                candidate_claims(&cand(Cargo::Presidente, "", "", "10", bad)).is_err(),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn a_contest_takes_the_active_matching_candidates_ordered_by_number() {
        let a = cand(Cargo::Governador, "SP", "", "13", "Ana");
        let b = cand(Cargo::Governador, "SP", "", "9", "Beto");
        let rj = cand(Cargo::Governador, "RJ", "", "20", "Rio");
        let pres = cand(Cargo::Presidente, "", "", "30", "Pres");
        let entries = vec![
            entry(1, 1, &a, 9_000, true),
            entry(2, 2, &b, 9_000, true),
            entry(3, 3, &rj, 9_000, true),
            entry(4, 4, &pres, 9_000, true),
        ];
        let got =
            select_candidates(&entries, "geral-2026", Cargo::Governador, "SP", 5_000, 100).unwrap();
        assert_eq!(
            got.iter().map(|s| s.numero.as_str()).collect::<Vec<_>>(),
            ["9", "13"]
        );
        assert_eq!(got[0].id, vec![2; 32]);
    }

    #[test]
    fn inactive_expiring_or_foreign_candidacies_do_not_enter() {
        let a = cand(Cargo::Governador, "SP", "", "13", "Ana");
        let b = cand(Cargo::Governador, "SP", "", "9", "Beto");
        let c = cand(Cargo::Governador, "SP", "", "7", "Cris");
        let mut other_election = cand(Cargo::Governador, "SP", "", "8", "Dani");
        other_election.eleicao = "outra".into();
        let entries = vec![
            entry(1, 1, &a, 9_000, true),
            entry(2, 2, &b, 5_050, true),  // ends before voting + margin
            entry(3, 3, &c, 9_000, false), // revoked, vetoed or still pending
            entry(4, 4, &other_election, 9_000, true),
        ];
        let e = select_candidates(&entries, "geral-2026", Cargo::Governador, "SP", 5_000, 100)
            .unwrap_err();
        assert!(e.contains("ao menos 2"), "{e}");
        // a lifetime credential (valid_until 0) is never "too short"
        let mut entries = entries;
        entries[1].valid_until = 0;
        assert_eq!(
            select_candidates(&entries, "geral-2026", Cargo::Governador, "SP", 5_000, 100)
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn a_repeated_number_or_holder_refuses_the_contest() {
        let a = cand(Cargo::Presidente, "", "", "13", "Ana");
        let b = cand(Cargo::Presidente, "", "", "13", "Beto");
        let e = select_candidates(
            &[entry(1, 1, &a, 0, true), entry(2, 2, &b, 0, true)],
            "geral-2026",
            Cargo::Presidente,
            "",
            5_000,
            0,
        )
        .unwrap_err();
        assert!(e.contains("repetido"), "{e}");
        let c = cand(Cargo::Presidente, "", "", "14", "Ana de novo");
        let e = select_candidates(
            &[entry(1, 1, &a, 0, true), entry(3, 1, &c, 0, true)],
            "geral-2026",
            Cargo::Presidente,
            "",
            5_000,
            0,
        )
        .unwrap_err();
        assert!(e.contains("duas vezes"), "{e}");
    }

    fn deputy(cargo: Cargo, partido: &str, legenda: &str, numero: &str) -> Candidacy {
        Candidacy {
            cargo,
            uf: if cargo == Cargo::Vereador {
                String::new()
            } else {
                "SP".into()
            },
            municipio: if cargo == Cargo::Vereador {
                "3550308".into()
            } else {
                String::new()
            },
            partido: partido.into(),
            legenda: legenda.into(),
            numero: numero.into(),
            nome: "Ana".into(),
            eleicao: "geral-2026".into(),
        }
    }

    #[test]
    fn a_proportional_candidacy_carries_its_party_and_its_number_starts_with_it() {
        let c = deputy(Cargo::DeputadoEstadual, "13", "Verde", "13001");
        let claims = candidate_claims(&c).unwrap();
        assert_eq!(
            claims,
            "cargo=deputado-estadual;uf=SP;partido=13;legenda=Verde;numero=13001;nome=Ana;eleicao=geral-2026"
        );
        assert_eq!(parse_candidate(claims.as_bytes()), Some(c));
        // federal: 4 digits; councillor: municipality and 5 digits
        assert!(candidate_claims(&deputy(Cargo::DeputadoFederal, "13", "Verde", "1301")).is_ok());
        assert!(candidate_claims(&deputy(Cargo::Vereador, "13", "Verde", "13001")).is_ok());
        for bad in [
            deputy(Cargo::DeputadoEstadual, "13", "Verde", "13"),
            deputy(Cargo::DeputadoEstadual, "13", "Verde", "1301"),
            deputy(Cargo::DeputadoEstadual, "13", "Verde", "45001"),
            deputy(Cargo::DeputadoEstadual, "1", "Verde", "13001"),
            deputy(Cargo::DeputadoEstadual, "13", "", "13001"),
            deputy(Cargo::DeputadoEstadual, "13", "Ve;rde", "13001"),
        ] {
            assert!(candidate_claims(&bad).is_err(), "{bad:?}");
        }
        // a party only in the proportional offices
        let mut g = cand(Cargo::Governador, "SP", "", "13", "Ana");
        g.partido = "13".into();
        assert!(candidate_claims(&g).is_err());
        // the chain keeps the old claims unchanged: a governor has no party claims
        assert!(
            !candidate_claims(&cand(Cargo::Governador, "SP", "", "13", "Ana"))
                .unwrap()
                .contains("partido")
        );
    }

    #[test]
    fn the_same_party_cannot_appear_with_two_names_in_one_contest() {
        let a = deputy(Cargo::DeputadoEstadual, "13", "Verde", "13001");
        let b = deputy(Cargo::DeputadoEstadual, "13", "Verdes", "13002");
        let es = vec![entry(1, 1, &a, 0, true), entry(2, 2, &b, 0, true)];
        let e = select_candidates(&es, "geral-2026", Cargo::DeputadoEstadual, "SP", 100, 0)
            .unwrap_err();
        assert!(e.contains("dois nomes"), "{e}");
        let b = deputy(Cargo::DeputadoEstadual, "13", "Verde", "13002");
        let es = vec![entry(1, 1, &a, 0, true), entry(2, 2, &b, 0, true)];
        let sel =
            select_candidates(&es, "geral-2026", Cargo::DeputadoEstadual, "SP", 100, 0).unwrap();
        assert_eq!(
            (sel[0].partido.as_str(), sel[0].legenda.as_str()),
            ("13", "Verde")
        );
    }
}
