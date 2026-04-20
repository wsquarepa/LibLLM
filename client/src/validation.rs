//! Input validation rules for TUI dialog fields.

use std::path::Path;

use anyhow::{Context, Result};

const ENCRYPTED_REQUIRED_MARKERS: &[&str] = &["config.toml", ".salt"];
const PLAINTEXT_ANY_MARKERS: &[&str] = &["config.toml", "data.db"];

/// Check whether a non-empty directory looks like a libllm data directory.
///
/// In encrypted mode (`no_encrypt=false`), requires every file in
/// `ENCRYPTED_REQUIRED_MARKERS` to be present so a dropped `.salt` or a stray
/// `config.toml` alone cannot bless a random directory -- and so a `data.db`
/// sitting without a matching `.salt` is refused rather than silently treated
/// as encrypted. `data.db` is intentionally not required here because it is
/// legitimately absent during a legacy (v1.0) migration before the migrate
/// utility has run.
///
/// In plaintext mode (`no_encrypt=true`), accepts any of
/// `PLAINTEXT_ANY_MARKERS`; encrypted-mode markers are irrelevant.
///
/// A directory containing legacy file-based storage always qualifies so the
/// migration path remains reachable.
pub fn is_libllm_data_dir(path: &Path, no_encrypt: bool) -> bool {
    if crate::legacy_migration::has_legacy_data(path) {
        return true;
    }
    if no_encrypt {
        PLAINTEXT_ANY_MARKERS
            .iter()
            .any(|m| path.join(m).exists())
    } else {
        ENCRYPTED_REQUIRED_MARKERS
            .iter()
            .all(|m| path.join(m).exists())
    }
}

/// Validate and prepare a `--data` directory.
///
/// On success returns whether the directory already contained data
/// (`true`) or was freshly created / empty (`false`). `no_encrypt` selects the
/// marker rules applied by [`is_libllm_data_dir`].
pub fn validate_data_dir(data_path: &Path, no_encrypt: bool) -> Result<bool> {
    let path_str = data_path.display().to_string();
    libllm::timed_result!(tracing::Level::INFO, "validation.data_dir", path = path_str.as_str(), no_encrypt = no_encrypt ; {
        if data_path.exists() {
            if !data_path.is_dir() {
                tracing::warn!(phase = "summary", result = "error", reason = "not_a_dir", path = %data_path.display(), "validation.data_dir");
                anyhow::bail!(
                    "--data path exists but is not a directory: {}",
                    data_path.display()
                );
            }
            let is_empty = std::fs::read_dir(data_path)
                .with_context(|| {
                    format!("failed to read --data directory: {}", data_path.display())
                })?
                .next()
                .is_none();
            let is_libllm = is_libllm_data_dir(data_path, no_encrypt);
            if !is_empty && !is_libllm {
                let has_db = data_path.join("data.db").exists();
                let has_salt = data_path.join(".salt").exists();
                if !no_encrypt && has_db && !has_salt {
                    tracing::warn!(phase = "summary", result = "error", reason = "db_without_salt", path = %data_path.display(), "validation.data_dir");
                    anyhow::bail!(
                        "--data directory has data.db but no .salt: {}\n\
                         pass --no-encrypt to open it as plaintext, or restore the .salt file before proceeding",
                        data_path.display()
                    );
                }
                tracing::warn!(phase = "summary", result = "error", reason = "not_libllm_dir", path = %data_path.display(), no_encrypt = no_encrypt, "validation.data_dir");
                anyhow::bail!(
                    "--data directory is not empty and does not appear to be a libllm data directory: {}",
                    data_path.display()
                );
            }
            tracing::info!(phase = "summary", result = "ok", existed = true, is_empty = is_empty, is_libllm_dir = is_libllm, created = false, "validation.data_dir");
            Ok(!is_empty)
        } else {
            std::fs::create_dir_all(data_path).with_context(|| {
                format!("failed to create --data directory: {}", data_path.display())
            })?;
            tracing::info!(phase = "summary", result = "ok", existed = false, created = true, "validation.data_dir");
            Ok(false)
        }
    })
}
