use alloc::string::String;
use alloc::vec::Vec;

use teko_crypto::hex;

/// Rejects control characters and bidi overrides (visual spoofing on a tiny
/// trusted screen) and over-long text.
pub fn sanitize(text: &str, max_chars: usize) -> Option<String> {
    let bad = |c: char| {
        c.is_control()
            || ('\u{202A}'..='\u{202E}').contains(&c)
            || ('\u{2066}'..='\u{2069}').contains(&c)
    };
    if text.chars().count() > max_chars || text.chars().any(bad) {
        return None;
    }
    Some(String::from(text))
}

/// Hex in 4-char groups, `per_line` bytes per line (e.g. 8 bytes = 16 hex).
pub fn hex_lines(bytes: &[u8], per_line: usize) -> Vec<String> {
    bytes
        .chunks(per_line.max(1))
        .map(|chunk| {
            let h = hex::encode(chunk);
            let mut out = String::new();
            for (i, c) in h.chars().enumerate() {
                if i > 0 && i % 4 == 0 {
                    out.push(' ');
                }
                out.push(c);
            }
            out
        })
        .collect()
}

/// Splits `s` into lines of at most `width` chars (for IDs and long text).
pub fn wrap(s: &str, width: usize) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    chars
        .chunks(width.max(1))
        .map(|c| c.iter().collect())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_rejects_spoofing() {
        assert!(sanitize("Contrato de aluguel", 100).is_some());
        assert!(sanitize("a\nb", 100).is_none());
        assert!(sanitize("a\u{202E}b", 100).is_none());
        assert!(sanitize("muito longo", 3).is_none());
    }

    #[test]
    fn hex_lines_concatenate_to_hex() {
        let b = [1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let joined: String = hex_lines(&b, 4).concat().replace(' ', "");
        assert_eq!(joined, hex::encode(&b));
    }
}
