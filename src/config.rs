use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::sampling::SamplingParams;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    pub api_url: Option<String>,
    pub template: Option<String>,
    pub system_prompt: Option<String>,
    pub sampling: Option<SamplingConfig>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct SamplingConfig {
    pub temperature: Option<f64>,
    pub top_k: Option<i64>,
    pub top_p: Option<f64>,
    pub min_p: Option<f64>,
    pub repeat_last_n: Option<i64>,
    pub repeat_penalty: Option<f64>,
    pub max_tokens: Option<i64>,
}

impl Config {
    pub fn resolve_sampling(&self) -> SamplingParams {
        let defaults = SamplingParams::default();
        match &self.sampling {
            None => defaults,
            Some(s) => SamplingParams {
                temperature: s.temperature.unwrap_or(defaults.temperature),
                top_k: s.top_k.unwrap_or(defaults.top_k),
                top_p: s.top_p.unwrap_or(defaults.top_p),
                min_p: s.min_p.unwrap_or(defaults.min_p),
                repeat_last_n: s.repeat_last_n.unwrap_or(defaults.repeat_last_n),
                repeat_penalty: s.repeat_penalty.unwrap_or(defaults.repeat_penalty),
                max_tokens: s.max_tokens.unwrap_or(defaults.max_tokens),
            },
        }
    }

    pub fn api_url(&self) -> &str {
        self.api_url
            .as_deref()
            .unwrap_or("http://localhost:5001/v1")
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
