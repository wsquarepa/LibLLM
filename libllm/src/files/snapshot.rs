//! Snapshot body assembler, delimiter-collision detector, and
//! detection helpers for identifying file-snapshot `Role::System`
//! nodes in a loaded session tree.

use super::error::{DelimiterKind, FileError};

/// Compose the wire-and-storage body for one attached file.
pub fn build_snapshot_body(basename: &str, text: &str) -> String {
    format!(
        "The user has attached a file. Its name is \"{basename}\" and its contents follow between the <<<FILE {basename}>>> and <<<END {basename}>>> delimiters.\n\n<<<FILE {basename}>>>\n{text}\n<<<END {basename}>>>"
    )
}

/// Fail with `FileError::Collision` if `text` contains either delimiter
/// for the given basename on its own line.
pub fn check_delimiter_collision(
    path: &std::path::Path,
    basename: &str,
    text: &str,
) -> Result<(), FileError> {
    let start_marker = format!("<<<FILE {basename}>>>");
    let end_marker = format!("<<<END {basename}>>>");
    for line in text.lines() {
        if line == start_marker {
            return Err(FileError::Collision {
                path: path.to_path_buf(),
                kind: DelimiterKind::Start,
            });
        }
        if line == end_marker {
            return Err(FileError::Collision {
                path: path.to_path_buf(),
                kind: DelimiterKind::End,
            });
        }
    }
    Ok(())
}

/// True if `content` is a file-snapshot system message: it contains a
/// matched `<<<FILE name>>>` / `<<<END name>>>` pair, each on its own
/// line, for the same `name`.
pub fn is_snapshot(content: &str) -> bool {
    snapshot_basename(content).is_some()
}

/// Extract the basename declared in a snapshot body, or `None` when the
/// content isn't a recognised snapshot.
pub fn snapshot_basename(content: &str) -> Option<String> {
    let mut start_name: Option<&str> = None;
    for line in content.lines() {
        if start_name.is_none() {
            if let Some(rest) = line.strip_prefix("<<<FILE ")
                && let Some(name) = rest.strip_suffix(">>>")
                && !name.is_empty()
            {
                start_name = Some(name);
            }
        } else if let Some(rest) = line.strip_prefix("<<<END ")
            && let Some(name) = rest.strip_suffix(">>>")
            && Some(name) == start_name
        {
            return Some(name.to_owned());
        }
    }
    None
}

