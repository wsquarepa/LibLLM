//! Input validation rules for TUI dialog fields.

use std::path::Path;

use anyhow::{Context, Result};

/// Check whether a non-empty directory looks like a libllm data directory.
///
/// In encrypted mode (`no_encrypt=false`), both `.salt` and `data.db` must be
/// present. A lone `.salt` or a lone `data.db` is refused so a stray marker
/// cannot bless a random directory and a mismatched pair is surfaced to the
/// user via [`validate_data_dir`].
///
/// In plaintext mode (`no_encrypt=true`), `data.db` is the sole marker.
/// `.salt` is encrypted-mode only and irrelevant here.
///
/// A directory containing legacy file-based storage always qualifies so the
/// migration path remains reachable.
pub fn is_libllm_data_dir(path: &Path, no_encrypt: bool) -> bool {
    if crate::legacy_migration::has_legacy_data(path) {
        return true;
    }
    let has_db = path.join("data.db").exists();
    if no_encrypt {
        has_db
    } else {
        has_db && path.join(".salt").exists()
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
                if !no_encrypt && has_salt && !has_db {
                    tracing::warn!(phase = "summary", result = "error", reason = "salt_without_db", path = %data_path.display(), "validation.data_dir");
                    anyhow::bail!(
                        "--data directory has .salt but no data.db: {}\n\
                         restore the data.db file or remove the .salt to start fresh",
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
