//! Input validation rules for TUI dialog fields.

use std::path::Path;

use anyhow::{Context, Result};

const LIBLLM_MARKERS: &[&str] = &["config.toml", "data.db"];

/// Check whether a non-empty directory looks like a libllm data directory.
///
/// Returns `true` when at least one known marker file is present.
/// The recognised markers are: `config.toml` and `data.db`.
pub fn is_libllm_data_dir(path: &Path) -> bool {
    LIBLLM_MARKERS.iter().any(|m| path.join(m).exists())
}

/// Validate and prepare a `--data` directory.
///
/// On success returns whether the directory already contained data
/// (`true`) or was freshly created / empty (`false`).
pub fn validate_data_dir(data_path: &Path) -> Result<bool> {
    let path_str = data_path.display().to_string();
    libllm::timed_result!(tracing::Level::INFO, "validation.data_dir", path = path_str.as_str() ; {
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
            let is_libllm = is_libllm_data_dir(data_path);
            if !is_empty && !is_libllm {
                tracing::warn!(phase = "summary", result = "error", reason = "not_libllm_dir", path = %data_path.display(), "validation.data_dir");
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
