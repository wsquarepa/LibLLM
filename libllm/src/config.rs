use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};
#[cfg(not(feature = "test-support"))]
use anyhow::anyhow;
use serde::{Deserialize, Serialize};

use crate::sampling::SamplingOverrides;

#[cfg(not(feature = "test-support"))]
static DATA_DIR_OVERRIDE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

#[cfg(feature = "test-support")]
thread_local! {
    static DATA_DIR_OVERRIDE: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    pub api_url: Option<String>,
    #[serde(default, skip_serializing)]
    pub template: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub template_preset: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub instruct_preset: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub reasoning_preset: Option<String>,
    #[serde(default, skip_serializing)]
    pub user_name: Option<String>,
    #[serde(default, skip_serializing)]
    pub user_persona: Option<String>,
    #[serde(default)]
    pub sampling: SamplingOverrides,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub worldbooks: Vec<String>,
    #[serde(default)]
    pub tls_skip_verify: bool,
    #[serde(default)]
    pub debug_log: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub default_persona: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub macros: HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub theme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub theme_colors: Option<ThemeColorOverrides>,
    #[serde(default)]
    pub backup: BackupConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupConfig {
    #[serde(default = "BackupConfig::default_enabled")]
    pub enabled: bool,
    #[serde(default = "BackupConfig::default_keep_all_days")]
    pub keep_all_days: u32,
    #[serde(default = "BackupConfig::default_keep_daily_days")]
    pub keep_daily_days: u32,
    #[serde(default = "BackupConfig::default_keep_weekly_days")]
    pub keep_weekly_days: u32,
    #[serde(default = "BackupConfig::default_rebase_threshold_percent")]
    pub rebase_threshold_percent: u32,
    #[serde(default = "BackupConfig::default_rebase_hard_ceiling")]
    pub rebase_hard_ceiling: u32,
}

impl BackupConfig {
    fn default_enabled() -> bool { true }
    fn default_keep_all_days() -> u32 { 7 }
    fn default_keep_daily_days() -> u32 { 30 }
    fn default_keep_weekly_days() -> u32 { 90 }
    fn default_rebase_threshold_percent() -> u32 { 50 }
    fn default_rebase_hard_ceiling() -> u32 { 10 }
}

impl Default for BackupConfig {
    fn default() -> Self {
        Self {
            enabled: Self::default_enabled(),
            keep_all_days: Self::default_keep_all_days(),
            keep_daily_days: Self::default_keep_daily_days(),
            keep_weekly_days: Self::default_keep_weekly_days(),
            rebase_threshold_percent: Self::default_rebase_threshold_percent(),
            rebase_hard_ceiling: Self::default_rebase_hard_ceiling(),
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ThemeColorOverrides {
    pub user_message: Option<String>,
    pub assistant_message_fg: Option<String>,
    pub assistant_message_bg: Option<String>,
    pub system_message: Option<String>,
    pub border_focused: Option<String>,
    pub border_unfocused: Option<String>,
    pub status_bar_fg: Option<String>,
    pub status_bar_bg: Option<String>,
    pub status_error_fg: Option<String>,
    pub status_error_bg: Option<String>,
    pub status_info_fg: Option<String>,
    pub status_info_bg: Option<String>,
    pub status_warning_fg: Option<String>,
    pub status_warning_bg: Option<String>,
    pub dialogue: Option<String>,
    pub nav_cursor_fg: Option<String>,
    pub nav_cursor_bg: Option<String>,
    pub hover_bg: Option<String>,
    pub dimmed: Option<String>,
    pub sidebar_highlight_fg: Option<String>,
    pub sidebar_highlight_bg: Option<String>,
    pub command_picker_fg: Option<String>,
    pub command_picker_bg: Option<String>,
    pub streaming_indicator: Option<String>,
    pub api_unavailable: Option<String>,
}

const DEFAULT_API_URL: &str = "http://localhost:5001/v1";

impl Config {
    pub fn api_url(&self) -> &str {
        self.api_url.as_deref().unwrap_or(DEFAULT_API_URL)
    }
}

#[cfg(not(feature = "test-support"))]
pub fn set_data_dir(path: PathBuf) -> Result<()> {
    DATA_DIR_OVERRIDE
        .set(path)
        .map_err(|_| anyhow!("data directory override already set"))
}

#[cfg(feature = "test-support")]
pub fn set_data_dir(path: PathBuf) -> Result<()> {
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

pub fn ensure_dirs() -> Result<()> {
    std::fs::create_dir_all(data_dir()).context("failed to create data directory")?;
    crate::preset::ensure_default_presets();
    std::fs::create_dir_all(crate::preset::template_presets_dir())
        .context("failed to create template presets directory")
}

pub fn config_path() -> PathBuf {
    data_dir().join("config.toml")
}

fn old_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("libllm").join("config.toml"))
}

pub(crate) fn migrate_config() {
    let new_path = config_path();
    if new_path.exists() {
        crate::debug_log::log_kv(
            "config.migrate",
            &[
                crate::debug_log::field("result", "skipped"),
                crate::debug_log::field("reason", "already_exists"),
                crate::debug_log::field("path", new_path.display()),
            ],
        );
        return;
    }

    let old_path = match old_config_path() {
        Some(p) if p.exists() => p,
        _ => {
            crate::debug_log::log_kv(
                "config.migrate",
                &[
                    crate::debug_log::field("result", "skipped"),
                    crate::debug_log::field("reason", "no_legacy_config"),
                ],
            );
            return;
        }
    };

    if let Some(parent) = new_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("Warning: failed to create config directory: {e}");
        }
    }

    if std::fs::rename(&old_path, &new_path).is_ok() {
        crate::debug_log::log_kv(
            "config.migrate",
            &[
                crate::debug_log::field("result", "ok"),
                crate::debug_log::field("from", old_path.display()),
                crate::debug_log::field("to", new_path.display()),
            ],
        );
        eprintln!("Config migrated to {}", new_path.display());
    } else {
        crate::debug_log::log_kv(
            "config.migrate",
            &[
                crate::debug_log::field("result", "error"),
                crate::debug_log::field("from", old_path.display()),
                crate::debug_log::field("to", new_path.display()),
            ],
        );
    }
}

pub fn load() -> Config {
    let path = config_path();
    let read_start = Instant::now();
    match std::fs::read_to_string(&path) {
        Ok(contents) => {
            let read_elapsed_ms = read_start.elapsed().as_secs_f64() * 1000.0;
            crate::debug_log::log_kv(
                "config.load",
                &[
                    crate::debug_log::field("phase", "read"),
                    crate::debug_log::field("result", "ok"),
                    crate::debug_log::field("path", path.display()),
                    crate::debug_log::field("bytes", contents.len()),
                    crate::debug_log::field("elapsed_ms", format!("{read_elapsed_ms:.3}")),
                ],
            );
            let parse_start = Instant::now();
            match toml::from_str(&contents) {
                Ok(cfg) => {
                    let parse_elapsed_ms = parse_start.elapsed().as_secs_f64() * 1000.0;
                    crate::debug_log::log_kv(
                        "config.load",
                        &[
                            crate::debug_log::field("phase", "parse"),
                            crate::debug_log::field("result", "ok"),
                            crate::debug_log::field("path", path.display()),
                            crate::debug_log::field("elapsed_ms", format!("{parse_elapsed_ms:.3}")),
                        ],
                    );
                    cfg
                }
                Err(e) => {
                    let parse_elapsed_ms = parse_start.elapsed().as_secs_f64() * 1000.0;
                    crate::debug_log::log_kv(
                        "config.load",
                        &[
                            crate::debug_log::field("phase", "parse"),
                            crate::debug_log::field("result", "error"),
                            crate::debug_log::field("path", path.display()),
                            crate::debug_log::field("elapsed_ms", format!("{parse_elapsed_ms:.3}")),
                            crate::debug_log::field("error", &e),
                        ],
                    );
                    eprintln!("Warning: failed to parse {}: {e}", path.display());
                    Config::default()
                }
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            let read_elapsed_ms = read_start.elapsed().as_secs_f64() * 1000.0;
            crate::debug_log::log_kv(
                "config.load",
                &[
                    crate::debug_log::field("phase", "read"),
                    crate::debug_log::field("result", "missing"),
                    crate::debug_log::field("path", path.display()),
                    crate::debug_log::field("elapsed_ms", format!("{read_elapsed_ms:.3}")),
                ],
            );
            Config::default()
        }
        Err(err) => {
            let read_elapsed_ms = read_start.elapsed().as_secs_f64() * 1000.0;
            crate::debug_log::log_kv(
                "config.load",
                &[
                    crate::debug_log::field("phase", "read"),
                    crate::debug_log::field("result", "error"),
                    crate::debug_log::field("path", path.display()),
                    crate::debug_log::field("elapsed_ms", format!("{read_elapsed_ms:.3}")),
                    crate::debug_log::field("error", err),
                ],
            );
            Config::default()
        }
    }
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

    #[test]
    fn api_url_defaults_when_empty() {
        let cfg = Config::default();
        assert_eq!(cfg.api_url(), "http://localhost:5001/v1");
    }

    #[test]
    fn api_url_returns_custom_when_set() {
        let cfg = Config {
            api_url: Some("http://example.com/v1".to_string()),
            ..Config::default()
        };
        assert_eq!(cfg.api_url(), "http://example.com/v1");
    }

    #[test]
    fn backup_config_defaults_when_missing() {
        let toml_str = r#"
            api_url = "http://localhost:5001/v1"
        "#;
        let cfg: Config = toml::from_str(toml_str).unwrap();
        assert!(cfg.backup.enabled);
        assert_eq!(cfg.backup.keep_all_days, 7);
        assert_eq!(cfg.backup.keep_daily_days, 30);
        assert_eq!(cfg.backup.keep_weekly_days, 90);
        assert_eq!(cfg.backup.rebase_threshold_percent, 50);
        assert_eq!(cfg.backup.rebase_hard_ceiling, 10);
    }

    #[test]
    fn backup_config_round_trips_through_toml() {
        let toml_str = r#"
            [backup]
            enabled = false
            keep_all_days = 14
            keep_daily_days = 60
            keep_weekly_days = 180
            rebase_threshold_percent = 30
            rebase_hard_ceiling = 5
        "#;
        let cfg: Config = toml::from_str(toml_str).unwrap();
        assert!(!cfg.backup.enabled);
        assert_eq!(cfg.backup.keep_all_days, 14);
        assert_eq!(cfg.backup.rebase_hard_ceiling, 5);
    }
}

pub fn save(cfg: &Config) -> Result<()> {
    let path = config_path();
    let serialize_start = Instant::now();
    let toml_str = toml::to_string_pretty(cfg).context("failed to serialize config")?;
    let serialize_elapsed_ms = serialize_start.elapsed().as_secs_f64() * 1000.0;
    crate::debug_log::log_kv(
        "config.save",
        &[
            crate::debug_log::field("phase", "serialize"),
            crate::debug_log::field("result", "ok"),
            crate::debug_log::field("path", path.display()),
            crate::debug_log::field("bytes", toml_str.len()),
            crate::debug_log::field("elapsed_ms", format!("{serialize_elapsed_ms:.3}")),
        ],
    );
    crate::debug_log::timed_result(
        "config.save",
        &[
            crate::debug_log::field("phase", "write"),
            crate::debug_log::field("path", path.display()),
            crate::debug_log::field("bytes", toml_str.len()),
        ],
        || {
            crate::crypto::write_atomic(&path, toml_str.as_bytes())
                .context(format!("failed to write config: {}", path.display()))
        },
    )
}
