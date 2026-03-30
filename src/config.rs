use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::sampling::SamplingOverrides;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    pub api_url: Option<String>,
    pub template: Option<String>,
    pub system_prompt: Option<String>,
    pub roleplay_system_prompt: Option<String>,
    pub user_name: Option<String>,
    pub user_persona: Option<String>,
    #[serde(default)]
    pub sampling: SamplingOverrides,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub worldbooks: Vec<String>,
    #[serde(default)]
    pub tls_skip_verify: bool,
}

const DEFAULT_API_URL: &str = "http://localhost:5001/v1";

impl Config {
    pub fn api_url(&self) -> &str {
        self.api_url.as_deref().unwrap_or(DEFAULT_API_URL)
    }
}

pub fn data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("libllm")
}

pub fn sessions_dir() -> PathBuf {
    data_dir().join("sessions")
}

pub fn characters_dir() -> PathBuf {
    data_dir().join("characters")
}

pub fn worldinfo_dir() -> PathBuf {
    data_dir().join("worldinfo")
}

pub fn salt_path() -> PathBuf {
    data_dir().join(".salt")
}

pub fn index_path() -> PathBuf {
    data_dir().join("index.json")
}

pub fn key_check_path() -> PathBuf {
    data_dir().join(".key_check")
}

pub fn ensure_dirs() -> Result<()> {
    std::fs::create_dir_all(sessions_dir()).context("failed to create sessions directory")?;
    std::fs::create_dir_all(characters_dir()).context("failed to create characters directory")?;
    std::fs::create_dir_all(worldinfo_dir()).context("failed to create worldinfo directory")
}

pub fn config_path() -> PathBuf {
    data_dir().join("config.toml")
}

fn old_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("libllm").join("config.toml"))
}

fn migrate_config() {
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
        let _ = std::fs::create_dir_all(parent);
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
    crate::debug_log::timed_kv(
        "config.load",
        &[crate::debug_log::field("phase", "migrate")],
        migrate_config,
    );

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
