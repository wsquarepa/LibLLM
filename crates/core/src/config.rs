//! Application configuration types with default resolution.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::sampling::SamplingOverrides;

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
#[derive(Default)]
pub enum Auth {
    #[default]
    None,
    Basic {
        username: String,
        password: String,
    },
    Bearer {
        token: String,
    },
    Header {
        name: String,
        value: String,
    },
    Query {
        name: String,
        value: String,
    },
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
}

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

const DEFAULT_SUMMARIZATION_PROMPT: &str = "Summarize the following conversation. Preserve key decisions, important details, character information, and narrative developments. Be concise but comprehensive.";

pub const MAX_SUMMARIZATION_CONTEXT_SIZE: usize = 131_072;
const DEFAULT_CONTEXT_SIZE: usize = MAX_SUMMARIZATION_CONTEXT_SIZE;

const DEFAULT_TRIGGER_PERCENT: u8 = 90;
const DEFAULT_KEEP_LAST: usize = 4;

const DEFAULT_BACKUP_ENABLED: bool = true;
const DEFAULT_BACKUP_KEEP_ALL_DAYS: u32 = 7;
const DEFAULT_BACKUP_KEEP_DAILY_DAYS: u32 = 30;
const DEFAULT_BACKUP_KEEP_WEEKLY_DAYS: u32 = 90;
const DEFAULT_BACKUP_REBASE_THRESHOLD_PERCENT: u32 = 50;
const DEFAULT_BACKUP_REBASE_HARD_CEILING: u32 = 10;

fn default_summarization_enabled() -> bool {
    true
}

fn default_context_size() -> usize {
    DEFAULT_CONTEXT_SIZE
}

fn default_trigger_percent() -> u8 {
    DEFAULT_TRIGGER_PERCENT
}

fn default_keep_last() -> usize {
    DEFAULT_KEEP_LAST
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
    #[serde(default = "default_trigger_percent")]
    pub trigger_percent: u8,
    /// Number of most-recent non-Summary messages preserved verbatim after a summary
    /// fires. Once the trigger fires, the summary collapses every older non-Summary
    /// message so subsequent turns do not re-summarize after just a message or two.
    #[serde(default = "default_keep_last")]
    pub keep_last: usize,
    #[serde(default = "default_summarization_prompt")]
    pub prompt: String,
}

impl Default for SummarizationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            api_url: None,
            context_size: DEFAULT_CONTEXT_SIZE,
            trigger_percent: DEFAULT_TRIGGER_PERCENT,
            keep_last: DEFAULT_KEEP_LAST,
            prompt: DEFAULT_SUMMARIZATION_PROMPT.to_owned(),
        }
    }
}

