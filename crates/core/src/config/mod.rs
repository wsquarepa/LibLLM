//! Application configuration types: [`Config`], authentication, section defaults, theme color
//! overrides, and CLI flag overrides that shadow config fields.

mod auth;
mod overrides;
mod sections;
mod theme;

pub use auth::{Auth, AuthKind, AuthOverrides, resolve_auth};
pub use overrides::CliOverrides;
pub use sections::{
    BackupConfig, FileSummarizeMode, FilesConfig, GroupChatConfig, MAX_SUMMARIZATION_CONTEXT_SIZE,
    SummarizationConfig,
};
pub use theme::{ColorLabel, ThemeColorOverrides};

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::sampling::SamplingOverrides;

/// Top-level application configuration, serialized as `config.toml` in the data directory.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Config {
    pub api_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub template_preset: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub instruct_preset: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub reasoning_preset: Option<String>,
    #[serde(default)]
    pub sampling: SamplingOverrides,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub worldbooks: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub regex: Vec<crate::regex_rules::RegexRule>,
    #[serde(default)]
    pub tls_skip_verify: bool,
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
    #[serde(default)]
    pub summarization: SummarizationConfig,
    #[serde(default)]
    pub auth: Auth,
    #[serde(default)]
    pub files: FilesConfig,
    #[serde(default)]
    pub group_chat: GroupChatConfig,
}

const DEFAULT_API_URL: &str = "http://localhost:5001/v1";

impl Config {
    pub fn api_url(&self) -> &str {
        self.api_url.as_deref().unwrap_or(DEFAULT_API_URL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
