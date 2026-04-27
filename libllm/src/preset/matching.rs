//! Match a server-supplied Jinja `chat_template` against the user's instruct presets
//! by rendering both sides over a fixed canonical conversation, normalizing the result,
//! and scoring with normalized Levenshtein distance.

use unicode_normalization::UnicodeNormalization;

const BOS_PREFIXES: &[&str] = &[
    "<|begin_of_text|>",
    "<|begin▁of▁sentence|>",
    "[BOS]",
    "<s>",
];

/// Strip leading BOS-like prefix, NFC normalize, trim, collapse whitespace runs.
pub fn normalize(s: &str) -> String {
    let stripped_nul = s.trim_end_matches('\0');
    let mut after_bos = stripped_nul;
    for prefix in BOS_PREFIXES {
        if let Some(rest) = after_bos.strip_prefix(prefix) {
            after_bos = rest;
            break;
        }
    }
    let nfc: String = after_bos.nfc().collect();
    let trimmed = nfc.trim();
    collapse_whitespace(trimmed)
}

fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut newline_run = 0_usize;
    let mut space_run = 0_usize;
    for ch in s.chars() {
        match ch {
            '\n' => {
                space_run = 0;
                newline_run += 1;
                if newline_run <= 2 {
                    out.push('\n');
                }
            }
            ' ' | '\t' => {
                newline_run = 0;
                space_run += 1;
                if space_run <= 1 {
                    out.push(' ');
                }
            }
            other => {
                newline_run = 0;
                space_run = 0;
                out.push(other);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_trailing_nul() {
        assert_eq!(normalize("hello\0\0\0"), "hello");
    }

    #[test]
    fn strips_known_bos_prefixes() {
        assert_eq!(normalize("<|begin_of_text|>hello"), "hello");
        assert_eq!(normalize("<s>hello"), "hello");
        assert_eq!(normalize("[BOS]hello"), "hello");
    }

    #[test]
    fn trims_leading_and_trailing_whitespace() {
        assert_eq!(normalize("   hello   "), "hello");
    }

    #[test]
    fn collapses_repeated_spaces() {
        assert_eq!(normalize("a    b"), "a b");
    }

    #[test]
    fn collapses_three_or_more_newlines_to_two() {
        assert_eq!(normalize("a\n\n\n\nb"), "a\n\nb");
    }

    #[test]
    fn preserves_two_newlines_unchanged() {
        assert_eq!(normalize("a\n\nb"), "a\n\nb");
    }

    #[test]
    fn does_not_lowercase() {
        assert_eq!(normalize("System User"), "System User");
    }

    #[test]
    fn empty_input_returns_empty() {
        assert_eq!(normalize(""), "");
    }

    #[test]
    fn nfc_normalizes_decomposed_chars() {
        // "é" composed vs decomposed
        let composed = "\u{00E9}";       // é
        let decomposed = "e\u{0301}";    // e + combining acute
        assert_eq!(normalize(decomposed), composed);
    }
}