impl SummarizationConfig {
    /// Clamp `trigger_percent` into `[1, 100]` at read time. Emits a one-shot warn when
    /// the stored value is out of range. Called at each use site instead of at load to
    /// avoid the loader's current "return defaults on parse error" contract.
    pub fn effective_trigger_percent(&self) -> u8 {
        if !(1..=100).contains(&self.trigger_percent) {
            tracing::warn!(
                value = self.trigger_percent,
                "summarization.trigger_percent out of range [1, 100]; clamping",
            );
        }
        self.trigger_percent.clamp(1, 100)
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
    fn default_enabled() -> bool {
        DEFAULT_BACKUP_ENABLED
    }
    fn default_keep_all_days() -> u32 {
        DEFAULT_BACKUP_KEEP_ALL_DAYS
    }
    fn default_keep_daily_days() -> u32 {
        DEFAULT_BACKUP_KEEP_DAILY_DAYS
    }
    fn default_keep_weekly_days() -> u32 {
        DEFAULT_BACKUP_KEEP_WEEKLY_DAYS
    }
    fn default_rebase_threshold_percent() -> u32 {
        DEFAULT_BACKUP_REBASE_THRESHOLD_PERCENT
    }
    fn default_rebase_hard_ceiling() -> u32 {
        DEFAULT_BACKUP_REBASE_HARD_CEILING
    }
}

impl Default for BackupConfig {
    fn default() -> Self {
        Self {
            enabled: DEFAULT_BACKUP_ENABLED,
            keep_all_days: DEFAULT_BACKUP_KEEP_ALL_DAYS,
            keep_daily_days: DEFAULT_BACKUP_KEEP_DAILY_DAYS,
            keep_weekly_days: DEFAULT_BACKUP_KEEP_WEEKLY_DAYS,
            rebase_threshold_percent: DEFAULT_BACKUP_REBASE_THRESHOLD_PERCENT,
            rebase_hard_ceiling: DEFAULT_BACKUP_REBASE_HARD_CEILING,
        }
    }
}

const DEFAULT_FILES_ENABLED: bool = true;
const DEFAULT_FILES_PER_FILE_BYTES: usize = 524_288;
const DEFAULT_FILES_PER_MESSAGE_BYTES: usize = 4_194_304;

fn default_files_enabled() -> bool {
    DEFAULT_FILES_ENABLED
}

fn default_files_per_file_bytes() -> usize {
    DEFAULT_FILES_PER_FILE_BYTES
}

fn default_files_per_message_bytes() -> usize {
    DEFAULT_FILES_PER_MESSAGE_BYTES
}

const DEFAULT_FILES_SUMMARY_PROMPT: &str = "Summarize this file. Focus on its purpose, structure, and key facts useful for answering questions about its contents. Be concise.";

fn default_files_summarize_mode() -> FileSummarizeMode {
    FileSummarizeMode::Eager
}

fn default_files_summary_prompt() -> String {
    DEFAULT_FILES_SUMMARY_PROMPT.to_owned()
}

/// When the file-summary cache generates summaries.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FileSummarizeMode {
    Eager,
    Lazy,
}

/// File-ingestion size caps and feature toggle, nested under `[files]` in config.toml.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilesConfig {
    #[serde(default = "default_files_enabled")]
    pub enabled: bool,
    #[serde(default = "default_files_per_file_bytes")]
    pub per_file_bytes: usize,
    #[serde(default = "default_files_per_message_bytes")]
    pub per_message_bytes: usize,
    #[serde(default = "default_files_summarize_mode")]
    pub summarize_mode: FileSummarizeMode,
    #[serde(default = "default_files_summary_prompt")]
    pub summary_prompt: String,
}

impl Default for FilesConfig {
    fn default() -> Self {
        Self {
            enabled: DEFAULT_FILES_ENABLED,
            per_file_bytes: DEFAULT_FILES_PER_FILE_BYTES,
            per_message_bytes: DEFAULT_FILES_PER_MESSAGE_BYTES,
            summarize_mode: FileSummarizeMode::Eager,
            summary_prompt: DEFAULT_FILES_SUMMARY_PROMPT.to_owned(),
        }
    }
}

fn default_max_consecutive_turns() -> u32 {
    6
}

/// Group-chat orchestration settings, nested under `[group_chat]` in config.toml.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupChatConfig {
    #[serde(default = "default_max_consecutive_turns")]
    pub max_consecutive_turns: u32,
    /// Short system message injected just before the assistant turn opens, naming the
    /// active speaker. Mirrors SillyTavern's `group_nudge_prompt` (default
    /// `[Write the next reply only as {{char}}.]`). Empty string disables.
    /// `{{char}}` and `{{user}}` macros are substituted at prompt-build time.
    #[serde(default = "default_group_nudge_prompt")]
    pub nudge_prompt: String,
}

impl Default for GroupChatConfig {
    fn default() -> Self {
        Self {
            max_consecutive_turns: default_max_consecutive_turns(),
            nudge_prompt: default_group_nudge_prompt(),
        }
    }
}

fn default_group_nudge_prompt() -> String {
    "[Write the next reply only as {{char}}.]".to_owned()
}

impl GroupChatConfig {
    /// Clamp `max_consecutive_turns` into `[1, 50]` at read time. Emits a warn when
    /// the stored value is out of range. Called at each use site instead of at load to
    /// avoid the loader's current "return defaults on parse error" contract.
    pub fn effective_max_consecutive_turns(&self) -> u32 {
        if !(1..=50).contains(&self.max_consecutive_turns) {
            tracing::warn!(
                value = self.max_consecutive_turns,
                "group_chat.max_consecutive_turns out of range [1, 50]; clamping",
            );
        }
        self.max_consecutive_turns.clamp(1, 50)
    }
}

