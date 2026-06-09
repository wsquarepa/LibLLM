//! Configuration sub-sections: summarization, backup, file-ingestion, and group-chat settings.

use serde::{Deserialize, Serialize};

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

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
        let cfg = GroupChatConfig::default();
        assert_eq!(cfg.effective_max_consecutive_turns(), 6);
    }

    #[test]
    fn effective_max_consecutive_turns_clamps_zero_to_one() {
        let cfg = GroupChatConfig {
            max_consecutive_turns: 0,
            ..Default::default()
        };
        assert_eq!(cfg.effective_max_consecutive_turns(), 1);
    }

    #[test]
    fn effective_max_consecutive_turns_clamps_over_max() {
        let cfg = GroupChatConfig {
            max_consecutive_turns: 51,
            ..Default::default()
        };
        assert_eq!(cfg.effective_max_consecutive_turns(), 50);
        let cfg = GroupChatConfig {
            max_consecutive_turns: u32::MAX,
            ..Default::default()
        };
        assert_eq!(cfg.effective_max_consecutive_turns(), 50);
    }

    #[test]
    fn effective_max_consecutive_turns_in_range_passthrough() {
        for v in [1u32, 2, 6, 25, 50] {
            let cfg = GroupChatConfig {
                max_consecutive_turns: v,
                ..Default::default()
            };
            assert_eq!(cfg.effective_max_consecutive_turns(), v);
        }
    }

    #[test]
    fn group_chat_config_defaults_when_missing() {
        let toml_str = r#"api_url = "http://localhost:5001/v1""#;
        let cfg: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.group_chat.max_consecutive_turns, 6);
    }

    #[test]
    fn group_chat_config_round_trips_through_toml() {
        let toml_str = "[group_chat]\nmax_consecutive_turns = 12\n";
        let cfg: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.group_chat.max_consecutive_turns, 12);
    }
}
