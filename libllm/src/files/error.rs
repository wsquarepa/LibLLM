//! Error type for the file-ingestion pipeline.

use std::path::PathBuf;

/// Which delimiter variant collided with a file's body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DelimiterKind {
    Start,
    End,
}

impl std::fmt::Display for DelimiterKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DelimiterKind::Start => f.write_str("<<<FILE …>>>"),
            DelimiterKind::End => f.write_str("<<<END …>>>"),
        }
    }
}

/// Every failure mode of `libllm::files::resolve_all`. Each variant carries
/// enough context for the UI copy to name the offending file.
#[derive(Debug)]
pub enum FileError {
    Missing(PathBuf),
    TooLarge { path: PathBuf, size: usize, cap: usize },
    MessageTooLarge { total: usize, cap: usize },
    BinaryUnsupported(PathBuf),
    PdfNoText(PathBuf),
    Collision { path: PathBuf, kind: DelimiterKind },
    Io { path: PathBuf, source: std::io::Error },
    TooLargeForSummary { path: PathBuf, tokens: usize, limit: usize },
    SummaryTokenize { path: PathBuf, source: anyhow::Error },
}

impl std::fmt::Display for FileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FileError::Missing(path) => {
                write!(f, "file not found: {}", path.display())
            }
            FileError::TooLarge { path, size, cap } => write!(
                f,
                "file too large: {} ({size} bytes > {cap} byte cap)",
                path.display()
            ),
            FileError::MessageTooLarge { total, cap } => write!(
                f,
                "attached files exceed per-message cap: {total} bytes > {cap} byte cap"
            ),
            FileError::BinaryUnsupported(path) => write!(
                f,
                "unsupported binary file: {}",
                path.display()
            ),
            FileError::PdfNoText(path) => write!(
                f,
                "PDF has no extractable text (scanned without OCR?): {}",
                path.display()
            ),
            FileError::Collision { path, kind } => write!(
                f,
                "file body contains the reserved {kind} delimiter: {}",
                path.display()
            ),
            FileError::Io { path, source } => {
                write!(f, "I/O error reading {}: {source}", path.display())
            }
            FileError::TooLargeForSummary { path, tokens, limit } => write!(
                f,
                "file '{}' is too large to summarize ({tokens} tokens, max {limit})",
                path.display()
            ),
            FileError::SummaryTokenize { path, source } => write!(
                f,
                "could not tokenize '{}' for summary size check: {source}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for FileError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            FileError::Io { source, .. } => Some(source),
            FileError::SummaryTokenize { source, .. } => Some(source.as_ref()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_display_names_the_path() {
        let err = FileError::Missing(PathBuf::from("/tmp/nope.txt"));
        assert_eq!(err.to_string(), "file not found: /tmp/nope.txt");
    }

    #[test]
    fn too_large_display_shows_size_and_cap() {
        let err = FileError::TooLarge {
            path: PathBuf::from("/tmp/big.md"),
            size: 1_000_000,
            cap: 524_288,
        };
        assert!(err.to_string().contains("1000000"));
        assert!(err.to_string().contains("524288"));
    }

    #[test]
    fn collision_display_labels_delimiter_kind() {
        let err = FileError::Collision {
            path: PathBuf::from("/tmp/evil.md"),
            kind: DelimiterKind::Start,
        };
        assert!(err.to_string().contains("<<<FILE"));
        let err = FileError::Collision {
            path: PathBuf::from("/tmp/evil.md"),
            kind: DelimiterKind::End,
        };
        assert!(err.to_string().contains("<<<END"));
    }

    #[test]
    fn io_error_exposes_source() {
        let err = FileError::Io {
            path: PathBuf::from("/tmp/x"),
            source: std::io::Error::new(std::io::ErrorKind::PermissionDenied, "nope"),
        };
        assert!(std::error::Error::source(&err).is_some());
    }

    #[test]
    fn too_large_for_summary_display_includes_path_and_counts() {
        let err = FileError::TooLargeForSummary {
            path: PathBuf::from("/tmp/big.md"),
            tokens: 150_000,
            limit: 100_000,
        };
        let s = err.to_string();
        assert!(s.contains("/tmp/big.md"));
        assert!(s.contains("150000"));
        assert!(s.contains("100000"));
    }

    #[test]
    fn summary_tokenize_display_names_path_and_exposes_source() {
        let err = FileError::SummaryTokenize {
            path: PathBuf::from("/tmp/notes.md"),
            source: anyhow::anyhow!("connection refused"),
        };
        let s = err.to_string();
        assert!(s.contains("/tmp/notes.md"));
        assert!(s.contains("connection refused"));
        assert!(std::error::Error::source(&err).is_some());
    }
}