/// Optional color overrides for TUI theme elements, specified as CSS-style hex strings.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ThemeColorOverrides {
    pub user_character_fg: Option<String>,
    pub user_character_bg: Option<String>,
    pub side_character_fg: Option<String>,
    pub side_character_bg: Option<String>,
    pub file_reference_fg: Option<String>,
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
    pub token_band_ok: Option<String>,
    pub token_band_warn: Option<String>,
    pub token_band_over: Option<String>,
    pub group_character_fg_1: Option<String>,
    pub group_character_fg_2: Option<String>,
    pub group_character_fg_3: Option<String>,
    pub group_character_fg_4: Option<String>,
    pub group_character_fg_5: Option<String>,
    pub group_character_fg_6: Option<String>,
    pub group_character_fg_7: Option<String>,
    pub group_character_fg_8: Option<String>,
    pub group_character_bg_1: Option<String>,
    pub group_character_bg_2: Option<String>,
    pub group_character_bg_3: Option<String>,
    pub group_character_bg_4: Option<String>,
    pub group_character_bg_5: Option<String>,
    pub group_character_bg_6: Option<String>,
    pub group_character_bg_7: Option<String>,
    pub group_character_bg_8: Option<String>,
    pub missing_character_badge_fg: Option<String>,
    pub search_highlight_fg: Option<String>,
    pub search_highlight_bg: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorLabel {
    UserCharacterFg,
    UserCharacterBg,
    SideCharacterFg,
    SideCharacterBg,
    FileReferenceFg,
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
    TokenBandOk,
    TokenBandWarn,
    TokenBandOver,
    GroupCharacterFg1,
    GroupCharacterFg2,
    GroupCharacterFg3,
    GroupCharacterFg4,
    GroupCharacterFg5,
    GroupCharacterFg6,
    GroupCharacterFg7,
    GroupCharacterFg8,
    GroupCharacterBg1,
    GroupCharacterBg2,
    GroupCharacterBg3,
    GroupCharacterBg4,
    GroupCharacterBg5,
    GroupCharacterBg6,
    GroupCharacterBg7,
    GroupCharacterBg8,
    MissingCharacterBadgeFg,
    SearchHighlightFg,
    SearchHighlightBg,
}

impl ColorLabel {
    pub const ALL: [ColorLabel; 52] = [
        Self::UserCharacterFg,
        Self::UserCharacterBg,
        Self::SideCharacterFg,
        Self::SideCharacterBg,
        Self::FileReferenceFg,
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
        Self::TokenBandOk,
        Self::TokenBandWarn,
        Self::TokenBandOver,
        Self::GroupCharacterFg1,
        Self::GroupCharacterFg2,
        Self::GroupCharacterFg3,
        Self::GroupCharacterFg4,
        Self::GroupCharacterFg5,
        Self::GroupCharacterFg6,
        Self::GroupCharacterFg7,
        Self::GroupCharacterFg8,
        Self::GroupCharacterBg1,
        Self::GroupCharacterBg2,
        Self::GroupCharacterBg3,
        Self::GroupCharacterBg4,
        Self::GroupCharacterBg5,
        Self::GroupCharacterBg6,
        Self::GroupCharacterBg7,
        Self::GroupCharacterBg8,
        Self::MissingCharacterBadgeFg,
        Self::SearchHighlightFg,
        Self::SearchHighlightBg,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::UserCharacterFg => "user_character_fg",
            Self::UserCharacterBg => "user_character_bg",
            Self::SideCharacterFg => "side_character_fg",
            Self::SideCharacterBg => "side_character_bg",
            Self::FileReferenceFg => "file_reference_fg",
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
            Self::TokenBandOk => "token_band_ok",
            Self::TokenBandWarn => "token_band_warn",
            Self::TokenBandOver => "token_band_over",
            Self::GroupCharacterFg1 => "group_character_fg_1",
            Self::GroupCharacterFg2 => "group_character_fg_2",
            Self::GroupCharacterFg3 => "group_character_fg_3",
            Self::GroupCharacterFg4 => "group_character_fg_4",
            Self::GroupCharacterFg5 => "group_character_fg_5",
            Self::GroupCharacterFg6 => "group_character_fg_6",
            Self::GroupCharacterFg7 => "group_character_fg_7",
            Self::GroupCharacterFg8 => "group_character_fg_8",
            Self::GroupCharacterBg1 => "group_character_bg_1",
            Self::GroupCharacterBg2 => "group_character_bg_2",
            Self::GroupCharacterBg3 => "group_character_bg_3",
            Self::GroupCharacterBg4 => "group_character_bg_4",
            Self::GroupCharacterBg5 => "group_character_bg_5",
            Self::GroupCharacterBg6 => "group_character_bg_6",
            Self::GroupCharacterBg7 => "group_character_bg_7",
            Self::GroupCharacterBg8 => "group_character_bg_8",
            Self::MissingCharacterBadgeFg => "missing_character_badge_fg",
            Self::SearchHighlightFg => "search_highlight_fg",
            Self::SearchHighlightBg => "search_highlight_bg",
        }
    }

    pub fn from_name(label: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|l| l.name() == label)
    }
}

