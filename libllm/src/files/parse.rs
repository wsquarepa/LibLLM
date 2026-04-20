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
    /// The path component of the token, without the leading `@` and
    /// with any surrounding double quotes stripped. `@notes.md` yields
    /// `notes.md`; `@"Lecture 29 notes.pdf"` yields `Lecture 29 notes.pdf`.
    pub fn path(&self) -> &str {
        let after_at = &self.raw[1..];
        if after_at.len() >= 2 && after_at.starts_with('"') && after_at.ends_with('"') {
            &after_at[1..after_at.len() - 1]
        } else {
            after_at
        }
    }

    /// True if the token uses the quoted form `@"..."`.
    pub fn is_quoted(&self) -> bool {
        let after_at = self.raw.get(1..).unwrap_or("");
        after_at.len() >= 2 && after_at.starts_with('"') && after_at.ends_with('"')
    }
}

/// Find every `@<path>` token in `raw` anchored at a word boundary: start
/// of input, after a whitespace character, or immediately after a newline.
/// A `\@` prefix escapes the token and is not returned.
///
/// Two forms are recognised:
///
/// - **Bare:** `@` followed by one or more non-whitespace, non-quote
///   characters. Token ends at the next whitespace byte or end of line.
/// - **Quoted:** `@"..."` — `@` immediately followed by `"`. Token
///   ends at the next `"` on the same line. This lets paths with
///   spaces survive the tokeniser. An unterminated quote (no closing
///   `"` on the same line) is not a token.
///
/// Empty tokens (`@` alone, `@""`) are ignored.
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
            if !is_boundary {
                i += 1;
                continue;
            }
            let start = i;
            let after_at = start + 1;
            let end_opt = if after_at < bytes.len() && bytes[after_at] == b'"' {
                // Quoted form: find the closing quote on this line.
                let search_from = after_at + 1;
                line[search_from..]
                    .find('"')
                    .map(|rel| search_from + rel + 1)
            } else {
                // Bare form: scan to whitespace.
                let mut e = after_at;
                while e < bytes.len() && !bytes[e].is_ascii_whitespace() {
                    e += 1;
                }
                Some(e)
            };
            let Some(end) = end_opt else {
                i = after_at;
                continue;
            };
            let raw_slice = &line[start..end];
            if !is_empty_token(raw_slice) {
                out.push(FileReference {
                    line: line_idx,
                    start,
                    end,
                    raw: raw_slice.to_owned(),
                });
            }
            i = end.max(i + 1);
        }
    }
    out
}

/// True for tokens whose path component is empty: `@` alone (bare, no
/// chars after), or `@""` (quoted with no content).
fn is_empty_token(raw: &str) -> bool {
    let after_at = match raw.strip_prefix('@') {
        Some(rest) => rest,
        None => return true,
    };
    if after_at.is_empty() {
        return true;
    }
    if after_at == "\"\"" {
        return true;
    }
    false
}

/// Strip the leading backslash from every `\@` sequence in `text`,
/// turning each into a literal `@`. Use after the tokeniser has
/// decided which `@` tokens to act on — escaped `@`s are emitted as
/// plain characters here.
///
/// Simple global replace: `text.replace("\\@", "@")`. This does not
/// implement a backslash escape ladder — `\\@` becomes `@`, not
/// `\@` or `\\ + @`. A message that needs a literal `\` immediately
/// before `@` must use a space or another character to separate them.
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

    #[test]
    fn empty_input_has_no_ranges() {
        assert!(file_reference_ranges("").is_empty());
    }

    #[test]
    fn token_captures_trailing_punctuation() {
        // Tokenizer stops at whitespace, so `@file.md,` includes the comma.
        // This test pins the current behaviour so a future refactor that
        // strips trailing punctuation is caught explicitly.
        let refs = file_reference_ranges("see @file.md, here");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].raw, "@file.md,");
        assert_eq!(refs[0].path(), "file.md,");
    }

    #[test]
    fn quoted_token_captures_spaces() {
        let refs = file_reference_ranges(r#"read @"Lecture 29 notes.pdf" please"#);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].raw, r#"@"Lecture 29 notes.pdf""#);
        assert_eq!(refs[0].path(), "Lecture 29 notes.pdf");
        assert!(refs[0].is_quoted());
    }

    #[test]
    fn quoted_token_next_to_plain_text() {
        let refs = file_reference_ranges(r#"@"a b.md"rest"#);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].raw, r#"@"a b.md""#);
        assert_eq!(refs[0].path(), "a b.md");
    }

    #[test]
    fn unterminated_quote_yields_no_token() {
        let refs = file_reference_ranges(r#"read @"Lecture 29 notes"#);
        assert!(refs.is_empty());
    }

    #[test]
    fn empty_quoted_token_is_ignored() {
        let refs = file_reference_ranges(r#"nothing @"" here"#);
        assert!(refs.is_empty());
    }

    #[test]
    fn quoted_does_not_cross_newline() {
        let refs = file_reference_ranges("@\"foo\nbar\"");
        assert!(
            refs.is_empty(),
            "no closing quote on same line means no token"
        );
    }

    #[test]
    fn bare_and_quoted_tokens_coexist() {
        let refs = file_reference_ranges(r#"cmp @a.md with @"b c.md""#);
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].path(), "a.md");
        assert!(!refs[0].is_quoted());
        assert_eq!(refs[1].path(), "b c.md");
        assert!(refs[1].is_quoted());
    }
}
