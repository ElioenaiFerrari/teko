//! Coin amounts (ADR-0029): the ledger stores integers in **ypy**, the
//! smallest unit; humans see **TEKO** with 6 decimals (1 TEKO = 1,000,000 ypy).
//! Exact integer math only: no floating point, no rounding.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// Decimal places of the coin.
pub const DECIMALS: u32 = 6;
/// Smallest units (ypy) in one TEKO.
pub const YPY_PER_TEKO: u64 = 1_000_000;
/// Coin symbol.
pub const SYMBOL: &str = "TEKO";
/// Name of the smallest unit.
pub const UNIT: &str = "ypy";

/// Why a typed amount was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AmountError {
    Empty,
    /// Characters other than digits and separators (signs, exponents, ...).
    Invalid,
    /// More than 6 decimal places.
    TooManyDecimals,
    /// Does not fit in the ledger integer.
    Overflow,
}

impl core::fmt::Display for AmountError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            AmountError::Empty => "valor vazio",
            AmountError::Invalid => "valor inválido (use dígitos, vírgula e ponto)",
            AmountError::TooManyDecimals => "no máximo 6 casas decimais",
            AmountError::Overflow => "valor grande demais",
        })
    }
}

/// `1234567` ypy → `"1,234567"`; pt-BR, thousands dots, no trailing zeros, no symbol.
pub fn format_number(ypy: u64) -> String {
    let whole = ypy / YPY_PER_TEKO;
    let frac = ypy % YPY_PER_TEKO;
    let digits = format!("{whole}");
    let mut out = String::new();
    let len = digits.len();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            out.push('.');
        }
        out.push(c);
    }
    if frac != 0 {
        let f = format!("{frac:06}");
        out.push(',');
        out.push_str(f.trim_end_matches('0'));
    }
    out
}

/// Like [`format_number`] with the symbol: `"1.234,5 TEKO"`.
pub fn format_amount(ypy: u64) -> String {
    format!("{} {SYMBOL}", format_number(ypy))
}

/// Parses what a person types into ypy. Accepts `1234`, `1234,5`, `1.234,5`
/// and `1234.5` (a lone dot is decimal unless it forms thousands groups).
pub fn parse_amount(text: &str) -> Result<u64, AmountError> {
    let t = text.trim();
    let t = t.strip_suffix(SYMBOL).map(str::trim).unwrap_or(t);
    if t.is_empty() {
        return Err(AmountError::Empty);
    }
    if !t
        .chars()
        .all(|c| c.is_ascii_digit() || c == ',' || c == '.')
    {
        return Err(AmountError::Invalid);
    }
    let (int_part, frac_part): (String, &str) = if let Some((i, f)) = t.split_once(',') {
        if f.contains(',') || f.contains('.') {
            return Err(AmountError::Invalid);
        }
        (strip_groups(i)?, f)
    } else if t.matches('.').count() == 1 {
        let (i, f) = t.split_once('.').ok_or(AmountError::Invalid)?;
        if f.len() == 3 && !i.is_empty() && i.len() <= 3 && i != "0" {
            // "1.234": thousands group.
            (strip_groups(t)?, "")
        } else {
            (i.into(), f)
        }
    } else {
        (strip_groups(t)?, "")
    };
    if int_part.is_empty() && frac_part.is_empty() {
        return Err(AmountError::Invalid);
    }
    if frac_part.len() > DECIMALS as usize {
        return Err(AmountError::TooManyDecimals);
    }
    if !int_part.chars().all(|c| c.is_ascii_digit())
        || !frac_part.chars().all(|c| c.is_ascii_digit())
    {
        return Err(AmountError::Invalid);
    }
    let whole: u64 = if int_part.is_empty() {
        0
    } else {
        int_part.parse().map_err(|_| AmountError::Overflow)?
    };
    let mut frac_digits: String = frac_part.into();
    while frac_digits.len() < DECIMALS as usize {
        frac_digits.push('0');
    }
    let frac: u64 = frac_digits.parse().map_err(|_| AmountError::Invalid)?;
    whole
        .checked_mul(YPY_PER_TEKO)
        .and_then(|w| w.checked_add(frac))
        .ok_or(AmountError::Overflow)
}

/// Removes thousands dots, requiring well-formed groups of 3.
fn strip_groups(s: &str) -> Result<String, AmountError> {
    if !s.contains('.') {
        return Ok(s.into());
    }
    let groups: Vec<&str> = s.split('.').collect();
    let (first, rest) = groups.split_first().ok_or(AmountError::Invalid)?;
    if first.is_empty() || first.len() > 3 || rest.iter().any(|g| g.len() != 3) {
        return Err(AmountError::Invalid);
    }
    Ok(groups.concat())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn formats() {
        assert_eq!(format_amount(0), "0 TEKO");
        assert_eq!(format_amount(50_000_000), "50 TEKO");
        assert_eq!(format_amount(162), "0,000162 TEKO");
        assert_eq!(format_amount(1_234_500_000), "1.234,5 TEKO");
        assert_eq!(format_amount(21_000_000_000_000), "21.000.000 TEKO");
        assert_eq!(format_number(1), "0,000001");
    }

    #[test]
    fn parses() {
        assert_eq!(parse_amount("1234"), Ok(1_234_000_000));
        assert_eq!(parse_amount("1234,5"), Ok(1_234_500_000));
        assert_eq!(parse_amount("1.234,5"), Ok(1_234_500_000));
        assert_eq!(parse_amount("1234.5"), Ok(1_234_500_000));
        assert_eq!(parse_amount("1.234"), Ok(1_234_000_000));
        assert_eq!(parse_amount("0,5"), Ok(500_000));
        assert_eq!(parse_amount("0.5"), Ok(500_000));
        assert_eq!(parse_amount(",5"), Ok(500_000));
        assert_eq!(parse_amount("12,345678 TEKO"), Ok(12_345_678));
        assert_eq!(
            parse_amount("12,3456789"),
            Err(AmountError::TooManyDecimals)
        );
        assert_eq!(parse_amount(""), Err(AmountError::Empty));
        assert_eq!(parse_amount("-1"), Err(AmountError::Invalid));
        assert_eq!(parse_amount("1e3"), Err(AmountError::Invalid));
        assert_eq!(parse_amount("1,2,3"), Err(AmountError::Invalid));
        assert_eq!(parse_amount("12.34.5"), Err(AmountError::Invalid));
        assert_eq!(
            parse_amount("99999999999999999999"),
            Err(AmountError::Overflow)
        );
    }

    proptest! {
        #[test]
        fn roundtrip(x in 0u64..=21_000_000_000_000u64) {
            prop_assert_eq!(parse_amount(&format_amount(x)), Ok(x));
        }
        #[test]
        fn roundtrip_any(x in any::<u64>()) {
            prop_assert_eq!(parse_amount(&format_number(x)), Ok(x));
        }
    }
}