impl ThemeColorOverrides {
    pub fn get(&self, label: ColorLabel) -> Option<&str> {
        let slot = match label {
            ColorLabel::UserCharacterFg => &self.user_character_fg,
            ColorLabel::UserCharacterBg => &self.user_character_bg,
            ColorLabel::SideCharacterFg => &self.side_character_fg,
            ColorLabel::SideCharacterBg => &self.side_character_bg,
            ColorLabel::FileReferenceFg => &self.file_reference_fg,
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
            ColorLabel::TokenBandOk => &self.token_band_ok,
            ColorLabel::TokenBandWarn => &self.token_band_warn,
            ColorLabel::TokenBandOver => &self.token_band_over,
            ColorLabel::GroupCharacterFg1 => &self.group_character_fg_1,
            ColorLabel::GroupCharacterFg2 => &self.group_character_fg_2,
            ColorLabel::GroupCharacterFg3 => &self.group_character_fg_3,
            ColorLabel::GroupCharacterFg4 => &self.group_character_fg_4,
            ColorLabel::GroupCharacterFg5 => &self.group_character_fg_5,
            ColorLabel::GroupCharacterFg6 => &self.group_character_fg_6,
            ColorLabel::GroupCharacterFg7 => &self.group_character_fg_7,
            ColorLabel::GroupCharacterFg8 => &self.group_character_fg_8,
            ColorLabel::GroupCharacterBg1 => &self.group_character_bg_1,
            ColorLabel::GroupCharacterBg2 => &self.group_character_bg_2,
            ColorLabel::GroupCharacterBg3 => &self.group_character_bg_3,
            ColorLabel::GroupCharacterBg4 => &self.group_character_bg_4,
            ColorLabel::GroupCharacterBg5 => &self.group_character_bg_5,
            ColorLabel::GroupCharacterBg6 => &self.group_character_bg_6,
            ColorLabel::GroupCharacterBg7 => &self.group_character_bg_7,
            ColorLabel::GroupCharacterBg8 => &self.group_character_bg_8,
            ColorLabel::MissingCharacterBadgeFg => &self.missing_character_badge_fg,
            ColorLabel::SearchHighlightFg => &self.search_highlight_fg,
            ColorLabel::SearchHighlightBg => &self.search_highlight_bg,
        };
        slot.as_deref()
    }

