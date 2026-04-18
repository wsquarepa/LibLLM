//! Application configuration with TOML persistence and default resolution.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};
#[cfg(not(feature = "test-support"))]
use anyhow::anyhow;
use reqwest::header::{HeaderName, HeaderValue, InvalidHeaderName, InvalidHeaderValue};
use serde::{Deserialize, Serialize};

use crate::sampling::SamplingOverrides;

#[cfg(not(feature = "test-support"))]
static DATA_DIR_OVERRIDE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

#[cfg(feature = "test-support")]
thread_local! {
    static DATA_DIR_OVERRIDE: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// Discriminator for `Auth` — used for labels and UI state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthKind {
    None,
    Basic,
    Bearer,
    Header,
    Query,
}

impl std::fmt::Display for AuthKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            AuthKind::None => "None",
            AuthKind::Basic => "Basic",
            AuthKind::Bearer => "Bearer",
            AuthKind::Header => "Header",
            AuthKind::Query => "Query",
        };
        f.write_str(s)
    }
}

/// Outbound-request authentication configuration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Auth {
    None,
    Basic { username: String, password: String },
    Bearer { token: String },
    Header { name: String, value: String },
    Query { name: String, value: String },
}

impl Default for Auth {
    fn default() -> Self {
        Auth::None
    }
}

impl Auth {
    pub fn kind(&self) -> AuthKind {
        match self {
            Auth::None => AuthKind::None,
            Auth::Basic { .. } => AuthKind::Basic,
            Auth::Bearer { .. } => AuthKind::Bearer,
            Auth::Header { .. } => AuthKind::Header,
            Auth::Query { .. } => AuthKind::Query,
        }
    }

    pub fn display_label(&self) -> &'static str {
        match self.kind() {
            AuthKind::None => "None",
            AuthKind::Basic => "Basic",
            AuthKind::Bearer => "Bearer",
            AuthKind::Header => "Header",
            AuthKind::Query => "Query",
        }
    }

    pub fn basic_username(&self) -> String {
        match self {
            Auth::Basic { username, .. } => username.clone(),
            _ => String::new(),
        }
    }

    pub fn basic_password(&self) -> String {
        match self {
            Auth::Basic { password, .. } => password.clone(),
            _ => String::new(),
        }
    }

    pub fn bearer_token(&self) -> String {
        match self {
            Auth::Bearer { token } => token.clone(),
            _ => String::new(),
        }
    }

    pub fn header_name(&self) -> String {
        match self {
            Auth::Header { name, .. } => name.clone(),
            _ => String::new(),
        }
    }

    pub fn header_value(&self) -> String {
        match self {
            Auth::Header { value, .. } => value.clone(),
            _ => String::new(),
        }
    }

    pub fn query_name(&self) -> String {
        match self {
            Auth::Query { name, .. } => name.clone(),
            _ => String::new(),
        }
    }

    pub fn query_value(&self) -> String {
        match self {
            Auth::Query { value, .. } => value.clone(),
            _ => String::new(),
        }
    }

    pub fn validate(&self) -> std::result::Result<(), AuthError> {
        match self {
            Auth::None => Ok(()),
            Auth::Basic { username, password } => {
                if username.is_empty() {
                    return Err(AuthError::EmptyRequiredField { variant: AuthKind::Basic, field: "username" });
                }
                if password.is_empty() {
                    return Err(AuthError::EmptyRequiredField { variant: AuthKind::Basic, field: "password" });
                }
                Ok(())
            }
            Auth::Bearer { token } => {
                if token.is_empty() {
                    return Err(AuthError::EmptyRequiredField { variant: AuthKind::Bearer, field: "token" });
                }
                Ok(())
            }
            Auth::Header { name, value } => {
                if name.is_empty() {
                    return Err(AuthError::EmptyRequiredField { variant: AuthKind::Header, field: "name" });
                }
                if value.is_empty() {
                    return Err(AuthError::EmptyRequiredField { variant: AuthKind::Header, field: "value" });
                }
                HeaderName::from_bytes(name.as_bytes()).map_err(AuthError::InvalidHeaderName)?;
                HeaderValue::from_str(value).map_err(AuthError::InvalidHeaderValue)?;
                Ok(())
            }
            Auth::Query { name, value } => {
                if name.is_empty() {
                    return Err(AuthError::EmptyRequiredField { variant: AuthKind::Query, field: "name" });
                }
                if value.is_empty() {
                    return Err(AuthError::EmptyRequiredField { variant: AuthKind::Query, field: "value" });
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug)]
pub enum AuthError {
    EmptyRequiredField { variant: AuthKind, field: &'static str },
    InvalidHeaderName(InvalidHeaderName),
    InvalidHeaderValue(InvalidHeaderValue),
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthError::EmptyRequiredField { variant, field } => {
                write!(f, "auth {variant}: {field} is required")
            }
            AuthError::InvalidHeaderName(e) => write!(f, "invalid header name: {e}"),
            AuthError::InvalidHeaderValue(e) => write!(f, "invalid header value: {e}"),
        }
    }
}

impl std::error::Error for AuthError {}

/// Plain-data bundle of CLI- and env-sourced overrides for the `Auth` config.
/// Populated by the `client` crate from `CliOverrides` and env vars.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AuthOverrides {
    pub auth_type: Option<AuthKind>,
    pub auth_basic_username: Option<String>,
    pub auth_basic_password: Option<String>,
    pub auth_bearer_token: Option<String>,
    pub auth_header_name: Option<String>,
    pub auth_header_value: Option<String>,
    pub auth_query_name: Option<String>,
    pub auth_query_value: Option<String>,
}

