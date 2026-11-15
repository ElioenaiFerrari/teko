//! Metadata records of assets (ADR-0032). The chain stores only a 32-byte HASH of the
//! record; the record itself (plate, chassis, address...) lives off-chain, so the
//! owner's data is not public. The device, the Suite, the ledger and the explorer share
//! THIS code, so "what the device shows is what the hash covers".
//!
//! A record is a **canonical text**: one `field=value` line per schema field, in the
//! schema's fixed order, UTF-8, normalized (upper-case plate and chassis, digits-only
//! RENAVAM), at most [`MAX_RECORD`] bytes. Any other encoding is refused, so two
//! different texts never hash to the same asset.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::keys::tagged_hash;
use crate::tags;

/// Largest canonical record, in bytes.
pub const MAX_RECORD: usize = 512;

/// Why a record was refused (Display is pt-BR, for the Suite and the device).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetaError {
    /// This class has no metadata schema.
    UnknownClass,
    Missing(&'static str),
    /// A field that the schema does not have.
    UnknownField(String),
    Invalid {
        field: &'static str,
        why: &'static str,
    },
    TooLong,
    /// Not the canonical encoding.
    Malformed,
}

impl core::fmt::Display for MetaError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            MetaError::UnknownClass => f.write_str("classe sem esquema de metadados"),
            MetaError::Missing(k) => write!(f, "falta o campo {k}"),
            MetaError::UnknownField(k) => write!(f, "campo desconhecido: {k}"),
            MetaError::Invalid { field, why } => write!(f, "{field} inválido: {why}"),
            MetaError::TooLong => f.write_str("registro grande demais"),
            MetaError::Malformed => f.write_str("registro fora do formato canônico"),
        }
    }
}

/// One field of a schema, with what a form needs to show it.
#[derive(Debug, Clone, Copy)]
pub struct FieldSpec {
    pub key: &'static str,
    pub label: &'static str,
    pub help: &'static str,
    check: fn(&str) -> Result<String, &'static str>,
}

fn text(s: &str, max: usize) -> Result<String, &'static str> {
    let t = s.trim();
    if t.is_empty() {
        return Err("não pode ficar vazio");
    }
    if t.chars().count() > max {
        return Err("texto longo demais");
    }
    if t.chars().any(char::is_control) {
        return Err("caracteres inválidos");
    }
    Ok(t.to_string())
}

fn marca(s: &str) -> Result<String, &'static str> {
    text(s, 30)
}
fn modelo(s: &str) -> Result<String, &'static str> {
    text(s, 40)
}
fn cor(s: &str) -> Result<String, &'static str> {
    text(s, 20)
}
fn cartorio(s: &str) -> Result<String, &'static str> {
    text(s, 60)
}
fn endereco(s: &str) -> Result<String, &'static str> {
    text(s, 120)
}

fn chassi(s: &str) -> Result<String, &'static str> {
    let v = s.trim().to_ascii_uppercase();
    let ok = v.len() == 17
        && v.bytes().all(|b| {
            b.is_ascii_digit() || (b.is_ascii_uppercase() && !matches!(b, b'I' | b'O' | b'Q'))
        });
    if ok {
        Ok(v)
    } else {
        Err("17 caracteres, letras e números, sem I, O ou Q")
    }
}

fn placa(s: &str) -> Result<String, &'static str> {
    let v: String = s
        .trim()
        .to_ascii_uppercase()
        .chars()
        .filter(|c| *c != '-')
        .collect();
    let b = v.as_bytes();
    let letters = |r: &[u8]| r.iter().all(u8::is_ascii_uppercase);
    let digits = |r: &[u8]| r.iter().all(u8::is_ascii_digit);
    let old = b.len() == 7 && letters(&b[..3]) && digits(&b[3..]);
    let mercosul = b.len() == 7
        && letters(&b[..3])
        && b[3].is_ascii_digit()
        && b[4].is_ascii_uppercase()
        && digits(&b[5..]);
    if old || mercosul {
        Ok(v)
    } else {
        Err("use AAA0A00 (Mercosul) ou AAA0000")
    }
}

