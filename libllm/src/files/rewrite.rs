//! Rewrites `@<path>` tokens in a user message body to `[basename]` form
//! for the LLM-facing representation. Storage remains unchanged.

use std::path::Path;

use super::parse::file_reference_ranges;

/// Substitute every `@<path>` token in `body` with `[basename]`, where
/// `basename` is the final path component of `<path>`.
///
/// Escaped tokens (`\@<path>`) are not matched by the tokeniser and pass
/// through unmodified. If a path has no file name component, the full raw
/// path string is used as the label instead.
pub fn rewrite_user_message(body: &str) -> String {
    let refs = file_reference_ranges(body);
    if refs.is_empty() {
        return body.to_owned();
    }

    let mut out = String::with_capacity(body.len());
    let mut cursor_col = 0usize;
    let mut ref_idx = 0usize;
    for (line_idx, line) in body.split('\n').enumerate() {
        if line_idx > 0 {
            out.push('\n');
            cursor_col = 0;
        }
        while ref_idx < refs.len() && refs[ref_idx].line == line_idx {
            let r = &refs[ref_idx];
            out.push_str(&line[cursor_col..r.start]);
            let raw_path = r.path();
            let basename = Path::new(raw_path)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(raw_path);
            out.push_str(&format!("[{basename}]"));
            cursor_col = r.end;
            ref_idx += 1;
        }
        out.push_str(&line[cursor_col..]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_tokens_passes_through() {
        assert_eq!(rewrite_user_message("just text"), "just text");
    }

    #[test]
    fn single_relative_path() {
        assert_eq!(
            rewrite_user_message("summarise @./src/notes.md please"),
            "summarise [notes.md] please",
        );
    }

    #[test]
    fn absolute_path_uses_basename() {
        assert_eq!(
            rewrite_user_message("read @/home/alice/notes.md"),
            "read [notes.md]",
        );
    }

    #[test]
    fn stdin_synthetic_path_maps_to_stdin() {
        assert_eq!(
            rewrite_user_message("summarise @stdin"),
            "summarise [stdin]",
        );
    }

    #[test]
    fn multiple_tokens_on_same_line() {
        assert_eq!(
            rewrite_user_message("cmp @a.md and @b.md"),
            "cmp [a.md] and [b.md]",
        );
    }

    #[test]
    fn tokens_across_lines() {
        assert_eq!(
            rewrite_user_message("start\n@a.md\nmid @b.md end"),
            "start\n[a.md]\nmid [b.md] end",
        );
    }

    #[test]
    fn escaped_token_is_not_rewritten() {
        assert_eq!(
            rewrite_user_message("literal \\@keep"),
            "literal \\@keep",
        );
    }

    #[test]
    fn tilde_prefix_rewrites_to_basename() {
        let out = rewrite_user_message("read @~/notes/list.md");
        assert_eq!(out, "read [list.md]");
    }

    #[test]
    fn quoted_path_rewrites_to_basename_without_quotes() {
        assert_eq!(
            rewrite_user_message(r#"summarise @"Lecture 29 notes.pdf" now"#),
            "summarise [Lecture 29 notes.pdf] now",
        );
    }

    #[test]
    fn quoted_absolute_path_rewrites_to_basename() {
        assert_eq!(
            rewrite_user_message(r#"read @"/home/alice/My Docs/plan.md""#),
            "read [plan.md]",
        );
    }
}
