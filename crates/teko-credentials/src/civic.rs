//! The claims of the civic identity credential (ADR-0038), as the issuing organ writes them:
//! `name=value;name=value`. Pure functions (the organ's app supplies the clock and the registry id).

/// `age_over_N` claims the organ derives, so a holder can prove an age without the birth date.
pub const AGE_THRESHOLDS: [u32; 4] = [16, 18, 21, 65];

/// Person data the organ verified OUT OF BAND.
#[derive(Debug, Clone)]
pub struct CivicSubject {
    pub name: String,
    /// `YYYY-MM-DD`.
    pub birth_date: String,
    /// ISO 3166-1 alpha-2.
    pub nationality: String,
    /// Electoral domicile: UF, for example `SP`.
    pub electoral_state: String,
    /// 7-digit IBGE municipality code, for example `3550308`.
    pub electoral_municipality: String,
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if is_leap(y) => 29,
        _ => 28,
    }
}

/// `YYYY-MM-DD` to `(year, month, day)`, `None` if it is not a real date.
pub fn parse_date(s: &str) -> Option<(i64, u32, u32)> {
    let mut it = s.split('-');
    let (y, m, d) = (it.next()?, it.next()?, it.next()?);
    if it.next().is_some() || y.len() != 4 || m.len() != 2 || d.len() != 2 {
        return None;
    }
    let (y, m, d): (i64, u32, u32) = (y.parse().ok()?, m.parse().ok()?, d.parse().ok()?);
    ((1..=12).contains(&m) && d >= 1 && d <= days_in_month(y, m)).then_some((y, m, d))
}

/// Completed years between `birth` and the date of `now` (unix seconds).
pub fn age_at(birth: (i64, u32, u32), now: u64) -> i64 {
    let (ty, tm, td) = civil_from_days((now / 86_400) as i64);
    let mut age = ty - birth.0;
    if (tm, td) < (birth.1, birth.2) {
        age -= 1;
    }
    age
}

/// The claims of the credential, validated. `registry_id` is the organ's own identifier of the
/// person (deliberately NOT an RG/CPF); it stays the SAME across reissues, so the organ can tell
/// a reissue from a second identity.
pub fn civic_claims(s: &CivicSubject, registry_id: &str, now: u64) -> Result<String, String> {
    let name = s.name.trim();
    if name.is_empty()
        || name.chars().count() > 80
        || name.chars().any(|c| c.is_control() || c == ';' || c == '=')
    {
        return Err("nome deve ter de 1 a 80 caracteres (sem ; nem =)".into());
    }
    let birth =
        parse_date(s.birth_date.trim()).ok_or("data de nascimento deve ser AAAA-MM-DD válida")?;
    let age = age_at(birth, now);
    if !(0..=130).contains(&age) {
        return Err("data de nascimento fora do intervalo plausível".into());
    }
    let nat = s.nationality.trim().to_ascii_uppercase();
    if nat.len() != 2 || !nat.chars().all(|c| c.is_ascii_alphabetic()) {
        return Err("nacionalidade deve ter 2 letras (ex.: BR)".into());
    }
    let uf = s.electoral_state.trim();
    let city = s.electoral_municipality.trim();
    if uf.len() != 2
        || !uf.chars().all(|c| c.is_ascii_uppercase())
        || city.len() != 7
        || !city.chars().all(|c| c.is_ascii_digit())
    {
        return Err(
            "domicílio eleitoral: UF com 2 letras maiúsculas e município com 7 dígitos (IBGE)"
                .into(),
        );
    }
    if registry_id.is_empty()
        || registry_id.len() > 32
        || !registry_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err("identificador de registro inválido".into());
    }
    let mut parts = vec![
        format!("name={name}"),
        format!("birth_date={}", s.birth_date.trim()),
        format!("nationality={nat}"),
        format!("registry_id={registry_id}"),
        format!("electoral_state={uf}"),
        format!("electoral_municipality={city}"),
    ];
    for t in AGE_THRESHOLDS {
        parts.push(format!("age_over_{t}={}", age >= i64::from(t)));
    }
    Ok(parts.join(";"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subject() -> CivicSubject {
        CivicSubject {
            name: "Maria da Silva".into(),
            birth_date: "1990-05-17".into(),
            nationality: "br".into(),
            electoral_state: "SP".into(),
            electoral_municipality: "3550308".into(),
        }
    }

    #[test]
    fn dates_and_ages() {
        assert_eq!(parse_date("2024-02-29"), Some((2024, 2, 29)));
        assert_eq!(parse_date("2023-02-29"), None);
        assert_eq!(parse_date("1990-13-01"), None);
        assert_eq!(parse_date("90-01-01"), None);
        // 2030-03-17: birthday not yet reached => one year less
        assert_eq!(age_at((1990, 5, 17), 1_900_000_000), 39);
        assert_eq!(age_at((1990, 3, 17), 1_900_000_000), 40);
    }

    #[test]
    fn the_claims_carry_the_domicile_and_the_derived_ages() {
        let c = civic_claims(&subject(), "TK-0A1B2C3D", 1_900_000_000).unwrap();
        assert!(c.contains("nationality=BR"));
        assert!(c.contains("electoral_state=SP;electoral_municipality=3550308"));
        assert!(c.contains("registry_id=TK-0A1B2C3D"));
        assert!(c.contains("age_over_18=true") && c.contains("age_over_65=false"));
        // the device and the chain read these claims back
        let parsed = teko_crypto::civic::parse_claims(c.as_bytes()).unwrap();
        assert_eq!(parsed.len(), 10);
    }

    #[test]
    fn bad_subjects_are_refused() {
        let now = 1_900_000_000;
        for (n, b, c, uf, m) in [
            ("", "1990-01-01", "BR", "SP", "3550308"),
            ("A", "2090-01-01", "BR", "SP", "3550308"),
            ("A", "1990-01-01", "BRA", "SP", "3550308"),
            ("A\nB", "1990-01-01", "BR", "SP", "3550308"),
            ("A;B", "1990-01-01", "BR", "SP", "3550308"),
            ("A=B", "1990-01-01", "BR", "SP", "3550308"),
            ("A", "1990-01-01", "BR", "sp", "3550308"),
            ("A", "1990-01-01", "BR", "SP", "35503"),
        ] {
            let s = CivicSubject {
                name: n.into(),
                birth_date: b.into(),
                nationality: c.into(),
                electoral_state: uf.into(),
                electoral_municipality: m.into(),
            };
            assert!(
                civic_claims(&s, "TK-1", now).is_err(),
                "{n:?} {b} {c} {uf} {m}"
            );
        }
        assert!(civic_claims(&subject(), "", 1_900_000_000).is_err());
        assert!(civic_claims(&subject(), "a b", 1_900_000_000).is_err());
    }
}
