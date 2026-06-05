//! Process-boundary config state: data-directory resolution, config file load/save,
//! and preset directory helpers. Separated from `libllm-core` so that the
//! `test-support` feature (thread-local vs. `OnceLock` toggle) can be activated
//! by consumers' dev-dependencies without affecting the pure domain crate.

use std::path::PathBuf;
use std::time::Instant;

use libllm_core::config::Config;

/// Errors from process-boundary config operations.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// `set_data_dir` was called more than once in a production process.
    #[error("data directory override already set")]
    DataDirAlreadySet,
    /// The data directory could not be created.
    #[error("failed to create data directory {path}: {source}")]
    CreateDataDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// The config could not be serialized to TOML.
    #[error("failed to serialize config: {0}")]
    Serialize(#[source] toml::ser::Error),
    /// The config file could not be written to disk.
    #[error("failed to write config {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: anyhow::Error,
    },
}

#[cfg(not(feature = "test-support"))]
static DATA_DIR_OVERRIDE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

// Tests use a thread-local so parallel integration tests in the same binary
// can each pin their own tempdir without a global serialization lock. Any
// production code that runs on a worker thread (e.g. `tokio::task::spawn_blocking`)
// must NOT read `data_dir()` / `salt_path()` there — capture paths on the
// thread that called `set_data_dir` and pass them in.
#[cfg(feature = "test-support")]
thread_local! {
    static DATA_DIR_OVERRIDE: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

#[cfg(not(feature = "test-support"))]
pub fn set_data_dir(path: PathBuf) -> Result<(), ConfigError> {
    DATA_DIR_OVERRIDE
        .set(path)
        .map_err(|_| ConfigError::DataDirAlreadySet)
}

#[cfg(feature = "test-support")]
pub fn set_data_dir(path: PathBuf) -> Result<(), ConfigError> {
    DATA_DIR_OVERRIDE.with(|cell| {
        *cell.borrow_mut() = Some(path);
    });
    Ok(())
}

#[cfg(not(feature = "test-support"))]
pub fn data_dir() -> PathBuf {
    DATA_DIR_OVERRIDE.get().cloned().unwrap_or_else(|| {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("libllm")
    })
}

#[cfg(feature = "test-support")]
pub fn data_dir() -> PathBuf {
    DATA_DIR_OVERRIDE.with(|cell| {
        cell.borrow().clone().unwrap_or_else(|| {
            dirs::data_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("libllm")
        })
    })
}

pub fn salt_path() -> PathBuf {
    data_dir().join(".salt")
}

pub fn instruct_presets_dir() -> PathBuf {
    data_dir().join("presets").join("instruct")
}

pub fn reasoning_presets_dir() -> PathBuf {
    data_dir().join("presets").join("reasoning")
}

pub fn template_presets_dir() -> PathBuf {
    data_dir().join("presets").join("template")
}

pub fn ensure_dirs() -> Result<(), ConfigError> {
    let dir = data_dir();
    std::fs::create_dir_all(&dir).map_err(|source| ConfigError::CreateDataDir { path: dir, source })
}

pub fn config_path() -> PathBuf {
    data_dir().join("config.toml")
}

fn old_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("libllm").join("config.toml"))
}

pub fn migrate_config() {
    let new_path = config_path();
    if new_path.exists() {
        tracing::info!(result = "skipped", reason = "already_exists", path = %new_path.display(), "config.migrate");
        return;
    }

    let old_path = match old_config_path() {
        Some(p) if p.exists() => p,
        _ => {
            tracing::info!(
                result = "skipped",
                reason = "no_legacy_config",
                "config.migrate"
            );
            return;
        }
    };

    if let Some(parent) = new_path.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        eprintln!("Warning: failed to create config directory: {e}");
    }

    if std::fs::rename(&old_path, &new_path).is_ok() {
        tracing::info!(result = "ok", from = %old_path.display(), to = %new_path.display(), "config.migrate");
        eprintln!("Config migrated to {}", new_path.display());
    } else {
        tracing::error!(result = "error", from = %old_path.display(), to = %new_path.display(), "config.migrate");
    }
}

/// Reads and parses `config.toml` from the data directory.
///
/// Returns `Config::default()` when the file is missing or unparseable (with a
/// warning printed to stderr in the latter case).
pub fn load() -> Config {
    let path = config_path();
    let read_start = Instant::now();
    match std::fs::read_to_string(&path) {
        Ok(contents) => {
            let read_elapsed_ms = read_start.elapsed().as_secs_f64() * 1000.0;
            tracing::info!(phase = "read", result = "ok", path = %path.display(), bytes = contents.len(), elapsed_ms = read_elapsed_ms, "config.load");
            let parse_start = Instant::now();
            match toml::from_str(&contents) {
                Ok(cfg) => {
                    let parse_elapsed_ms = parse_start.elapsed().as_secs_f64() * 1000.0;
                    tracing::info!(phase = "parse", result = "ok", path = %path.display(), elapsed_ms = parse_elapsed_ms, "config.load");
                    cfg
                }
                Err(e) => {
                    let parse_elapsed_ms = parse_start.elapsed().as_secs_f64() * 1000.0;
                    tracing::warn!(phase = "parse", result = "error", path = %path.display(), elapsed_ms = parse_elapsed_ms, error = %e, "config.load");
                    eprintln!("Warning: failed to parse {}: {e}", path.display());
                    Config::default()
                }
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            let read_elapsed_ms = read_start.elapsed().as_secs_f64() * 1000.0;
            tracing::info!(phase = "read", result = "missing", path = %path.display(), elapsed_ms = read_elapsed_ms, "config.load");
            Config::default()
        }
        Err(err) => {
            let read_elapsed_ms = read_start.elapsed().as_secs_f64() * 1000.0;
            tracing::error!(phase = "read", result = "error", path = %path.display(), elapsed_ms = read_elapsed_ms, error = %err, "config.load");
            Config::default()
        }
    }
}

/// Serializes and atomically writes the config to `config.toml` in the data directory.
pub fn save(cfg: &Config) -> Result<(), ConfigError> {
    let path = config_path();
    let serialize_start = Instant::now();
    let toml_str = toml::to_string_pretty(cfg).map_err(ConfigError::Serialize)?;
    let serialize_elapsed_ms = serialize_start.elapsed().as_secs_f64() * 1000.0;
    let path_str = path.display().to_string();
    tracing::info!(
        phase = "serialize",
        result = "ok",
        path = path_str.as_str(),
        bytes = toml_str.len(),
        elapsed_ms = serialize_elapsed_ms,
        "config.save"
    );
    libllm_core::timed_result!(
        tracing::Level::INFO,
        "config.save",
        phase = "write",
        path = path_str.as_str(),
        bytes = toml_str.len()
        ; {
            libllm_core::crypto::write_atomic(&path, toml_str.as_bytes())
                .map_err(|source| ConfigError::Write { path: path.clone(), source })
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn salt_path_under_data_dir() {
        let dir = tempfile::tempdir().unwrap();
        set_data_dir(dir.path().to_path_buf()).ok();
        let path = salt_path();
        assert_eq!(path, dir.path().join(".salt"));
    }
}