/// RENAVAM check digit: the first 10 digits times 3,2,9,8,7,6,5,4,3,2, sum times 10,
/// modulo 11 (10 becomes 0).
fn renavam_dv(base10: &[u8]) -> u8 {
    const W: [u32; 10] = [3, 2, 9, 8, 7, 6, 5, 4, 3, 2];
    let sum: u32 = base10
        .iter()
        .zip(W)
        .map(|(d, w)| u32::from(d - b'0') * w)
        .sum();
    let r = (sum * 10) % 11;
    if r == 10 { 0 } else { r as u8 }
}

fn renavam(s: &str) -> Result<String, &'static str> {
    let v = s.trim();
    let b = v.as_bytes();
    if b.len() != 11 || !b.iter().all(u8::is_ascii_digit) {
        return Err("11 dígitos");
    }
    if renavam_dv(&b[..10]) != b[10] - b'0' {
        return Err("dígito verificador não confere");
    }
    Ok(v.to_string())
}

fn year(s: &str) -> Result<String, &'static str> {
    let v = s.trim();
    let n: u32 = v.parse().map_err(|_| "use 4 dígitos, ex.: 2021")?;
    if v.len() == 4 && (1950..=2100).contains(&n) {
        Ok(v.to_string())
    } else {
        Err("entre 1950 e 2100")
    }
}

fn matricula(s: &str) -> Result<String, &'static str> {
    let v = s.trim();
    let ok = !v.is_empty()
        && v.len() <= 64
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'/'));
    if ok {
        Ok(v.to_string())
    } else {
        Err("letras, números e - _ . / (até 64)")
    }
}

/// Area in m² with up to 2 decimals after a comma: `72`, `72,5`, `72,50`.
fn area(s: &str) -> Result<String, &'static str> {
    let v = s.trim();
    let (int, frac) = v.split_once(',').unwrap_or((v, ""));
    let ok = !int.is_empty()
        && int.len() <= 8
        && int.bytes().all(|b| b.is_ascii_digit())
        && frac.len() <= 2
        && frac.bytes().all(|b| b.is_ascii_digit())
        && (v.contains(',') == !frac.is_empty());
    let nonzero = int.bytes().chain(frac.bytes()).any(|b| b != b'0');
    if ok && nonzero {
        Ok(v.to_string())
    } else {
        Err("use números, com até 2 casas depois da vírgula (ex.: 72,5)")
    }
}

const VEICULO: &[FieldSpec] = &[
    FieldSpec {
        key: "chassi",
        label: "Chassi",
        help: "17 caracteres, sem I, O ou Q (é o identificador do veículo)",
        check: chassi,
    },
    FieldSpec {
        key: "placa",
        label: "Placa",
        help: "AAA0A00 (Mercosul) ou AAA0000",
        check: placa,
    },
    FieldSpec {
        key: "renavam",
        label: "RENAVAM",
        help: "11 dígitos, com dígito verificador",
        check: renavam,
    },
    FieldSpec {
        key: "marca",
        label: "Marca",
        help: "ex.: Volkswagen",
        check: marca,
    },
    FieldSpec {
        key: "modelo",
        label: "Modelo",
        help: "ex.: Gol 1.0",
        check: modelo,
    },
    FieldSpec {
        key: "ano_fabricacao",
        label: "Ano de fabricação",
        help: "4 dígitos",
        check: year,
    },
    FieldSpec {
        key: "ano_modelo",
        label: "Ano do modelo",
        help: "o mesmo ano ou o seguinte",
        check: year,
    },
    FieldSpec {
        key: "cor",
        label: "Cor",
        help: "ex.: Prata",
        check: cor,
    },
];

