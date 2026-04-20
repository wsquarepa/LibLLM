//! Classify a file's bytes as text or PDF, or reject it as an
//! unsupported binary.

use std::path::Path;

use super::error::FileError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Classified {
    Text(String),
    Pdf(String),
}

impl Classified {
    pub fn text(&self) -> &str {
        match self {
            Classified::Text(t) | Classified::Pdf(t) => t,
        }
    }

}

/// Classify `bytes` originating from `path`.
///
/// Order:
///   1. `infer` detects the MIME type from magic bytes.
///   2. If PDF → run `pdf-extract` text extraction; empty → `PdfNoText`.
///   3. If `infer` returns a non-text binary → `BinaryUnsupported`.
///   4. Else UTF-8-validate the full buffer. Valid → `Text`. Invalid →
///      `BinaryUnsupported`.
pub fn classify(path: &Path, bytes: &[u8]) -> Result<Classified, FileError> {
    if let Some(kind) = infer::get(bytes) {
        let mime = kind.mime_type();
        if mime == "application/pdf" {
            return classify_pdf(path, bytes);
        }
        if !mime.starts_with("text/") {
            return Err(FileError::BinaryUnsupported(path.to_path_buf()));
        }
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => Ok(Classified::Text(s.to_owned())),
        Err(_) => Err(FileError::BinaryUnsupported(path.to_path_buf())),
    }
}

fn classify_pdf(path: &Path, bytes: &[u8]) -> Result<Classified, FileError> {
    let text = pdf_extract::extract_text_from_mem(bytes)
        .map_err(|e| FileError::Io {
            path: path.to_path_buf(),
            source: std::io::Error::other(format!("pdf-extract: {e}")),
        })?;
    if text.trim().is_empty() {
        return Err(FileError::PdfNoText(path.to_path_buf()));
    }
    Ok(Classified::Pdf(text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn utf8_text_classifies_as_text() {
        let out = classify(Path::new("/tmp/x.md"), b"hello world").unwrap();
        assert_eq!(out.text(), "hello world");
        assert!(matches!(out, Classified::Text(_)));
    }

    #[test]
    fn extensionless_utf8_still_text() {
        // Dockerfile-style content with no extension.
        let out = classify(Path::new("/tmp/Dockerfile"), b"FROM alpine:3\nRUN echo hi\n").unwrap();
        assert!(matches!(out, Classified::Text(_)));
    }

    #[test]
    fn invalid_utf8_is_unsupported() {
        let bytes = [0xff, 0xfe, 0x00, 0x00];
        let err = classify(Path::new("/tmp/bin"), &bytes).unwrap_err();
        assert!(matches!(err, FileError::BinaryUnsupported(_)));
    }

    #[test]
    fn png_magic_is_unsupported() {
        let mut png = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        png.extend_from_slice(&[0u8; 32]);
        let err = classify(Path::new("/tmp/img.png"), &png).unwrap_err();
        assert!(matches!(err, FileError::BinaryUnsupported(_)));
    }

    #[test]
    fn pdf_with_no_text_is_rejected() {
        // A truncated PDF header. Both `PdfNoText` (empty extraction)
        // and `Io` (pdf-extract parse failure) are valid outcomes for a
        // byte sequence this malformed; the assertion tolerates either.
        let bytes = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n";
        let err = classify(Path::new("/tmp/x.pdf"), bytes).unwrap_err();
        assert!(
            matches!(err, FileError::PdfNoText(_) | FileError::Io { .. }),
            "expected PdfNoText or Io wrapping pdf-extract failure",
        );
    }

    #[test]
    fn utf8_multibyte_content_classifies_as_text() {
        // Japanese characters occupying 3 bytes each in UTF-8.
        let bytes = b"\xE6\x97\xA5\xE6\x9C\xAC\xE8\xAA\x9E";
        let out = classify(Path::new("/tmp/nihongo.txt"), bytes).unwrap();
        assert_eq!(out.text(), "日本語");
        assert!(matches!(out, Classified::Text(_)));
    }
}
