//! End-to-end resolution pipeline: tokenise → resolve paths → size-check
//! → classify → collision-check → assemble snapshot messages. Produces a
//! `Vec<Message>` ready to push ahead of the user message, or a
//! `FileError` naming the first failure encountered.

use std::path::{Path, PathBuf};

use crate::config::FilesConfig;
use crate::session::{Message, Role};

use super::classify::classify;
use super::error::FileError;
use super::parse::file_reference_ranges;
use super::snapshot::{build_snapshot_body, check_delimiter_collision};

/// One successfully resolved attachment. Used internally and surfaced in
/// tests to assert classification outcomes.
#[derive(Debug, Clone)]
pub struct ResolvedFile {
    pub raw_token: String,
    pub canonical_path: PathBuf,
    pub basename: String,
    pub body: String,
    pub byte_size: usize,
}

/// Resolve and classify every `@<token>` in `content`, producing a list
/// of `Role::System` messages in input order. Returns `Ok(Vec::new())`
/// when `content` contains no file references. Does not touch the
/// synthetic `@stdin` token — callers that handle stdin attach it
/// separately via `stdin_attachment`.
pub fn resolve_all(
    content: &str,
    cwd: &Path,
    config: &FilesConfig,
) -> Result<Vec<Message>, FileError> {
    if !config.enabled {
        return Ok(Vec::new());
    }
    let refs = file_reference_ranges(content);
    let mut files: Vec<ResolvedFile> = Vec::with_capacity(refs.len());
    for r in refs {
        if r.path() == "stdin" {
            continue; // handled by stdin_attachment path, not this loop
        }
        files.push(resolve_one(&r.raw, cwd, config)?);
    }
    finalise(files, config)
}

/// Build a `ResolvedFile` for piped stdin bytes, labelled as `stdin`.
/// Called by the CLI on piped invocations before invoking `resolve_all`
/// on the `@stdin`-appended message text.
pub fn stdin_attachment(
    bytes: Vec<u8>,
    config: &FilesConfig,
) -> Result<ResolvedFile, FileError> {
    let path = PathBuf::from("<stdin>");
    if bytes.len() > config.per_file_bytes {
        return Err(FileError::TooLarge {
            path: path.clone(),
            size: bytes.len(),
            cap: config.per_file_bytes,
        });
    }
    let classified = classify(&path, &bytes)?;
    let text = classified.text().to_owned();
    let basename = "stdin".to_owned();
    check_delimiter_collision(&path, &basename, &text)?;
    Ok(ResolvedFile {
        raw_token: "@stdin".to_owned(),
        canonical_path: path,
        basename,
        body: text,
        byte_size: bytes.len(),
    })
}

/// Consume an already-resolved list (e.g. `stdin_attachment` outputs)
/// plus a `content` body, run the per-token resolution for the rest,
/// and produce the final message list. Used by the CLI path to merge
/// the stdin attachment into the input-order stream.
pub fn resolve_with_prepended(
    prepended: Vec<ResolvedFile>,
    content: &str,
    cwd: &Path,
    config: &FilesConfig,
) -> Result<Vec<Message>, FileError> {
    if !config.enabled {
        return Ok(Vec::new());
    }
    let mut files = prepended;
    for r in file_reference_ranges(content) {
        if r.path() == "stdin" {
            continue;
        }
        files.push(resolve_one(&r.raw, cwd, config)?);
    }
    finalise(files, config)
}

fn resolve_one(
    raw_token: &str,
    cwd: &Path,
    config: &FilesConfig,
) -> Result<ResolvedFile, FileError> {
    let raw_path = &raw_token[1..];
    let path_buf = expand_path(raw_path, cwd);
    let canonical = std::fs::canonicalize(&path_buf).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            FileError::Missing(path_buf.clone())
        } else {
            FileError::Io {
                path: path_buf.clone(),
                source,
            }
        }
    })?;
    let metadata = std::fs::metadata(&canonical).map_err(|source| FileError::Io {
        path: canonical.clone(),
        source,
    })?;
    if !metadata.is_file() {
        return Err(FileError::Missing(canonical));
    }
    let size = metadata.len() as usize;
    if size > config.per_file_bytes {
        return Err(FileError::TooLarge {
            path: canonical,
            size,
            cap: config.per_file_bytes,
        });
    }
    let bytes = std::fs::read(&canonical).map_err(|source| FileError::Io {
        path: canonical.clone(),
        source,
    })?;
    let classified = classify(&canonical, &bytes)?;
    let text = classified.text().to_owned();
    let basename = canonical
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(raw_path)
        .to_owned();
    check_delimiter_collision(&canonical, &basename, &text)?;
    Ok(ResolvedFile {
        raw_token: raw_token.to_owned(),
        canonical_path: canonical,
        basename,
        body: text,
        byte_size: bytes.len(),
    })
}