    pub fn set(&mut self, label: ColorLabel, value: Option<String>) {
        let slot = match label {
            ColorLabel::UserCharacterFg => &mut self.user_character_fg,
            ColorLabel::UserCharacterBg => &mut self.user_character_bg,
            ColorLabel::SideCharacterFg => &mut self.side_character_fg,
            ColorLabel::SideCharacterBg => &mut self.side_character_bg,
            ColorLabel::FileReferenceFg => &mut self.file_reference_fg,
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
            ColorLabel::TokenBandOk => &mut self.token_band_ok,
            ColorLabel::TokenBandWarn => &mut self.token_band_warn,
            ColorLabel::TokenBandOver => &mut self.token_band_over,
            ColorLabel::GroupCharacterFg1 => &mut self.group_character_fg_1,
            ColorLabel::GroupCharacterFg2 => &mut self.group_character_fg_2,
            ColorLabel::GroupCharacterFg3 => &mut self.group_character_fg_3,
            ColorLabel::GroupCharacterFg4 => &mut self.group_character_fg_4,
            ColorLabel::GroupCharacterFg5 => &mut self.group_character_fg_5,
            ColorLabel::GroupCharacterFg6 => &mut self.group_character_fg_6,
            ColorLabel::GroupCharacterFg7 => &mut self.group_character_fg_7,
            ColorLabel::GroupCharacterFg8 => &mut self.group_character_fg_8,
            ColorLabel::GroupCharacterBg1 => &mut self.group_character_bg_1,
            ColorLabel::GroupCharacterBg2 => &mut self.group_character_bg_2,
            ColorLabel::GroupCharacterBg3 => &mut self.group_character_bg_3,
            ColorLabel::GroupCharacterBg4 => &mut self.group_character_bg_4,
            ColorLabel::GroupCharacterBg5 => &mut self.group_character_bg_5,
            ColorLabel::GroupCharacterBg6 => &mut self.group_character_bg_6,
            ColorLabel::GroupCharacterBg7 => &mut self.group_character_bg_7,
            ColorLabel::GroupCharacterBg8 => &mut self.group_character_bg_8,
            ColorLabel::MissingCharacterBadgeFg => &mut self.missing_character_badge_fg,
            ColorLabel::SearchHighlightFg => &mut self.search_highlight_fg,
            ColorLabel::SearchHighlightBg => &mut self.search_highlight_bg,
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
        let cfg = Config {
            auth: Auth::None,
            ..Config::default()
        };
        let s = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(back.auth, Auth::None);
    }

    #[test]
    fn auth_round_trips_through_toml_basic() {
        let cfg = Config {
            auth: Auth::Basic {
                username: "user".into(),
                password: "pw".into(),
            },
            ..Config::default()
        };
        let s = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(
            back.auth,
            Auth::Basic {
                username: "user".into(),
                password: "pw".into()
            }
        );
    }

    #[test]
    fn auth_round_trips_through_toml_bearer() {
        let cfg = Config {
            auth: Auth::Bearer {
                token: "sk-xyz".into(),
            },
            ..Config::default()
        };
        let s = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(
            back.auth,
            Auth::Bearer {
                token: "sk-xyz".into()
            }
        );
    }

    #[test]
    fn auth_round_trips_through_toml_header() {
        let cfg = Config {
            auth: Auth::Header {
                name: "X-Api-Key".into(),
                value: "abc".into(),
            },
            ..Config::default()
        };
        let s = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(
            back.auth,
            Auth::Header {
                name: "X-Api-Key".into(),
                value: "abc".into()
            }
        );
    }

    #[test]
    fn auth_round_trips_through_toml_query() {
        let cfg = Config {
            auth: Auth::Query {
                name: "api_key".into(),
                value: "abc".into(),
            },
            ..Config::default()
        };
        let s = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(
            back.auth,
            Auth::Query {
                name: "api_key".into(),
                value: "abc".into()
            }
        );
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
    fn auth_kind_display() {
        assert_eq!(AuthKind::None.to_string(), "None");
        assert_eq!(AuthKind::Basic.to_string(), "Basic");
        assert_eq!(AuthKind::Bearer.to_string(), "Bearer");
        assert_eq!(AuthKind::Header.to_string(), "Header");
        assert_eq!(AuthKind::Query.to_string(), "Query");
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
        let b = Auth::Basic {
            username: "u".into(),
            password: "p".into(),
        };
        assert_eq!(b.basic_username(), "u");
        assert_eq!(b.basic_password(), "p");
    }

    #[test]
    fn auth_field_accessors_for_header_query() {
        let h = Auth::Header {
            name: "X".into(),
            value: "1".into(),
        };
        assert_eq!(h.header_name(), "X");
        assert_eq!(h.header_value(), "1");
        let q = Auth::Query {
            name: "k".into(),
            value: "v".into(),
        };
        assert_eq!(q.query_name(), "k");
        assert_eq!(q.query_value(), "v");
    }

    #[test]
    fn resolve_auth_uses_config_when_no_overrides() {
        let cfg = Config {
            auth: Auth::Bearer {
                token: "disk-token".into(),
            },
            ..Config::default()
        };
        let overrides = AuthOverrides::default();
        assert_eq!(
            resolve_auth(&cfg, &overrides),
            Auth::Bearer {
                token: "disk-token".into()
            }
        );
    }

    #[test]
    fn resolve_auth_cli_type_overrides_disk() {
        let cfg = Config {
            auth: Auth::Bearer {
                token: "disk-token".into(),
            },
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
            auth: Auth::Bearer {
                token: "disk-token".into(),
            },
            ..Config::default()
        };
        let overrides = AuthOverrides {
            auth_bearer_token: Some("env-token".into()),
            ..Default::default()
        };
        assert_eq!(
            resolve_auth(&cfg, &overrides),
            Auth::Bearer {
                token: "env-token".into()
            }
        );
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
            Auth::Basic {
                username: "u".into(),
                password: String::new()
            }
        );
    }

    #[test]
    fn resolve_auth_mixes_cli_env_and_disk() {
        let cfg = Config {
            auth: Auth::Header {
                name: "X-Disk".into(),
                value: "disk-val".into(),
            },
            ..Config::default()
        };
        let overrides = AuthOverrides {
            auth_header_name: Some("X-Cli".into()),
            auth_header_value: Some("env-val".into()),
            ..Default::default()
        };
        assert_eq!(
            resolve_auth(&cfg, &overrides),
            Auth::Header {
                name: "X-Cli".into(),
                value: "env-val".into()
            }
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

    #[test]
    fn files_config_defaults() {
        let config = Config::default();
        assert!(config.files.enabled);
        assert_eq!(config.files.per_file_bytes, 524_288);
        assert_eq!(config.files.per_message_bytes, 4_194_304);
    }

    #[test]
    fn files_config_round_trips_toml() {
        let toml_text =
            "[files]\nenabled = false\nper_file_bytes = 1024\nper_message_bytes = 4096\n";
        let config: Config = toml::from_str(toml_text).expect("parse");
        assert!(!config.files.enabled);
        assert_eq!(config.files.per_file_bytes, 1024);
        assert_eq!(config.files.per_message_bytes, 4096);
    }

    #[test]
    fn file_reference_fg_in_color_label_all() {
        assert!(ColorLabel::ALL.contains(&ColorLabel::FileReferenceFg));
        assert_eq!(ColorLabel::FileReferenceFg.name(), "file_reference_fg");
        assert_eq!(
            ColorLabel::from_name("file_reference_fg"),
            Some(ColorLabel::FileReferenceFg),
        );
    }

    #[test]
    fn theme_color_overrides_file_reference_fg_round_trip() {
        let mut overrides = ThemeColorOverrides::default();
        overrides.set(ColorLabel::FileReferenceFg, Some("blue".to_owned()));
        assert_eq!(overrides.get(ColorLabel::FileReferenceFg), Some("blue"));
    }

    #[test]
    fn files_config_defaults_include_summarize_fields() {
        let config = Config::default();
        assert_eq!(
            config.files.summarize_mode,
            crate::config::FileSummarizeMode::Eager
        );
        assert!(config.files.summary_prompt.contains("Summarize this file"));
    }

    #[test]
    fn files_config_parses_summarize_fields() {
        let toml_text = r#"
[files]
enabled = true
per_file_bytes = 1024
per_message_bytes = 4096
summarize_mode = "lazy"
summary_prompt = "custom prompt"
"#;
        let config: Config = toml::from_str(toml_text).unwrap();
        assert_eq!(
            config.files.summarize_mode,
            crate::config::FileSummarizeMode::Lazy
        );
        assert_eq!(config.files.summary_prompt, "custom prompt");
    }

    #[test]
    fn files_config_missing_summarize_fields_use_defaults() {
        let toml_text =
            "[files]\nenabled = true\nper_file_bytes = 1024\nper_message_bytes = 4096\n";
        let config: Config = toml::from_str(toml_text).unwrap();
        assert_eq!(
            config.files.summarize_mode,
            crate::config::FileSummarizeMode::Eager
        );
        assert!(!config.files.summary_prompt.is_empty());
    }

    #[test]
    fn files_config_summarize_mode_only_keeps_other_defaults() {
        let toml_text = "[files]\nsummarize_mode = \"eager\"\n";
        let config: Config = toml::from_str(toml_text).unwrap();
        let defaults = crate::config::FilesConfig::default();
        assert_eq!(
            config.files.summarize_mode,
            crate::config::FileSummarizeMode::Eager
        );
        assert_eq!(config.files.enabled, defaults.enabled);
        assert_eq!(config.files.per_file_bytes, defaults.per_file_bytes);
        assert_eq!(config.files.per_message_bytes, defaults.per_message_bytes);
        assert_eq!(config.files.summary_prompt, defaults.summary_prompt);
    }

    #[test]
    fn effective_max_consecutive_turns_default_is_six() {
        let cfg = super::GroupChatConfig::default();
        assert_eq!(cfg.effective_max_consecutive_turns(), 6);
    }

    #[test]
    fn effective_max_consecutive_turns_clamps_zero_to_one() {
        let cfg = super::GroupChatConfig {
            max_consecutive_turns: 0,
            ..Default::default()
        };
        assert_eq!(cfg.effective_max_consecutive_turns(), 1);
    }

    #[test]
    fn effective_max_consecutive_turns_clamps_over_max() {
        let cfg = super::GroupChatConfig {
            max_consecutive_turns: 51,
            ..Default::default()
        };
        assert_eq!(cfg.effective_max_consecutive_turns(), 50);
        let cfg = super::GroupChatConfig {
            max_consecutive_turns: u32::MAX,
            ..Default::default()
        };
        assert_eq!(cfg.effective_max_consecutive_turns(), 50);
    }

    #[test]
    fn effective_max_consecutive_turns_in_range_passthrough() {
        for v in [1u32, 2, 6, 25, 50] {
            let cfg = super::GroupChatConfig {
                max_consecutive_turns: v,
                ..Default::default()
            };
            assert_eq!(cfg.effective_max_consecutive_turns(), v);
        }
    }

    #[test]
    fn group_chat_config_defaults_when_missing() {
        let toml_str = r#"api_url = "http://localhost:5001/v1""#;
        let cfg: super::Config = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.group_chat.max_consecutive_turns, 6);
    }

    #[test]
    fn group_chat_config_round_trips_through_toml() {
        let toml_str = "[group_chat]\nmax_consecutive_turns = 12\n";
        let cfg: super::Config = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.group_chat.max_consecutive_turns, 12);
    }
}

/// CLI flag values that override corresponding config fields; overridden fields display in red in `/config`.
#[derive(Default)]
pub struct CliOverrides {
    pub api_url: Option<String>,
    pub template: Option<String>,
    pub tls_skip_verify: bool,
    pub sampling: SamplingOverrides,
    pub system_prompt: Option<String>,
    pub persona: Option<String>,
    pub characters: Vec<String>,
    pub chat_mode: Option<crate::group_chat::ChatMode>,
    pub scenario: Option<String>,
    pub talkativeness: std::collections::HashMap<String, f32>,
    pub author_note: Option<String>,
    pub author_note_depth: Option<u32>,
    pub author_note_at_top: Option<bool>,
    pub no_summarize: bool,
    pub auth_type: Option<AuthKind>,
    pub auth_basic_username: Option<String>,
    pub auth_basic_password: Option<String>,
    pub auth_bearer_token: Option<String>,
    pub auth_header_name: Option<String>,
    pub auth_header_value: Option<String>,
    pub auth_query_name: Option<String>,
    pub auth_query_value: Option<String>,
}

impl CliOverrides {
    pub fn auth_overrides(&self) -> AuthOverrides {
        AuthOverrides {
            auth_type: self.auth_type,
            auth_basic_username: self.auth_basic_username.clone(),
            auth_basic_password: self.auth_basic_password.clone(),
            auth_bearer_token: self.auth_bearer_token.clone(),
            auth_header_name: self.auth_header_name.clone(),
            auth_header_value: self.auth_header_value.clone(),
            auth_query_name: self.auth_query_name.clone(),
            auth_query_value: self.auth_query_value.clone(),
        }
    }
}