fn pick(override_value: &Option<String>, fallback: String) -> String {
    override_value.clone().unwrap_or(fallback)
}

/// Resolves the effective `Auth` by merging CLI/env overrides onto the on-disk config.
///
/// Precedence: CLI/env > on-disk config. Field accessors return empty strings when the
/// on-disk variant doesn't match the effective kind, so a CLI-set type can stand alone.
pub fn resolve_auth(config: &Config, overrides: &AuthOverrides) -> Auth {
    let kind = overrides.auth_type.unwrap_or_else(|| config.auth.kind());
    match kind {
        AuthKind::None => Auth::None,
        AuthKind::Basic => Auth::Basic {
            username: pick(&overrides.auth_basic_username, config.auth.basic_username()),
            password: pick(&overrides.auth_basic_password, config.auth.basic_password()),
        },
        AuthKind::Bearer => Auth::Bearer {
            token: pick(&overrides.auth_bearer_token, config.auth.bearer_token()),
        },
        AuthKind::Header => Auth::Header {
            name: pick(&overrides.auth_header_name, config.auth.header_name()),
            value: pick(&overrides.auth_header_value, config.auth.header_value()),
        },
        AuthKind::Query => Auth::Query {
            name: pick(&overrides.auth_query_name, config.auth.query_name()),
            value: pick(&overrides.auth_query_value, config.auth.query_value()),
        },
    }
}

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
}

const DEFAULT_SUMMARIZATION_PROMPT: &str =
    "Summarize the following conversation. Preserve key decisions, important details, character information, and narrative developments. Be concise but comprehensive.";

const DEFAULT_CONTEXT_SIZE: usize = 8192;

const DEFAULT_TRIGGER_THRESHOLD: usize = 5;

fn default_summarization_enabled() -> bool {
    true
}

fn default_context_size() -> usize {
    DEFAULT_CONTEXT_SIZE
}

fn default_trigger_threshold() -> usize {
    DEFAULT_TRIGGER_THRESHOLD
}

fn default_summarization_prompt() -> String {
    DEFAULT_SUMMARIZATION_PROMPT.to_owned()
}

/// Auto-summarization settings, nested under `[summarization]` in config.toml.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummarizationConfig {
    #[serde(default = "default_summarization_enabled")]
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub api_url: Option<String>,
    #[serde(default = "default_context_size")]
    pub context_size: usize,
    #[serde(default = "default_trigger_threshold")]
    pub trigger_threshold: usize,
    #[serde(default = "default_summarization_prompt")]
    pub prompt: String,
}

impl Default for SummarizationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            api_url: None,
            context_size: DEFAULT_CONTEXT_SIZE,
            trigger_threshold: DEFAULT_TRIGGER_THRESHOLD,
            prompt: DEFAULT_SUMMARIZATION_PROMPT.to_owned(),
        }
    }
}

