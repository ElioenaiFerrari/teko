use alloc::string::String;

/// Rejects control characters, bidi overrides (visual spoofing on a tiny
/// trusted screen), empty and over-long text. Returns the text if safe.
pub fn sanitize_text(text: &str, max_chars: usize) -> Option<String> {
    let bad = |c: char| {
        c.is_control()
            || ('\u{202A}'..='\u{202E}').contains(&c)
            || ('\u{2066}'..='\u{2069}').contains(&c)
            || c == '\u{200B}'
            || c == '\u{FEFF}'
    };
    let n = text.chars().count();
    if n == 0 || n > max_chars || text.chars().any(bad) || text.trim().is_empty() {
        return None;
    }
    Some(String::from(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_spoofing_and_empty() {
        assert!(sanitize_text("Maria Silva", 40).is_some());
        assert!(sanitize_text("", 40).is_none());
        assert!(sanitize_text("   ", 40).is_none());
        assert!(sanitize_text("a\nb", 40).is_none());
        assert!(sanitize_text("a\u{202E}b", 40).is_none());
        assert!(
            sanitize_text("a\u{200B}b", 40).is_none(),
            "zero-width space"
        );
        assert!(sanitize_text("muito longo", 3).is_none());
    }
}
