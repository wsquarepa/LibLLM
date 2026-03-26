use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::sampling::SamplingOverrides;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    pub api_url: Option<String>,
    pub template: Option<String>,
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub sampling: SamplingOverrides,
}

const DEFAULT_API_URL: &str = "http://localhost:5001/v1";

impl Config {
    pub fn api_url(&self) -> &str {
        self.api_url.as_deref().unwrap_or(DEFAULT_API_URL)
    }
}

pub fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("libllm").join("config.toml"))
}

pub fn load() -> Config {
    let path = match config_path() {
        Some(p) => p,
        None => return Config::default(),
    };

    match std::fs::read_to_string(&path) {
        Ok(contents) => parse(&contents).unwrap_or_default(),
        Err(_) => Config::default(),
    }
}

fn parse(contents: &str) -> Result<Config> {
    toml::from_str(contents).context("failed to parse config file")
}