/// Backup retention and rebase policy settings, nested under `[backup]` in config.toml.
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

/// Optional color overrides for TUI theme elements, specified as CSS-style hex strings.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
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
    pub summary_indicator: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorLabel {
    UserMessage,
    AssistantMessageFg,
    AssistantMessageBg,
    SystemMessage,
    Dialogue,
    BorderFocused,
    BorderUnfocused,
    StatusBarFg,
    StatusBarBg,
    StatusErrorFg,
    StatusErrorBg,
    StatusInfoFg,
    StatusInfoBg,
    StatusWarningFg,
    StatusWarningBg,
    NavCursorFg,
    NavCursorBg,
    HoverBg,
    SidebarHighlightFg,
    SidebarHighlightBg,
    Dimmed,
    CommandPickerFg,
    CommandPickerBg,
    StreamingIndicator,
    ApiUnavailable,
    SummaryIndicator,
}

impl ColorLabel {
    pub const ALL: [ColorLabel; 26] = [
        Self::UserMessage,
        Self::AssistantMessageFg,
        Self::AssistantMessageBg,
        Self::SystemMessage,
        Self::Dialogue,
        Self::BorderFocused,
        Self::BorderUnfocused,
        Self::StatusBarFg,
        Self::StatusBarBg,
        Self::StatusErrorFg,
        Self::StatusErrorBg,
        Self::StatusInfoFg,
        Self::StatusInfoBg,
        Self::StatusWarningFg,
        Self::StatusWarningBg,
        Self::NavCursorFg,
        Self::NavCursorBg,
        Self::HoverBg,
        Self::SidebarHighlightFg,
        Self::SidebarHighlightBg,
        Self::Dimmed,
        Self::CommandPickerFg,
        Self::CommandPickerBg,
        Self::StreamingIndicator,
        Self::ApiUnavailable,
        Self::SummaryIndicator,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::UserMessage => "user_message",
            Self::AssistantMessageFg => "assistant_message_fg",
            Self::AssistantMessageBg => "assistant_message_bg",
            Self::SystemMessage => "system_message",
            Self::Dialogue => "dialogue",
            Self::BorderFocused => "border_focused",
            Self::BorderUnfocused => "border_unfocused",
            Self::StatusBarFg => "status_bar_fg",
            Self::StatusBarBg => "status_bar_bg",
            Self::StatusErrorFg => "status_error_fg",
            Self::StatusErrorBg => "status_error_bg",
            Self::StatusInfoFg => "status_info_fg",
            Self::StatusInfoBg => "status_info_bg",
            Self::StatusWarningFg => "status_warning_fg",
            Self::StatusWarningBg => "status_warning_bg",
            Self::NavCursorFg => "nav_cursor_fg",
            Self::NavCursorBg => "nav_cursor_bg",
            Self::HoverBg => "hover_bg",
            Self::SidebarHighlightFg => "sidebar_highlight_fg",
            Self::SidebarHighlightBg => "sidebar_highlight_bg",
            Self::Dimmed => "dimmed",
            Self::CommandPickerFg => "command_picker_fg",
            Self::CommandPickerBg => "command_picker_bg",
            Self::StreamingIndicator => "streaming_indicator",
            Self::ApiUnavailable => "api_unavailable",
            Self::SummaryIndicator => "summary_indicator",
        }
    }

    pub fn from_name(label: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|l| l.name() == label)
    }
}

