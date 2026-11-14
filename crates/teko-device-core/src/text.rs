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

/// Wraps `s` at WORD boundaries into lines of at most `width` chars, so a name
/// like "Candidato Boto-Cor-de-Rosa" is never cut mid-word on the trusted
/// screen. A single word longer than `width` (an ID, a hash) is hard-split.
pub fn wrap(s: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut cur_len = 0usize;
    for word in s.split_whitespace() {
        let wlen = word.chars().count();
        if wlen > width {
            if cur_len > 0 {
                lines.push(core::mem::take(&mut cur));
                cur_len = 0;
            }
            let chars: Vec<char> = word.chars().collect();
            for chunk in chars.chunks(width) {
                let part: String = chunk.iter().collect();
                if chunk.len() == width {
                    lines.push(part);
                } else {
                    cur_len = chunk.len();
                    cur = part;
                }
            }
            continue;
        }
        if cur_len == 0 {
            cur.push_str(word);
            cur_len = wlen;
        } else if cur_len + 1 + wlen <= width {
            cur.push(' ');
            cur.push_str(word);
            cur_len += 1 + wlen;
        } else {
            lines.push(core::mem::take(&mut cur));
            cur.push_str(word);
            cur_len = wlen;
        }
    }
    if cur_len > 0 {
        lines.push(cur);
    }
    lines
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
    fn wrap_breaks_on_words_and_hard_splits_only_what_cannot_fit() {
        assert_eq!(
            wrap("11 - Candidato Boto-Cor-de-Rosa", 28),
            vec!["11 - Candidato".to_string(), "Boto-Cor-de-Rosa".to_string()]
        );
        for line in wrap("Governador do estado de São Paulo e arredores", 20) {
            assert!(line.chars().count() <= 20, "{line}");
            assert!(!line.starts_with(' ') && !line.ends_with(' '));
        }
        let joined = wrap("Prefeito de São Paulo", 12).join(" ");
        assert_eq!(joined, "Prefeito de São Paulo", "no word is ever cut");
        // an ID with no spaces is hard-split and reassembles exactly
        let id = "teko1qyapz0ry8824vac4kdkcazh9uxc6saawyqhalhuk";
        let parts = wrap(id, 20);
        assert!(parts.iter().all(|p| p.chars().count() <= 20));
        assert_eq!(parts.concat(), id);
        assert!(wrap("", 10).is_empty());
    }

    #[test]
    fn hex_lines_concatenate_to_hex() {
        let b = [1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let joined: String = hex_lines(&b, 4).concat().replace(' ', "");
        assert_eq!(joined, hex::encode(&b));
    }
}
