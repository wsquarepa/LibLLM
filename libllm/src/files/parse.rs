//! Tokenise `@<path>` references at word boundaries.

/// Byte range of one `@<path>` token within the raw multi-line input,
/// expressed as (`line`, `start`..`end`) where offsets index into the
/// line returned by `raw.split('\n')`. `end` is exclusive and points one
/// past the final path byte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileReference {
    pub line: usize,
    pub start: usize,
    pub end: usize,
    pub raw: String,
}

impl FileReference {
    /// The path component (the `<path>` in `@<path>`), without the leading `@`.
    pub fn path(&self) -> &str {
        &self.raw[1..]
    }
}

/// Find every `@<path>` token in `raw` anchored at a word boundary: start
/// of input, after a whitespace character, or immediately after a newline.
/// A `\@` prefix escapes the token and is not returned.
///
/// The token extends from `@` to the next whitespace character (or end
/// of line); empty paths (`@` followed immediately by whitespace) are
/// ignored.
pub fn file_reference_ranges(raw: &str) -> Vec<FileReference> {
    let mut out: Vec<FileReference> = Vec::new();
    for (line_idx, line) in raw.split('\n').enumerate() {
        let bytes = line.as_bytes();
        let mut i = 0usize;
        while i < bytes.len() {
            if bytes[i] != b'@' {
                i += 1;
                continue;
            }
            let is_boundary = i == 0 || bytes[i - 1].is_ascii_whitespace();
            let is_escaped = i > 0 && bytes[i - 1] == b'\\';
            if !is_boundary || is_escaped {
                i += 1;
                continue;
            }
            let start = i;
            let mut end = i + 1;
            while end < bytes.len() && !bytes[end].is_ascii_whitespace() {
                end += 1;
            }
            if end > start + 1 {
                out.push(FileReference {
                    line: line_idx,
                    start,
                    end,
                    raw: line[start..end].to_owned(),
                });
            }
            i = end.max(i + 1);
        }
    }
    out
}

/// Unescape literal `\@` sequences produced by escape syntax so the
/// message stored in the tree reads as the user intended. `@` tokens
/// recognised by `file_reference_ranges` are left untouched — this
/// function only rewrites `\@` → `@` globally.
pub fn unescape_at(text: &str) -> String {
    text.replace("\\@", "@")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_has_no_ranges() {
        assert!(file_reference_ranges("hello world").is_empty());
    }

    #[test]
    fn single_at_start_matches() {
        let refs = file_reference_ranges("@notes.md summarise");
        assert_eq!(
            refs,
            vec![FileReference {
                line: 0,
                start: 0,
                end: 9,
                raw: "@notes.md".to_owned(),
            }]
        );
        assert_eq!(refs[0].path(), "notes.md");
    }

    #[test]
    fn at_mid_word_is_ignored() {
        assert!(file_reference_ranges("email@example.com").is_empty());
    }

    #[test]
    fn escaped_at_is_ignored() {
        assert!(file_reference_ranges("prefix \\@literal text").is_empty());
    }

    #[test]
    fn multiple_tokens_on_same_line() {
        let refs = file_reference_ranges("see @a.md and @b.md please");
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].raw, "@a.md");
        assert_eq!(refs[1].raw, "@b.md");
    }

    #[test]
    fn token_after_newline_is_word_boundary() {
        let refs = file_reference_ranges("line one\n@notes.md");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].line, 1);
        assert_eq!(refs[0].start, 0);
    }

    #[test]
    fn trailing_whitespace_terminates_token() {
        let refs = file_reference_ranges("@foo.md\n");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].raw, "@foo.md");
    }

    #[test]
    fn bare_at_is_not_a_token() {
        assert!(file_reference_ranges("@ bare").is_empty());
    }

    #[test]
    fn path_with_tilde_is_captured_verbatim() {
        let refs = file_reference_ranges("read @~/notes/list.md");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].path(), "~/notes/list.md");
    }

    #[test]
    fn unescape_at_strips_backslash() {
        assert_eq!(unescape_at("\\@literal"), "@literal");
        assert_eq!(unescape_at("plain"), "plain");
        assert_eq!(unescape_at("\\@a and \\@b"), "@a and @b");
    }
}