impl ThemeColorOverrides {
    pub fn get(&self, label: ColorLabel) -> Option<&str> {
        let slot = match label {
            ColorLabel::UserMessage => &self.user_message,
            ColorLabel::AssistantMessageFg => &self.assistant_message_fg,
            ColorLabel::AssistantMessageBg => &self.assistant_message_bg,
            ColorLabel::SystemMessage => &self.system_message,
            ColorLabel::Dialogue => &self.dialogue,
            ColorLabel::BorderFocused => &self.border_focused,
            ColorLabel::BorderUnfocused => &self.border_unfocused,
            ColorLabel::StatusBarFg => &self.status_bar_fg,
            ColorLabel::StatusBarBg => &self.status_bar_bg,
            ColorLabel::StatusErrorFg => &self.status_error_fg,
            ColorLabel::StatusErrorBg => &self.status_error_bg,
            ColorLabel::StatusInfoFg => &self.status_info_fg,
            ColorLabel::StatusInfoBg => &self.status_info_bg,
            ColorLabel::StatusWarningFg => &self.status_warning_fg,
            ColorLabel::StatusWarningBg => &self.status_warning_bg,
            ColorLabel::NavCursorFg => &self.nav_cursor_fg,
            ColorLabel::NavCursorBg => &self.nav_cursor_bg,
            ColorLabel::HoverBg => &self.hover_bg,
            ColorLabel::SidebarHighlightFg => &self.sidebar_highlight_fg,
            ColorLabel::SidebarHighlightBg => &self.sidebar_highlight_bg,
            ColorLabel::Dimmed => &self.dimmed,
            ColorLabel::CommandPickerFg => &self.command_picker_fg,
            ColorLabel::CommandPickerBg => &self.command_picker_bg,
            ColorLabel::StreamingIndicator => &self.streaming_indicator,
            ColorLabel::ApiUnavailable => &self.api_unavailable,
            ColorLabel::SummaryIndicator => &self.summary_indicator,
        };
        slot.as_deref()
    }

    pub fn set(&mut self, label: ColorLabel, value: Option<String>) {
        let slot = match label {
            ColorLabel::UserMessage => &mut self.user_message,
            ColorLabel::AssistantMessageFg => &mut self.assistant_message_fg,
            ColorLabel::AssistantMessageBg => &mut self.assistant_message_bg,
            ColorLabel::SystemMessage => &mut self.system_message,
            ColorLabel::Dialogue => &mut self.dialogue,
            ColorLabel::BorderFocused => &mut self.border_focused,
            ColorLabel::BorderUnfocused => &mut self.border_unfocused,
            ColorLabel::StatusBarFg => &mut self.status_bar_fg,
            ColorLabel::StatusBarBg => &mut self.status_bar_bg,
            ColorLabel::StatusErrorFg => &mut self.status_error_fg,
            ColorLabel::StatusErrorBg => &mut self.status_error_bg,
            ColorLabel::StatusInfoFg => &mut self.status_info_fg,
            ColorLabel::StatusInfoBg => &mut self.status_info_bg,
            ColorLabel::StatusWarningFg => &mut self.status_warning_fg,
            ColorLabel::StatusWarningBg => &mut self.status_warning_bg,
            ColorLabel::NavCursorFg => &mut self.nav_cursor_fg,
            ColorLabel::NavCursorBg => &mut self.nav_cursor_bg,
            ColorLabel::HoverBg => &mut self.hover_bg,
            ColorLabel::SidebarHighlightFg => &mut self.sidebar_highlight_fg,
            ColorLabel::SidebarHighlightBg => &mut self.sidebar_highlight_bg,
            ColorLabel::Dimmed => &mut self.dimmed,
            ColorLabel::CommandPickerFg => &mut self.command_picker_fg,
            ColorLabel::CommandPickerBg => &mut self.command_picker_bg,
            ColorLabel::StreamingIndicator => &mut self.streaming_indicator,
            ColorLabel::ApiUnavailable => &mut self.api_unavailable,
            ColorLabel::SummaryIndicator => &mut self.summary_indicator,
        };
        *slot = value;
    }

