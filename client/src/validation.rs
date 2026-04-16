//! Input validation rules for TUI dialog fields.

use std::path::Path;

use anyhow::{Context, Result};
use libllm::debug_log;

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
    debug_log::timed_result(
        "validation.data_dir",
        &[debug_log::field("path", data_path.display())],
        || {
            if data_path.exists() {
                if !data_path.is_dir() {
                    debug_log::log_kv(
                        "validation.data_dir",
                        &[
                            debug_log::field("phase", "summary"),
                            debug_log::field("result", "error"),
                            debug_log::field("reason", "not_a_dir"),
                            debug_log::field("path", data_path.display()),
                        ],
                    );
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
                    debug_log::log_kv(
                        "validation.data_dir",
                        &[
                            debug_log::field("phase", "summary"),
                            debug_log::field("result", "error"),
                            debug_log::field("reason", "not_libllm_dir"),
                            debug_log::field("path", data_path.display()),
                        ],
                    );
                    anyhow::bail!(
                        "--data directory is not empty and does not appear to be a libllm data directory: {}",
                        data_path.display()
                    );
                }
                debug_log::log_kv(
                    "validation.data_dir",
                    &[
                        debug_log::field("phase", "summary"),
                        debug_log::field("result", "ok"),
                        debug_log::field("existed", true),
                        debug_log::field("is_empty", is_empty),
                        debug_log::field("is_libllm_dir", is_libllm),
                        debug_log::field("created", false),
                    ],
                );
                Ok(!is_empty)
            } else {
                std::fs::create_dir_all(data_path).with_context(|| {
                    format!("failed to create --data directory: {}", data_path.display())
                })?;
                debug_log::log_kv(
                    "validation.data_dir",
                    &[
                        debug_log::field("phase", "summary"),
                        debug_log::field("result", "ok"),
                        debug_log::field("existed", false),
                        debug_log::field("created", true),
                    ],
                );
                Ok(false)
            }
        },
    )
}
