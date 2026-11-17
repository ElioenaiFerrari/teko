//! The CIVIL-REGISTRY credentials (ADR-0047): the marriage certificate (`certidao-casamento`) and the
//! divorce certificate (`certidao-divorcio`). Both are PRIVATE types: the chain keeps only a
//! commitment and the holder keeps the claims, so nobody can look up anybody's marital status. This
//! module holds the pure rules (claims, canonical form, the link between a divorce and the marriage
//! it ends), used by the organ's app that issues them and by whoever verifies what a holder shows.
//!
//! A marriage involves two people: the organ issues ONE credential to EACH spouse, with the same
//! registry number (`matricula`), each naming the other (`conjuge`). A divorce carries the
//! `matricula` of the marriage it ends; the organ then also REVOKES the marriage credentials, so the
//! marriage stops being valid and the divorce stays as the proof.

/// The credential types (created at genesis, 2-of-3 organ, private, never expire).
pub const MARRIAGE_TYPE: &str = "certidao-casamento";
pub const DIVORCE_TYPE: &str = "certidao-divorcio";

/// The property regime chosen at the marriage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Regime {
    ComunhaoParcial,
    ComunhaoUniversal,
    SeparacaoTotal,
    ParticipacaoFinal,
}

impl Regime {
    pub fn parse(s: &str) -> Option<Regime> {
        match s {
            "comunhao-parcial" => Some(Regime::ComunhaoParcial),
            "comunhao-universal" => Some(Regime::ComunhaoUniversal),
            "separacao-total" => Some(Regime::SeparacaoTotal),
            "participacao-final" => Some(Regime::ParticipacaoFinal),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Regime::ComunhaoParcial => "comunhao-parcial",
            Regime::ComunhaoUniversal => "comunhao-universal",
            Regime::SeparacaoTotal => "separacao-total",
            Regime::ParticipacaoFinal => "participacao-final",
        }
    }
}

/// How the divorce was granted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Forma {
    Judicial,
    Extrajudicial,
}

impl Forma {
    pub fn parse(s: &str) -> Option<Forma> {
        match s {
            "judicial" => Some(Forma::Judicial),
            "extrajudicial" => Some(Forma::Extrajudicial),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Forma::Judicial => "judicial",
            Forma::Extrajudicial => "extrajudicial",
        }
    }
}

/// What the registry verified about a marriage, as seen by ONE spouse (`nome`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marriage {
    /// The registry number, the same on both spouses' credentials and on the divorce.
    pub matricula: String,
    /// `YYYY-MM-DD`.
    pub data: String,
    pub regime: Regime,
    /// The holder.
    pub nome: String,
    /// The other spouse.
    pub conjuge: String,
    pub cartorio: String,
}

/// What the registry verified about a divorce, as seen by ONE former spouse (`nome`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Divorce {
    /// The registry number of the marriage it ends.
    pub matricula: String,
    pub data: String,
    pub forma: Forma,
    pub nome: String,
    pub conjuge: String,
    pub cartorio: String,
}

fn clean_text(s: &str, max: usize) -> bool {
    let t = s.trim();
    !t.is_empty()
        && t.chars().count() <= max
        && !t.chars().any(|c| c.is_control() || c == ';' || c == '=')
}

