use std::path::PathBuf;

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

pub fn key_check_path() -> PathBuf {
    data_dir().join(".key_check")
}

pub fn ensure_dirs() -> Result<()> {
    std::fs::create_dir_all(sessions_dir())
        .context("failed to create sessions directory")?;
    std::fs::create_dir_all(characters_dir())
        .context("failed to create characters directory")?;
    std::fs::create_dir_all(worldinfo_dir())
        .context("failed to create worldinfo directory")
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
        return;
    }

    let old_path = match old_config_path() {
        Some(p) if p.exists() => p,
        _ => return,
    };

    if let Some(parent) = new_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    if std::fs::rename(&old_path, &new_path).is_ok() {
        eprintln!("Config migrated to {}", new_path.display());
    }
}

pub fn load() -> Config {
    migrate_config();

    let path = config_path();
    match std::fs::read_to_string(&path) {
        Ok(contents) => match toml::from_str(&contents) {
            Ok(cfg) => cfg,
            Err(e) => {
                eprintln!("Warning: failed to parse {}: {e}", path.display());
                Config::default()
            }
        },
        Err(_) => Config::default(),
    }
}

pub fn save(cfg: &Config) -> Result<()> {
    let path = config_path();
    let toml_str = toml::to_string_pretty(cfg).context("failed to serialize config")?;
    std::fs::write(&path, toml_str).context(format!("failed to write config: {}", path.display()))
}
