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

/// Every failure mode of `libllm_core::files::resolve_all`. Each variant carries
/// enough context for the UI copy to name the offending file.
#[derive(Debug, thiserror::Error)]
pub enum FileError {
    #[error("file not found: {}", .0.display())]
    Missing(PathBuf),
    #[error("file too large: {} ({size} bytes > {cap} byte cap)", path.display())]
    TooLarge {
        path: PathBuf,
        size: usize,
        cap: usize,
    },
    #[error("attached files exceed per-message cap: {total} bytes > {cap} byte cap")]
    MessageTooLarge { total: usize, cap: usize },
    #[error("unsupported binary file: {}", .0.display())]
    BinaryUnsupported(PathBuf),
    #[error("PDF has no extractable text (scanned without OCR?): {}", .0.display())]
    PdfNoText(PathBuf),
    #[error("file body contains the reserved {kind} delimiter: {}", path.display())]
    Collision { path: PathBuf, kind: DelimiterKind },
    #[error("I/O error reading {}: {source}", path.display())]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("file '{}' is too large to summarize ({tokens} tokens, max {limit})", path.display())]
    TooLargeForSummary {
        path: PathBuf,
        tokens: usize,
        limit: usize,
    },
    #[error("could not tokenize '{}' for summary size check: {source}", path.display())]
    SummaryTokenize {
        path: PathBuf,
        source: Box<dyn std::error::Error + Send + Sync>,
    },
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
    fn message_too_large_display_shows_total_and_cap() {
        let err = FileError::MessageTooLarge {
            total: 3_000_000,
            cap: 2_097_152,
        };
        assert_eq!(
            err.to_string(),
            "attached files exceed per-message cap: 3000000 bytes > 2097152 byte cap"
        );
    }

    #[test]
    fn binary_unsupported_display_names_the_path() {
        let err = FileError::BinaryUnsupported(PathBuf::from("/tmp/img.png"));
        assert_eq!(err.to_string(), "unsupported binary file: /tmp/img.png");
    }

    #[test]
    fn pdf_no_text_display_names_the_path() {
        let err = FileError::PdfNoText(PathBuf::from("/tmp/scan.pdf"));
        assert_eq!(
            err.to_string(),
            "PDF has no extractable text (scanned without OCR?): /tmp/scan.pdf"
        );
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
    fn io_display_names_path_and_source() {
        let err = FileError::Io {
            path: PathBuf::from("/tmp/x"),
            source: std::io::Error::new(std::io::ErrorKind::PermissionDenied, "nope"),
        };
        assert_eq!(err.to_string(), "I/O error reading /tmp/x: nope");
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
            source: Box::new(std::io::Error::new(
                std::io::ErrorKind::ConnectionRefused,
                "connection refused",
            )),
        };
        let s = err.to_string();
        assert!(s.contains("/tmp/notes.md"));
        assert!(s.contains("connection refused"));
        assert!(std::error::Error::source(&err).is_some());
    }
}