    pub fn any_set(&self) -> bool {
        ColorLabel::ALL.iter().any(|l| self.get(*l).is_some())
    }
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
        tracing::info!(result = "skipped", reason = "already_exists", path = %new_path.display(), "config.migrate");
        return;
    }

    let old_path = match old_config_path() {
        Some(p) if p.exists() => p,
        _ => {
            tracing::info!(result = "skipped", reason = "no_legacy_config", "config.migrate");
            return;
        }
    };

    if let Some(parent) = new_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("Warning: failed to create config directory: {e}");
        }
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
                    tracing::error!(phase = "parse", result = "error", path = %path.display(), elapsed_ms = parse_elapsed_ms, error = %e, "config.load");
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

    #[test]
    fn auth_default_is_none() {
        let auth = Auth::default();
        assert_eq!(auth, Auth::None);
        assert_eq!(auth.kind(), AuthKind::None);
    }

    #[test]
    fn auth_round_trips_through_toml_none() {
        let cfg = Config { auth: Auth::None, ..Config::default() };
        let s = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(back.auth, Auth::None);
    }

    #[test]
    fn auth_round_trips_through_toml_basic() {
        let cfg = Config {
            auth: Auth::Basic { username: "user".into(), password: "pw".into() },
            ..Config::default()
        };
        let s = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(back.auth, Auth::Basic { username: "user".into(), password: "pw".into() });
    }

    #[test]
    fn auth_round_trips_through_toml_bearer() {
        let cfg = Config {
            auth: Auth::Bearer { token: "sk-xyz".into() },
            ..Config::default()
        };
        let s = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(back.auth, Auth::Bearer { token: "sk-xyz".into() });
    }

    #[test]
    fn auth_round_trips_through_toml_header() {
        let cfg = Config {
            auth: Auth::Header { name: "X-Api-Key".into(), value: "abc".into() },
            ..Config::default()
        };
        let s = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(back.auth, Auth::Header { name: "X-Api-Key".into(), value: "abc".into() });
    }

    #[test]
    fn auth_round_trips_through_toml_query() {
        let cfg = Config {
            auth: Auth::Query { name: "api_key".into(), value: "abc".into() },
            ..Config::default()
        };
        let s = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(back.auth, Auth::Query { name: "api_key".into(), value: "abc".into() });
    }

    #[test]
    fn auth_defaults_when_missing_from_toml() {
        let toml_str = r#"
            api_url = "http://localhost:5001/v1"
        "#;
        let cfg: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.auth, Auth::None);
    }

    #[test]
    fn auth_display_labels() {
        assert_eq!(Auth::None.display_label(), "None");
        assert_eq!(Auth::Basic { username: String::new(), password: String::new() }.display_label(), "Basic");
        assert_eq!(Auth::Bearer { token: String::new() }.display_label(), "Bearer");
        assert_eq!(Auth::Header { name: String::new(), value: String::new() }.display_label(), "Header");
        assert_eq!(Auth::Query { name: String::new(), value: String::new() }.display_label(), "Query");
    }

    #[test]
    fn auth_validate_none_always_ok() {
        assert!(Auth::None.validate().is_ok());
    }

    #[test]
    fn auth_validate_basic_requires_both_fields() {
        assert!(Auth::Basic { username: String::new(), password: "p".into() }.validate().is_err());
        assert!(Auth::Basic { username: "u".into(), password: String::new() }.validate().is_err());
        assert!(Auth::Basic { username: "u".into(), password: "p".into() }.validate().is_ok());
    }

    #[test]
    fn auth_validate_bearer_requires_token() {
        assert!(Auth::Bearer { token: String::new() }.validate().is_err());
        assert!(Auth::Bearer { token: "t".into() }.validate().is_ok());
    }

    #[test]
    fn auth_validate_header_requires_both_fields_and_valid_header_name() {
        assert!(Auth::Header { name: String::new(), value: "v".into() }.validate().is_err());
        assert!(Auth::Header { name: "X-Key".into(), value: String::new() }.validate().is_err());
        assert!(Auth::Header { name: "X Key".into(), value: "v".into() }.validate().is_err(), "spaces are invalid header chars");
        assert!(Auth::Header { name: "X-Key".into(), value: "v".into() }.validate().is_ok());
    }

    #[test]
    fn auth_validate_query_requires_both_fields() {
        assert!(Auth::Query { name: String::new(), value: "v".into() }.validate().is_err());
        assert!(Auth::Query { name: "k".into(), value: String::new() }.validate().is_err());
        assert!(Auth::Query { name: "k".into(), value: "v".into() }.validate().is_ok());
    }

    #[test]
    fn auth_field_accessors_return_empty_when_variant_mismatches() {
        let b = Auth::Bearer { token: "t".into() };
        assert_eq!(b.basic_username(), "");
        assert_eq!(b.basic_password(), "");
        assert_eq!(b.bearer_token(), "t");
        assert_eq!(b.header_name(), "");
        assert_eq!(b.header_value(), "");
        assert_eq!(b.query_name(), "");
        assert_eq!(b.query_value(), "");
    }

    #[test]
    fn auth_field_accessors_for_basic() {
        let b = Auth::Basic { username: "u".into(), password: "p".into() };
        assert_eq!(b.basic_username(), "u");
        assert_eq!(b.basic_password(), "p");
    }

    #[test]
    fn auth_field_accessors_for_header_query() {
        let h = Auth::Header { name: "X".into(), value: "1".into() };
        assert_eq!(h.header_name(), "X");
        assert_eq!(h.header_value(), "1");
        let q = Auth::Query { name: "k".into(), value: "v".into() };
        assert_eq!(q.query_name(), "k");
        assert_eq!(q.query_value(), "v");
    }

    #[test]
    fn resolve_auth_uses_config_when_no_overrides() {
        let cfg = Config {
            auth: Auth::Bearer { token: "disk-token".into() },
            ..Config::default()
        };
        let overrides = AuthOverrides::default();
        assert_eq!(resolve_auth(&cfg, &overrides), Auth::Bearer { token: "disk-token".into() });
    }

    #[test]
    fn resolve_auth_cli_type_overrides_disk() {
        let cfg = Config {
            auth: Auth::Bearer { token: "disk-token".into() },
            ..Config::default()
        };
        let overrides = AuthOverrides {
            auth_type: Some(AuthKind::None),
            ..Default::default()
        };
        assert_eq!(resolve_auth(&cfg, &overrides), Auth::None);
    }

    #[test]
    fn resolve_auth_env_secret_overrides_disk_token() {
        let cfg = Config {
            auth: Auth::Bearer { token: "disk-token".into() },
            ..Config::default()
        };
        let overrides = AuthOverrides {
            auth_bearer_token: Some("env-token".into()),
            ..Default::default()
        };
        assert_eq!(resolve_auth(&cfg, &overrides), Auth::Bearer { token: "env-token".into() });
    }

    #[test]
    fn resolve_auth_cli_type_with_no_disk_match_empty_fields() {
        let cfg = Config {
            auth: Auth::None,
            ..Config::default()
        };
        let overrides = AuthOverrides {
            auth_type: Some(AuthKind::Basic),
            auth_basic_username: Some("u".into()),
            ..Default::default()
        };
        assert_eq!(
            resolve_auth(&cfg, &overrides),
            Auth::Basic { username: "u".into(), password: String::new() }
        );
    }

    #[test]
    fn resolve_auth_mixes_cli_env_and_disk() {
        let cfg = Config {
            auth: Auth::Header { name: "X-Disk".into(), value: "disk-val".into() },
            ..Config::default()
        };
        let overrides = AuthOverrides {
            auth_header_name: Some("X-Cli".into()),
            auth_header_value: Some("env-val".into()),
            ..Default::default()
        };
        assert_eq!(
            resolve_auth(&cfg, &overrides),
            Auth::Header { name: "X-Cli".into(), value: "env-val".into() }
        );
    }

    #[test]
    fn resolve_auth_none_variant_ignores_other_fields() {
        let cfg = Config::default();
        let overrides = AuthOverrides {
            auth_type: Some(AuthKind::None),
            auth_bearer_token: Some("ignored".into()),
            ..Default::default()
        };
        assert_eq!(resolve_auth(&cfg, &overrides), Auth::None);
    }
}

/// Serializes and atomically writes the config to `config.toml` in the data directory.
pub fn save(cfg: &Config) -> Result<()> {
    let path = config_path();
    let serialize_start = Instant::now();
    let toml_str = toml::to_string_pretty(cfg).context("failed to serialize config")?;
    let serialize_elapsed_ms = serialize_start.elapsed().as_secs_f64() * 1000.0;
    let path_str = path.display().to_string();
    tracing::info!(phase = "serialize", result = "ok", path = path_str.as_str(), bytes = toml_str.len(), elapsed_ms = serialize_elapsed_ms, "config.save");
    crate::timed_result!(
        tracing::Level::INFO,
        "config.save",
        phase = "write",
        path = path_str.as_str(),
        bytes = toml_str.len()
        ; {
            crate::crypto::write_atomic(&path, toml_str.as_bytes())
                .context(format!("failed to write config: {}", path.display()))
        }
    )
}