fn matricula_ok(s: &str) -> bool {
    (1..=40).contains(&s.len())
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

/// A real calendar date `YYYY-MM-DD` (leap years included).
pub fn valid_date(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let num = |r: core::ops::Range<usize>| -> Option<u32> {
        let t = s.get(r)?;
        t.chars()
            .all(|c| c.is_ascii_digit())
            .then(|| t.parse().ok())?
    };
    let (Some(y), Some(m), Some(d)) = (num(0..4), num(5..7), num(8..10)) else {
        return false;
    };
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    y >= 1 && (1..=days).contains(&d)
}

/// `YYYY-MM-DD` of a unix time (UTC).
pub fn iso_date(unix: u64) -> String {
    // civil-from-days (Howard Hinnant)
    let z = (unix / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

fn common(
    matricula: &str,
    data: &str,
    nome: &str,
    conjuge: &str,
    cartorio: &str,
) -> Result<(), String> {
    if !matricula_ok(matricula) {
        return Err("matrícula: de 1 a 40 caracteres (letras, dígitos, ponto e hífen)".into());
    }
    if !valid_date(data) {
        return Err("data: AAAA-MM-DD, uma data real".into());
    }
    if !clean_text(nome, 80) || !clean_text(conjuge, 80) {
        return Err("nomes: de 1 a 80 caracteres (sem ; nem =)".into());
    }
    if nome.trim().eq_ignore_ascii_case(conjuge.trim()) {
        return Err("os dois cônjuges não podem ter o mesmo nome".into());
    }
    if !clean_text(cartorio, 80) {
        return Err("cartório: de 1 a 80 caracteres (sem ; nem =)".into());
    }
    Ok(())
}

/// Validates a marriage and writes its claims in the canonical order.
pub fn marriage_claims(m: &Marriage) -> Result<String, String> {
    common(&m.matricula, &m.data, &m.nome, &m.conjuge, &m.cartorio)?;
    Ok(format!(
        "matricula={};data={};regime={};nome={};conjuge={};cartorio={}",
        m.matricula,
        m.data,
        m.regime.name(),
        m.nome.trim(),
        m.conjuge.trim(),
        m.cartorio.trim()
    ))
}

/// Validates a divorce and writes its claims in the canonical order.
pub fn divorce_claims(d: &Divorce) -> Result<String, String> {
    common(&d.matricula, &d.data, &d.nome, &d.conjuge, &d.cartorio)?;
    Ok(format!(
        "matricula={};data={};forma={};nome={};conjuge={};cartorio={}",
        d.matricula,
        d.data,
        d.forma.name(),
        d.nome.trim(),
        d.conjuge.trim(),
        d.cartorio.trim()
    ))
}

fn fields(claims: &[u8], keys: &[&str]) -> Option<Vec<String>> {
    let pairs = teko_crypto::civic::parse_claims(claims)?;
    if pairs.len() != keys.len() || pairs.iter().any(|(n, _)| !keys.contains(&n.as_str())) {
        return None;
    }
    keys.iter()
        .map(|k| pairs.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()))
        .collect()
}

/// Reads the claims of a marriage; `None` if they are not exactly a valid marriage.
pub fn parse_marriage(claims: &[u8]) -> Option<Marriage> {
    let f = fields(
        claims,
        &["matricula", "data", "regime", "nome", "conjuge", "cartorio"],
    )?;
    let m = Marriage {
        matricula: f[0].clone(),
        data: f[1].clone(),
        regime: Regime::parse(&f[2])?,
        nome: f[3].clone(),
        conjuge: f[4].clone(),
        cartorio: f[5].clone(),
    };
    // canonical: exactly what `marriage_claims` would write
    (marriage_claims(&m).ok()?.as_bytes() == claims).then_some(m)
}

/// Reads the claims of a divorce; `None` if they are not exactly a valid divorce.
pub fn parse_divorce(claims: &[u8]) -> Option<Divorce> {
    let f = fields(
        claims,
        &["matricula", "data", "forma", "nome", "conjuge", "cartorio"],
    )?;
    let d = Divorce {
        matricula: f[0].clone(),
        data: f[1].clone(),
        forma: Forma::parse(&f[2])?,
        nome: f[3].clone(),
        conjuge: f[4].clone(),
        cartorio: f[5].clone(),
    };
    (divorce_claims(&d).ok()?.as_bytes() == claims).then_some(d)
}

/// Does this divorce END this marriage? Same registry number, the same two people, and the divorce
/// is AFTER the wedding.
pub fn divorce_ends(m: &Marriage, d: &Divorce) -> Result<(), String> {
    if m.matricula != d.matricula {
        return Err("o divórcio é de outra matrícula".into());
    }
    if m.nome != d.nome || m.conjuge != d.conjuge {
        return Err("o divórcio não é do mesmo titular e cônjuge do casamento".into());
    }
    if d.data <= m.data {
        return Err("o divórcio tem de ser depois do casamento".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marriage(nome: &str, conjuge: &str) -> Marriage {
        Marriage {
            matricula: "123456.01.55.2020.2.00045".into(),
            data: "2020-02-29".into(),
            regime: Regime::ComunhaoParcial,
            nome: nome.into(),
            conjuge: conjuge.into(),
            cartorio: "1º Ofício de Registro Civil de São Paulo".into(),
        }
    }

    fn divorce(nome: &str, conjuge: &str, data: &str) -> Divorce {
        let m = marriage(nome, conjuge);
        Divorce {
            matricula: m.matricula,
            data: data.into(),
            forma: Forma::Extrajudicial,
            nome: nome.into(),
            conjuge: conjuge.into(),
            cartorio: m.cartorio,
        }
    }

    #[test]
    fn a_marriage_has_canonical_claims_that_round_trip() {
        let m = marriage("Ana Souza", "Beto Lima");
        let c = marriage_claims(&m).unwrap();
        assert!(c.starts_with("matricula=123456.01.55.2020.2.00045;data=2020-02-29;regime=comunhao-parcial;nome=Ana Souza;conjuge=Beto Lima;"));
        assert_eq!(parse_marriage(c.as_bytes()), Some(m));
        // a stray claim, a missing one, another order or a bad regime are not a marriage
        for bad in [
            format!("{c};extra=1"),
            c.replace("nome=Ana Souza;", ""),
            c.replace("comunhao-parcial", "sei-la"),
            c.replace("matricula=1", "matricula=1;nome=X;matricula=1"),
        ] {
            assert!(parse_marriage(bad.as_bytes()).is_none(), "{bad}");
        }
        let (a, b) = (
            "matricula=1;data=2020-01-01;",
            "regime=separacao-total;nome=A;conjuge=B;cartorio=C",
        );
        assert!(parse_marriage(format!("{a}{b}").as_bytes()).is_some());
        assert!(
            parse_marriage(
                format!("regime=separacao-total;{a}nome=A;conjuge=B;cartorio=C").as_bytes()
            )
            .is_none(),
            "not canonical"
        );
    }

    #[test]
    fn the_claims_are_validated() {
        let ok = marriage("Ana", "Beto");
        for edit in [
            (|m: &mut Marriage| m.matricula = String::new()) as fn(&mut Marriage),
            |m| m.matricula = "a b".into(),
            |m| m.matricula = "x".repeat(41),
            |m| m.data = "2021-02-29".into(), // not a leap year
            |m| m.data = "2020-13-01".into(),
            |m| m.data = "20-1-1".into(),
            |m| m.nome = "A;B".into(),
            |m| m.conjuge = "A=B".into(),
            |m| m.conjuge = "ana".into(), // the same person
            |m| m.cartorio = String::new(),
        ] {
            let mut m = ok.clone();
            edit(&mut m);
            assert!(marriage_claims(&m).is_err(), "{m:?}");
        }
        assert!(valid_date("2020-02-29") && valid_date("2000-02-29"));
        assert!(!valid_date("1900-02-29") && !valid_date("2021-04-31"));
        assert_eq!(iso_date(0), "1970-01-01");
        assert_eq!(iso_date(1_582_934_400), "2020-02-29");
        assert!(valid_date(&iso_date(1_900_000_000)));
    }

    #[test]
    fn a_divorce_ends_exactly_the_marriage_it_names() {
        let m = marriage("Ana", "Beto");
        let d = divorce("Ana", "Beto", "2024-05-10");
        let c = divorce_claims(&d).unwrap();
        assert_eq!(parse_divorce(c.as_bytes()), Some(d.clone()));
        assert!(divorce_ends(&m, &d).is_ok());
        // the other spouse's view of the SAME marriage is ended by the other spouse's divorce
        assert!(
            divorce_ends(
                &marriage("Beto", "Ana"),
                &divorce("Beto", "Ana", "2024-05-10")
            )
            .is_ok()
        );
        // another registry number, another couple, or a date that is not after the wedding
        let mut other = d.clone();
        other.matricula = "999".into();
        assert!(divorce_ends(&m, &other).is_err());
        assert!(divorce_ends(&m, &divorce("Ana", "Caio", "2024-05-10")).is_err());
        assert!(
            divorce_ends(&m, &divorce("Beto", "Ana", "2024-05-10")).is_err(),
            "not this spouse's"
        );
        assert!(divorce_ends(&m, &divorce("Ana", "Beto", "2020-02-29")).is_err());
        assert!(divorce_ends(&m, &divorce("Ana", "Beto", "2019-01-01")).is_err());
        // a marriage is not a divorce and vice versa
        assert!(parse_divorce(marriage_claims(&m).unwrap().as_bytes()).is_none());
        assert!(parse_marriage(c.as_bytes()).is_none());
    }
}