const IMOVEL: &[FieldSpec] = &[
    FieldSpec {
        key: "matricula",
        label: "Matrícula",
        help: "número da matrícula (é o identificador do imóvel)",
        check: matricula,
    },
    FieldSpec {
        key: "cartorio",
        label: "Cartório",
        help: "ex.: 1º Registro de Imóveis de São Paulo",
        check: cartorio,
    },
    FieldSpec {
        key: "endereco",
        label: "Endereço",
        help: "rua, número, cidade/UF",
        check: endereco,
    },
    FieldSpec {
        key: "area_m2",
        label: "Área (m²)",
        help: "ex.: 72,5",
        check: area,
    },
];

/// The fields of a class, in canonical order. `None` = the class has no schema.
pub fn schema(class: &str) -> Option<&'static [FieldSpec]> {
    match class {
        "veiculo" => Some(VEICULO),
        "imovel" => Some(IMOVEL),
        _ => None,
    }
}

/// The field that is also the asset id on the chain.
pub fn id_field(class: &str) -> Option<&'static str> {
    match class {
        "veiculo" => Some("chassi"),
        "imovel" => Some("matricula"),
        _ => None,
    }
}

fn cross_check(class: &str, v: &[(&'static str, String)]) -> Result<(), MetaError> {
    if class == "veiculo" {
        let get = |k: &str| {
            v.iter()
                .find(|(f, _)| *f == k)
                .and_then(|(_, x)| x.parse::<u32>().ok())
        };
        if let (Some(f), Some(m)) = (get("ano_fabricacao"), get("ano_modelo"))
            && !(f..=f + 1).contains(&m)
        {
            return Err(MetaError::Invalid {
                field: "ano_modelo",
                why: "deve ser o ano de fabricação ou o seguinte",
            });
        }
    }
    Ok(())
}

/// Validates `fields` against the class schema and returns the canonical record.
pub fn canonical(class: &str, fields: &[(&str, &str)]) -> Result<Vec<u8>, MetaError> {
    let spec = schema(class).ok_or(MetaError::UnknownClass)?;
    if let Some((k, _)) = fields
        .iter()
        .find(|(k, _)| !spec.iter().any(|s| s.key == *k))
    {
        return Err(MetaError::UnknownField((*k).to_string()));
    }
    let mut values = Vec::with_capacity(spec.len());
    for s in spec {
        let raw = fields
            .iter()
            .find(|(k, _)| *k == s.key)
            .map(|(_, v)| *v)
            .ok_or(MetaError::Missing(s.key))?;
        let v = (s.check)(raw).map_err(|why| MetaError::Invalid { field: s.key, why })?;
        values.push((s.key, v));
    }
    cross_check(class, &values)?;
    let mut out = String::new();
    for (k, v) in &values {
        out.push_str(&format!("{k}={v}\n"));
    }
    if out.len() > MAX_RECORD {
        return Err(MetaError::TooLong);
    }
    Ok(out.into_bytes())
}

/// Reads a canonical record back into `(field, value)` pairs. Strict: anything that is
/// not exactly what [`canonical`] would produce is refused.
pub fn parse(class: &str, record: &[u8]) -> Result<Vec<(String, String)>, MetaError> {
    if record.len() > MAX_RECORD {
        return Err(MetaError::TooLong);
    }
    let text = core::str::from_utf8(record).map_err(|_| MetaError::Malformed)?;
    let pairs: Vec<(&str, &str)> = text
        .lines()
        .map(|l| l.split_once('=').ok_or(MetaError::Malformed))
        .collect::<Result<_, _>>()?;
    let again = canonical(class, &pairs)?;
    if again != record {
        return Err(MetaError::Malformed);
    }
    Ok(pairs
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect())
}

/// The 32-byte value stored on the chain as the asset's `metadata_hash`.
pub fn hash(class: &str, record: &[u8]) -> [u8; 32] {
    tagged_hash(tags::ASSET_METADATA, &[class.as_bytes(), record])
}

/// The asset id the record implies (`chassi` or `matricula`).
pub fn asset_id(class: &str, record: &[u8]) -> Result<String, MetaError> {
    let key = id_field(class).ok_or(MetaError::UnknownClass)?;
    parse(class, record)?
        .into_iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v)
        .ok_or(MetaError::Missing("identificador"))
}

/// What the device shows before the owner consents (one logical line per entry; the
/// device wraps them).
pub fn summary_lines(class: &str, record: &[u8]) -> Result<Vec<String>, MetaError> {
    let p = parse(class, record)?;
    let g = |k: &str| {
        p.iter()
            .find(|(f, _)| f == k)
            .map(|(_, v)| v.as_str())
            .unwrap_or("")
    };
    Ok(match class {
        "veiculo" => alloc::vec![
            format!("Placa: {}", g("placa")),
            format!("Veículo: {} {}", g("marca"), g("modelo")),
            format!("Ano: {}/{}", g("ano_fabricacao"), g("ano_modelo")),
            format!("Cor: {}", g("cor")),
        ],
        "imovel" => alloc::vec![
            format!("Matrícula: {}", g("matricula")),
            format!("Endereço: {}", g("endereco")),
            format!("Área: {} m²", g("area_m2")),
        ],
        _ => return Err(MetaError::UnknownClass),
    })
}

/// Valid TEST data for the `n`-th asset of `class` (a "fill example" button).
pub fn sample(class: &str, n: u32) -> Option<Vec<(String, String)>> {
    let kv = |a: &[(&str, String)]| {
        a.iter()
            .map(|(k, v)| ((*k).to_string(), v.clone()))
            .collect()
    };
    match class {
        "veiculo" => {
            let base = format!("{:010}", 6_393_659 + u64::from(n));
            let dv = renavam_dv(base.as_bytes());
            let letter = char::from(b'A' + (n / 10 % 26) as u8);
            Some(kv(&[
                ("chassi", format!("9BWZZZ377VT{:06}", n % 1_000_000)),
                ("placa", format!("BRA{}{letter}{:02}", n % 10, n % 100)),
                ("renavam", format!("{base}{dv}")),
                ("marca", "Volkswagen".into()),
                ("modelo", "Gol 1.0".into()),
                ("ano_fabricacao", "2020".into()),
                ("ano_modelo", "2021".into()),
                ("cor", "Prata".into()),
            ]))
        }
        "imovel" => Some(kv(&[
            ("matricula", format!("matricula-{n:03}")),
            ("cartorio", "1º Registro de Imóveis de São Paulo".into()),
            ("endereco", "Rua das Palmeiras, 100, São Paulo/SP".into()),
            ("area_m2", "72,5".into()),
        ])),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn pairs(v: &[(String, String)]) -> Vec<(&str, &str)> {
        v.iter().map(|(k, x)| (k.as_str(), x.as_str())).collect()
    }

    #[test]
    fn renavam_check_digit_matches_a_known_value() {
        assert_eq!(renavam("00639884962").unwrap(), "00639884962");
        assert!(renavam("00639884963").is_err()); // wrong digit
        assert!(renavam("0063988496").is_err()); // too short
        assert!(renavam("0063988496a").is_err());
    }

    #[test]
    fn plates_chassis_and_years() {
        assert_eq!(placa("abc-1234").unwrap(), "ABC1234");
        assert_eq!(placa("BRA2E19").unwrap(), "BRA2E19");
        assert!(placa("AB12345").is_err());
        assert!(placa("ABCD123").is_err());
        assert_eq!(chassi("9bwzzz377vt004251").unwrap(), "9BWZZZ377VT004251");
        assert!(chassi("9BWZZZ377VT00425").is_err()); // 16
        assert!(chassi("9BWZZZ377VT00425I").is_err()); // I is forbidden
        assert!(year("1949").is_err() && year("2101").is_err() && year("21").is_err());
        assert_eq!(area("72,5").unwrap(), "72,5");
        assert!(area("0").is_err() && area("72,").is_err() && area("72,555").is_err());
    }

    #[test]
    fn samples_are_valid_and_round_trip_for_both_classes() {
        for class in ["veiculo", "imovel"] {
            let s = sample(class, 7).unwrap();
            let rec = canonical(class, &pairs(&s)).unwrap();
            let parsed = parse(class, &rec).unwrap();
            assert_eq!(parsed.len(), schema(class).unwrap().len());
            assert!(!summary_lines(class, &rec).unwrap().is_empty());
            assert!(rec.len() <= MAX_RECORD);
        }
    }

    #[test]
    fn the_asset_id_comes_from_the_record() {
        let s = sample("veiculo", 4).unwrap();
        let rec = canonical("veiculo", &pairs(&s)).unwrap();
        assert_eq!(asset_id("veiculo", &rec).unwrap(), "9BWZZZ377VT000004");
        let i = sample("imovel", 4).unwrap();
        let rec = canonical("imovel", &pairs(&i)).unwrap();
        assert_eq!(asset_id("imovel", &rec).unwrap(), "matricula-004");
    }

    #[test]
    fn errors_name_the_field() {
        let mut s = sample("veiculo", 1).unwrap();
        s[2].1 = "12345678901".into(); // bad RENAVAM
        let e = canonical("veiculo", &pairs(&s)).unwrap_err();
        assert!(matches!(
            e,
            MetaError::Invalid {
                field: "renavam",
                ..
            }
        ));
        assert!(e.to_string().starts_with("renavam inválido"));
        let mut s = sample("veiculo", 1).unwrap();
        s.pop();
        assert_eq!(
            canonical("veiculo", &pairs(&s)).unwrap_err(),
            MetaError::Missing("cor")
        );
        let mut s = sample("veiculo", 1).unwrap();
        s.push(("extra".into(), "x".into()));
        assert!(matches!(
            canonical("veiculo", &pairs(&s)),
            Err(MetaError::UnknownField(_))
        ));
        let mut s = sample("veiculo", 1).unwrap();
        s[6].1 = "2023".into(); // model year too far from 2020
        assert!(matches!(
            canonical("veiculo", &pairs(&s)),
            Err(MetaError::Invalid {
                field: "ano_modelo",
                ..
            })
        ));
        assert_eq!(
            canonical("barco", &[]).unwrap_err(),
            MetaError::UnknownClass
        );
    }

    #[test]
    fn only_the_canonical_encoding_is_accepted() {
        let s = sample("imovel", 1).unwrap();
        let rec = canonical("imovel", &pairs(&s)).unwrap();
        let mut reordered: Vec<&str> = core::str::from_utf8(&rec).unwrap().lines().collect();
        reordered.swap(0, 1);
        let bad = format!("{}\n", reordered.join("\n"));
        assert_eq!(
            parse("imovel", bad.as_bytes()).unwrap_err(),
            MetaError::Malformed
        );
        let mut padded = rec.clone();
        padded.extend_from_slice(b"\n");
        assert!(parse("imovel", &padded).is_err());
        assert!(parse("imovel", &alloc::vec![b'x'; MAX_RECORD + 1]).is_err());
    }

    #[test]
    fn the_hash_binds_class_and_content() {
        let s = sample("veiculo", 1).unwrap();
        let rec = canonical("veiculo", &pairs(&s)).unwrap();
        assert_ne!(hash("veiculo", &rec), hash("imovel", &rec));
        let t = sample("veiculo", 2).unwrap();
        let other = canonical("veiculo", &pairs(&t)).unwrap();
        assert_ne!(hash("veiculo", &rec), hash("veiculo", &other));
        assert_eq!(hash("veiculo", &rec), hash("veiculo", &rec));
    }

    proptest! {
        #[test]
        fn samples_always_validate(n in 0u32..2_000_000) {
            for class in ["veiculo", "imovel"] {
                let s = sample(class, n).unwrap();
                let rec = canonical(class, &pairs(&s)).unwrap();
                prop_assert_eq!(parse(class, &rec).unwrap().len(), schema(class).unwrap().len());
            }
        }
    }
}