/// Returns the substring between the first `<<<FILE name>>>` and the matching
/// `<<<END name>>>` marker, exclusive of the marker lines. Returns `""` when
/// `content` is not a recognised snapshot.
pub fn snapshot_inner_text(content: &str) -> &str {
    let Some(basename) = snapshot_basename(content) else {
        return "";
    };
    let start_marker = format!("<<<FILE {basename}>>>");
    let end_marker = format!("<<<END {basename}>>>");

    let newline_marker = format!("\n{start_marker}");
    let marker_line_start = if content.starts_with(&start_marker) {
        Some(0)
    } else {
        content.find(&newline_marker).map(|idx| idx + 1)
    };
    let start_line_end = match marker_line_start {
        Some(idx) => {
            let after = idx + start_marker.len();
            let rest = &content[after..];
            match rest.find('\n') {
                Some(nl) => after + nl + 1,
                None => return "",
            }
        }
        None => return "",
    };
    let newline_end_marker = format!("\n{end_marker}");
    let end_line_start = match content[start_line_end..].find(&newline_end_marker) {
        Some(rel) => {
            let abs = start_line_end + rel;
            let before = &content[..abs];
            before.trim_end_matches('\n').trim_end_matches('\r').len()
        }
        None => return "",
    };
    if end_line_start <= start_line_end {
        return "";
    }
    &content[start_line_end..end_line_start]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn build_snapshot_body_has_matched_delimiters() {
        let body = build_snapshot_body("notes.md", "hello\nworld");
        assert!(body.contains("<<<FILE notes.md>>>\nhello\nworld\n<<<END notes.md>>>"));
        assert!(body.starts_with("The user has attached a file."));
    }

    #[test]
    fn check_collision_accepts_clean_content() {
        let path = Path::new("/tmp/clean.md");
        assert!(check_delimiter_collision(path, "clean.md", "just text").is_ok());
    }

    #[test]
    fn check_collision_flags_start_marker() {
        let path = Path::new("/tmp/evil.md");
        let body = "normal\n<<<FILE evil.md>>>\nmore";
        let err = check_delimiter_collision(path, "evil.md", body).unwrap_err();
        match err {
            FileError::Collision { kind: DelimiterKind::Start, .. } => (),
            other => panic!("expected Start collision, got {other:?}"),
        }
    }

    #[test]
    fn check_collision_flags_end_marker() {
        let path = Path::new("/tmp/evil.md");
        let body = "<<<END evil.md>>>";
        let err = check_delimiter_collision(path, "evil.md", body).unwrap_err();
        match err {
            FileError::Collision { kind: DelimiterKind::End, .. } => (),
            other => panic!("expected End collision, got {other:?}"),
        }
    }

    #[test]
    fn check_collision_ignores_mismatched_basename() {
        let path = Path::new("/tmp/ok.md");
        let body = "<<<FILE other.md>>>\nnot about us";
        assert!(check_delimiter_collision(path, "ok.md", body).is_ok());
    }

    #[test]
    fn check_collision_requires_whole_line_match() {
        let path = Path::new("/tmp/ok.md");
        let body = "look at <<<FILE ok.md>>> in text";
        assert!(check_delimiter_collision(path, "ok.md", body).is_ok());
    }

    #[test]
    fn is_snapshot_accepts_built_body() {
        let body = build_snapshot_body("file.txt", "content");
        assert!(is_snapshot(&body));
        assert_eq!(snapshot_basename(&body).as_deref(), Some("file.txt"));
    }

    #[test]
    fn is_snapshot_rejects_freeform_system_message() {
        assert!(!is_snapshot("Some manual system message"));
        assert_eq!(snapshot_basename("Some manual system message"), None);
    }

    #[test]
    fn snapshot_basename_rejects_mismatched_names() {
        let body = "<<<FILE a.md>>>\ntext\n<<<END b.md>>>";
        assert_eq!(snapshot_basename(body), None);
    }

    #[test]
    fn snapshot_basename_rejects_empty_name() {
        let body = "<<<FILE >>>\ntext\n<<<END >>>";
        assert_eq!(snapshot_basename(body), None);
    }

    #[test]
    fn snapshot_basename_ignores_interior_file_marker() {
        let body = "<<<FILE a>>>\ninjected\n<<<FILE b>>>\nreal\n<<<END b>>>";
        assert_eq!(snapshot_basename(body), None);
    }

    #[test]
    fn snapshot_basename_returns_outer_name_even_with_interior_markers() {
        let body = "<<<FILE outer>>>\n<<<FILE inner>>>\ntext\n<<<END outer>>>";
        assert_eq!(snapshot_basename(body), Some("outer".to_owned()));
    }

    #[test]
    fn check_collision_detects_crlf_line_endings() {
        let path = std::path::Path::new("/tmp/evil.md");
        let body = "prefix\r\n<<<FILE evil.md>>>\r\nmore\r\n";
        let err = check_delimiter_collision(path, "evil.md", body).unwrap_err();
        match err {
            FileError::Collision { kind: DelimiterKind::Start, .. } => (),
            other => panic!("expected Start collision, got {other:?}"),
        }
    }

    #[test]
    fn snapshot_basename_handles_crlf_body() {
        let body = "The user has attached a file. Its name is \"x.md\" and its contents follow between the <<<FILE x.md>>> and <<<END x.md>>> delimiters.\r\n\r\n<<<FILE x.md>>>\r\nhello\r\nworld\r\n<<<END x.md>>>";
        assert_eq!(snapshot_basename(body).as_deref(), Some("x.md"));
        assert!(is_snapshot(body));
    }

    #[test]
    fn snapshot_inner_text_extracts_body_between_markers() {
        let body = build_snapshot_body("notes.md", "line one\nline two");
        assert_eq!(snapshot_inner_text(&body), "line one\nline two");
    }

    #[test]
    fn snapshot_inner_text_handles_crlf() {
        let body = "<<<FILE x.md>>>\r\nhello\r\nworld\r\n<<<END x.md>>>";
        assert_eq!(snapshot_inner_text(body), "hello\r\nworld");
    }

    #[test]
    fn snapshot_inner_text_returns_empty_for_non_snapshot() {
        assert_eq!(snapshot_inner_text("just freeform text"), "");
    }

    #[test]
    fn snapshot_inner_text_pins_to_outer_markers() {
        let body = "<<<FILE outer>>>\n<<<FILE inner>>>\ntext\n<<<END outer>>>";
        assert_eq!(snapshot_inner_text(body), "<<<FILE inner>>>\ntext");
    }

    #[test]
    fn snapshot_inner_text_empty_body() {
        let body = "<<<FILE z>>>\n<<<END z>>>";
        assert_eq!(snapshot_inner_text(body), "");
    }
}