fn finalise(
    files: Vec<ResolvedFile>,
    config: &FilesConfig,
) -> Result<Vec<Message>, FileError> {
    let total: usize = files.iter().map(|f| f.body.len()).sum();
    if total > config.per_message_bytes {
        return Err(FileError::MessageTooLarge {
            total,
            cap: config.per_message_bytes,
        });
    }
    Ok(files
        .into_iter()
        .map(|f| Message::new(Role::System, build_snapshot_body(&f.basename, &f.body)))
        .collect())
}

fn expand_path(raw: &str, cwd: &Path) -> PathBuf {
    if let Some(rest) = raw.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest);
    }
    if raw == "~"
        && let Some(home) = dirs::home_dir()
    {
        return home;
    }
    let p = Path::new(raw);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        cwd.join(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn config() -> FilesConfig {
        FilesConfig::default()
    }

    #[test]
    fn empty_content_returns_empty_list() {
        let tmp = TempDir::new().unwrap();
        let out = resolve_all("no tokens here", tmp.path(), &config()).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn disabled_config_is_noop() {
        let tmp = TempDir::new().unwrap();
        let cfg = FilesConfig {
            enabled: false,
            ..FilesConfig::default()
        };
        let out = resolve_all("read @anything.md", tmp.path(), &cfg).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn resolves_single_relative_text_file() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("notes.md"), "hello").unwrap();
        let msgs = resolve_all("summarise @notes.md", tmp.path(), &config()).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, Role::System);
        assert!(msgs[0].content.contains("<<<FILE notes.md>>>"));
        assert!(msgs[0].content.contains("hello"));
        assert!(msgs[0].content.contains("<<<END notes.md>>>"));
    }

    #[test]
    fn missing_file_errors_with_missing_variant() {
        let tmp = TempDir::new().unwrap();
        let err = resolve_all("read @nope.md", tmp.path(), &config()).unwrap_err();
        assert!(matches!(err, FileError::Missing(_)));
    }

    #[test]
    fn too_large_file_is_rejected() {
        let tmp = TempDir::new().unwrap();
        let big = vec![b'x'; 1024];
        std::fs::write(tmp.path().join("big.md"), &big).unwrap();
        let cfg = FilesConfig {
            per_file_bytes: 100,
            ..FilesConfig::default()
        };
        let err = resolve_all("read @big.md", tmp.path(), &cfg).unwrap_err();
        assert!(matches!(err, FileError::TooLarge { .. }));
    }

    #[test]
    fn per_message_cap_is_enforced() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("a.md"), "x".repeat(300)).unwrap();
        std::fs::write(tmp.path().join("b.md"), "x".repeat(300)).unwrap();
        let cfg = FilesConfig {
            per_file_bytes: 1024,
            per_message_bytes: 500,
            ..FilesConfig::default()
        };
        let err = resolve_all("cmp @a.md and @b.md", tmp.path(), &cfg).unwrap_err();
        assert!(matches!(err, FileError::MessageTooLarge { .. }));
    }

    #[test]
    fn multiple_files_produce_messages_in_order() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("a.md"), "first").unwrap();
        std::fs::write(tmp.path().join("b.md"), "second").unwrap();
        let msgs = resolve_all("cmp @a.md and @b.md", tmp.path(), &config()).unwrap();
        assert_eq!(msgs.len(), 2);
        assert!(msgs[0].content.contains("<<<FILE a.md>>>"));
        assert!(msgs[1].content.contains("<<<FILE b.md>>>"));
    }

    #[test]
    fn delimiter_collision_in_body_is_rejected() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("x.md"), "<<<FILE x.md>>>").unwrap();
        let err = resolve_all("read @x.md", tmp.path(), &config()).unwrap_err();
        assert!(matches!(err, FileError::Collision { .. }));
    }

    #[test]
    fn stdin_attachment_wraps_bytes() {
        let rf = stdin_attachment(b"piped text".to_vec(), &config()).unwrap();
        assert_eq!(rf.basename, "stdin");
        assert_eq!(rf.body, "piped text");
    }

    #[test]
    fn resolve_with_prepended_merges_stdin() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("extra.md"), "more").unwrap();
        let stdin_rf = stdin_attachment(b"piped".to_vec(), &config()).unwrap();
        let msgs = resolve_with_prepended(
            vec![stdin_rf],
            "summarise @extra.md @stdin",
            tmp.path(),
            &config(),
        )
        .unwrap();
        assert_eq!(msgs.len(), 2);
        assert!(msgs[0].content.contains("<<<FILE stdin>>>"));
        assert!(msgs[1].content.contains("<<<FILE extra.md>>>"));
    }

    #[test]
    fn plain_at_stdin_alone_is_skipped_when_not_prepended() {
        let tmp = TempDir::new().unwrap();
        let msgs = resolve_all("summarise @stdin", tmp.path(), &config()).unwrap();
        assert!(msgs.is_empty(), "bare @stdin without prepended attachment produces no message");
    }
}
