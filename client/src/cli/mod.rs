//! Command-line argument parsing and CLI override definitions.

pub mod db;
pub mod search;

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{anyhow, bail, ensure, Context};
use clap::{Parser, Subcommand};
use libllm::sampling::SamplingOverrides;

/// Client-side wrapper around `libllm::config::AuthKind` for clap's `ValueEnum` parsing.
/// Keeps CLI-framework concerns out of the `libllm` crate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
#[clap(rename_all = "lowercase")]
pub enum AuthKindArg {
    None,
    Basic,
    Bearer,
    Header,
    Query,
}

impl From<AuthKindArg> for libllm::config::AuthKind {
    fn from(arg: AuthKindArg) -> Self {
        match arg {
            AuthKindArg::None => libllm::config::AuthKind::None,
            AuthKindArg::Basic => libllm::config::AuthKind::Basic,
            AuthKindArg::Bearer => libllm::config::AuthKind::Bearer,
            AuthKindArg::Header => libllm::config::AuthKind::Header,
            AuthKindArg::Query => libllm::config::AuthKind::Query,
        }
    }
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum ChatModeArg {
    #[value(name = "action-value")]
    ActionValue,
    #[value(name = "round-robin")]
    RoundRobin,
    #[value(name = "weighted-random")]
    WeightedRandom,
    #[value(name = "directed")]
    Directed,
}

impl From<ChatModeArg> for libllm::group_chat::ChatMode {
    fn from(v: ChatModeArg) -> Self {
        match v {
            ChatModeArg::ActionValue => Self::ActionValue,
            ChatModeArg::RoundRobin => Self::RoundRobin,
            ChatModeArg::WeightedRandom => Self::WeightedRandom,
            ChatModeArg::Directed => Self::Directed,
        }
    }
}

#[derive(Subcommand)]
pub enum RecoverCommand {
    /// List all backup points
    List,
    /// Verify backup chain integrity
    Verify {
        /// Run full content verification (slower)
        #[arg(long)]
        full: bool,
        #[arg(
            long = "archived-passkey",
            env = "LIBLLM_ARCHIVED_PASSKEY",
            hide_env_values = true
        )]
        archived_passkey: Option<String>,
    },
    /// Restore database to a specific backup point
    Restore {
        /// Backup ID to restore to
        id: String,
        /// Skip confirmation prompt
        #[arg(long, short = 'y')]
        yes: bool,
        #[arg(
            long = "archived-passkey",
            env = "LIBLLM_ARCHIVED_PASSKEY",
            hide_env_values = true,
            help = "Passkey that was active when an archived backup chain was created"
        )]
        archived_passkey: Option<String>,
    },
    /// Rebuild backup index from backup files on disk
    RebuildIndex,
}

#[derive(Subcommand)]
pub enum DbSubcommand {
    /// Execute a single SQL statement
    Sql {
        /// Allow mutating statements (INSERT/UPDATE/DELETE/etc.)
        #[arg(long)]
        write: bool,
        /// Output format
        #[arg(long, default_value = "table")]
        format: String,
        /// SQL statement to execute
        query: String,
    },
    /// Open an interactive SQL REPL
    Shell {
        /// Allow mutating statements within the session
        #[arg(long)]
        write: bool,
        /// Disable on-disk history for this session
        #[arg(long)]
        private: bool,
    },
    /// Write a fully decrypted SQLite database to <path>
    Dump {
        /// Skip overwrite confirmation if <path> already exists
        #[arg(long, short = 'y')]
        yes: bool,
        /// Output path
        path: std::path::PathBuf,
    },
    /// Replace the encrypted database with the contents of a plaintext SQLite file at <path>
    Import {
        /// Skip the confirmation prompt
        #[arg(long, short = 'y')]
        yes: bool,
        /// Plaintext SQLite file
        path: std::path::PathBuf,
    },
}

#[derive(Subcommand)]
pub enum Command {
    /// Edit a character card or worldbook in $EDITOR
    Edit {
        /// Type of content to edit: "character" or "worldbook"
        kind: String,
        /// Name of the character or worldbook
        name: String,
    },
    /// Import characters, worldbooks, personas, or system prompts from files
    Import {
        /// File(s) to import (.json, .png, or .txt)
        files: Vec<std::path::PathBuf>,
        /// Force content type: character, char, worldbook, wb, book, persona, prompt, system-prompt
        #[arg(long = "type", short = 't')]
        kind: Option<String>,
    },
    /// Manage database backups (list, verify, restore, rebuild-index).
    /// Without a subcommand, opens an interactive menu on a TTY or prints
    /// this help in non-interactive environments.
    Recover {
        #[command(subcommand)]
        command: Option<RecoverCommand>,
    },
    /// Update libllm to the latest build. Without a branch, opens a
    /// branch picker on a TTY or updates to stable non-interactively.
    Update {
        /// Target branch name (omit for stable / interactive picker)
        branch: Option<String>,
        /// Skip downgrade confirmation
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Search every stored message for a query string.
    Search {
        /// Query string (terms, "phrase", scope filters, m: raw mode)
        query: String,
        /// Maximum number of results to print
        #[arg(long, default_value_t = 200)]
        limit: usize,
        /// Emit a JSON array instead of human-readable text
        #[arg(long)]
        json: bool,
        /// Print full message content (with match highlighting) instead of snippet
        #[arg(long)]
        full: bool,
    },
    /// Direct database inspection and editing.
    #[command(alias = "database")]
    Db {
        #[command(subcommand)]
        command: DbSubcommand,
    },
}

/// CLI flag values that override the corresponding config fields; overridden fields display in red in `/config`.
#[derive(Default)]
pub struct CliOverrides {
    pub api_url: Option<String>,
    pub template: Option<String>,
    pub tls_skip_verify: bool,
    pub sampling: SamplingOverrides,
    pub system_prompt: Option<String>,
    pub persona: Option<String>,
    pub characters: Vec<String>,
    pub chat_mode: Option<libllm::group_chat::ChatMode>,
    pub scenario: Option<String>,
    pub talkativeness: std::collections::HashMap<String, f32>,
    pub author_note: Option<String>,
    pub author_note_depth: Option<u32>,
    pub author_note_at_top: Option<bool>,
    pub no_summarize: bool,
    pub auth_type: Option<libllm::config::AuthKind>,
    pub auth_basic_username: Option<String>,
    pub auth_basic_password: Option<String>,
    pub auth_bearer_token: Option<String>,
    pub auth_header_name: Option<String>,
    pub auth_header_value: Option<String>,
    pub auth_query_name: Option<String>,
    pub auth_query_value: Option<String>,
}

/// Parses a `"slug=value,slug=value"` talkativeness override string into a map.
///
/// Each entry must be `slug=f32`. The value is clamped to `[0.0, 1.0]`. An empty string
/// returns an empty map. Malformed entries (missing `=`, empty slug, empty value, or
/// non-numeric value) return an error.
pub fn parse_talkativeness(raw: &str) -> anyhow::Result<HashMap<String, f32>> {
    let mut out = HashMap::new();
    if raw.trim().is_empty() {
        return Ok(out);
    }
    for piece in raw.split(',') {
        let piece = piece.trim();
        if piece.is_empty() {
            continue;
        }
        let (slug, value) = piece
            .split_once('=')
            .ok_or_else(|| anyhow!("invalid --talkativeness entry: {piece}"))?;
        let slug = slug.trim();
        let value = value.trim();
        if slug.is_empty() {
            return Err(anyhow!("--talkativeness entry has empty slug: {piece}"));
        }
        if value.is_empty() {
            return Err(anyhow!("--talkativeness entry has empty value: {piece}"));
        }
        let parsed: f32 = value
            .parse()
            .with_context(|| format!("invalid talkativeness value: {value}"))?;
        out.insert(slug.to_owned(), parsed.clamp(0.0, 1.0));
    }
    Ok(out)
}

/// Validates that the group-chat arguments are self-consistent.
///
/// Checks: character count is within `MAX_GROUP_SIZE`, every talkativeness slug refers to a
/// character in the `-c` list, every character slug has a matching card in `card_names_by_slug`,
/// and no two characters share a display name (case-insensitive).
pub fn validate_group_chat_args(
    characters: &[String],
    talkativeness: &HashMap<String, f32>,
    card_names_by_slug: &HashMap<String, String>,
) -> anyhow::Result<()> {
    ensure!(
        characters.len() <= libllm::group_chat::MAX_GROUP_SIZE,
        "group chats are limited to {} characters",
        libllm::group_chat::MAX_GROUP_SIZE,
    );

    for slug in talkativeness.keys() {
        ensure!(
            characters.iter().any(|c| c == slug),
            "--talkativeness references slug '{slug}' not in -c",
        );
    }

    for slug in characters {
        ensure!(
            card_names_by_slug.contains_key(slug),
            "character card not found: {slug}",
        );
    }

    let mut seen: HashMap<String, String> = HashMap::new();
    for slug in characters {
        let name = card_names_by_slug.get(slug).expect("checked above");
        let key = name.to_lowercase();
        if let Some(other_slug) = seen.get(&key) {
            bail!(
                "two characters in this group share display name '{name}'; pick distinct cards or rename one (slugs: {other_slug}, {slug})",
            );
        }
        seen.insert(key, slug.clone());
    }
    Ok(())
}

#[derive(Parser)]
#[command(
    name = "libllm",
    about = "CLI chat client for llama.cpp completions API",
    disable_version_flag = true
)]
pub struct Args {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Print version and exit
    #[arg(short = 'V', long, action = clap::ArgAction::SetTrue)]
    pub version: bool,

    /// Data directory path (initializes libllm structure at this path)
    #[arg(short = 'd', long)]
    pub data: Option<PathBuf>,

    /// Continue a previous session by UUID (use with -m and -d)
    #[arg(long = "continue", requires = "data", requires = "message")]
    pub continue_session: Option<String>,

    /// Passkey for session encryption (or set LIBLLM_PASSKEY env var, requires -d)
    #[arg(
        long,
        env = "LIBLLM_PASSKEY",
        hide_env_values = true,
        requires = "data"
    )]
    pub passkey: Option<String>,

    /// Disable session encryption (requires -d)
    #[arg(long, requires = "data")]
    pub no_encrypt: bool,

    /// Send a single message and exit (use "-" to read from stdin)
    #[arg(short = 'm', long)]
    pub message: Option<String>,

    /// System prompt (overrides all other system prompt sources)
    #[arg(short = 'r', long)]
    pub system_prompt: Option<String>,

    /// User persona to use (requires -c)
    #[arg(short = 'p', long, requires = "character")]
    pub persona: Option<String>,

    /// Author's note text (overrides session-level note)
    #[arg(long)]
    pub note: Option<String>,

    /// Author's note depth — messages from end to inject at (requires --note)
    #[arg(long, requires = "note")]
    pub note_depth: Option<u32>,

    /// Pin author's note just below the system prompt (requires --note)
    #[arg(long, requires = "note")]
    pub note_top: bool,

    /// API base URL (without /completions suffix)
    #[arg(long, env = "LIBLLM_API_URL")]
    pub api_url: Option<String>,

    /// Instruct preset (e.g. "Mistral V3-Tekken", "Llama 3 Instruct", "ChatML", "Phi", "Alpaca", "Raw")
    #[arg(short = 't', long)]
    pub template: Option<String>,

    /// Sampling temperature
    #[arg(long)]
    pub temperature: Option<f64>,

    /// Top-K sampling
    #[arg(long)]
    pub top_k: Option<i64>,

    /// Top-P (nucleus) sampling
    #[arg(long)]
    pub top_p: Option<f64>,

    /// Min-P sampling
    #[arg(long)]
    pub min_p: Option<f64>,

    /// Repeat penalty window size
    #[arg(long)]
    pub repeat_last_n: Option<i64>,

    /// Repeat penalty strength
    #[arg(long)]
    pub repeat_penalty: Option<f64>,

    /// Maximum tokens to generate (-1 for unlimited)
    #[arg(long)]
    pub max_tokens: Option<i64>,

    /// Character cards to use (repeatable for group chats; requires -p)
    #[arg(short = 'c', long, requires = "persona", num_args = 1)]
    pub character: Vec<String>,

    /// Turn-order mode for group chats (>= 2 characters)
    #[arg(long, value_enum, default_value_t = ChatModeArg::ActionValue)]
    pub chat_mode: ChatModeArg,

    /// Per-character talkativeness override (e.g. "alice=0.7,bob=0.3")
    #[arg(long)]
    pub talkativeness: Option<String>,

    /// Override the session scenario (required to bypass the scenario gate when creating a group chat non-interactively)
    #[arg(long, value_name = "TEXT")]
    pub scenario: Option<String>,

    /// Skip TLS certificate verification for API connections
    #[arg(long)]
    pub tls_skip_verify: bool,

    /// Disable auto-summarization
    #[arg(long)]
    pub no_summarize: bool,

    /// Authentication type for API requests
    #[arg(long, value_enum)]
    pub auth_type: Option<AuthKindArg>,

    /// Username for Basic auth
    #[arg(long)]
    pub auth_basic_username: Option<String>,

    /// Header name for Header auth
    #[arg(long)]
    pub auth_header_name: Option<String>,

    /// Query parameter name for Query auth
    #[arg(long)]
    pub auth_query_name: Option<String>,

    /// Write debug log to this path instead of a temp file
    #[arg(long)]
    pub debug: Option<PathBuf>,

    /// EnvFilter directive for the debug log (e.g. "info,libllm::db=debug"). Requires --debug.
    #[arg(long, requires = "debug")]
    pub log_filter: Option<String>,

    /// Write a timings report to ./timings.log or an optional custom path
    #[arg(long, num_args = 0..=1, default_missing_value = "./timings.log")]
    pub timings: Option<PathBuf>,

    /// Remove LibLLM temporary debug logs and exit
    #[arg(long)]
    pub cleanup: bool,

    /// Internal: trigger Destroy All Data flow non-interactively. Used only by tests.
    #[cfg(debug_assertions)]
    #[arg(long, hide = true)]
    pub debug_trigger_destroy_all: bool,
}

impl Args {
    pub fn sampling_overrides(&self) -> SamplingOverrides {
        SamplingOverrides {
            temperature: self.temperature,
            top_k: self.top_k,
            top_p: self.top_p,
            min_p: self.min_p,
            repeat_last_n: self.repeat_last_n,
            repeat_penalty: self.repeat_penalty,
            max_tokens: self.max_tokens,
        }
    }

    pub fn cli_overrides(&self) -> CliOverrides {
        CliOverrides {
            api_url: self.api_url.clone(),
            template: self.template.clone(),
            tls_skip_verify: self.tls_skip_verify,
            sampling: self.sampling_overrides(),
            system_prompt: self.system_prompt.clone(),
            persona: self.persona.as_deref().map(libllm::character::slugify),
            characters: self.character.clone(),
            chat_mode: if self.character.len() >= 2 {
                Some(self.chat_mode.into())
            } else {
                None
            },
            scenario: self.scenario.clone(),
            talkativeness: self
                .talkativeness
                .as_deref()
                .and_then(|s| parse_talkativeness(s).ok())
                .unwrap_or_default(),
            author_note: self.note.clone(),
            author_note_depth: self.note_depth,
            author_note_at_top: self.note_top.then_some(true),
            no_summarize: self.no_summarize,
            auth_type: self.auth_type.map(Into::into),
            auth_basic_username: self.auth_basic_username.clone(),
            auth_basic_password: std::env::var("LIBLLM_AUTH_BASIC_PASSWORD").ok(),
            auth_bearer_token: std::env::var("LIBLLM_AUTH_BEARER_TOKEN").ok(),
            auth_header_name: self.auth_header_name.clone(),
            auth_header_value: std::env::var("LIBLLM_AUTH_HEADER_VALUE").ok(),
            auth_query_name: self.auth_query_name.clone(),
            auth_query_value: std::env::var("LIBLLM_AUTH_QUERY_VALUE").ok(),
        }
    }
}

impl CliOverrides {
    pub fn auth_overrides(&self) -> libllm::config::AuthOverrides {
        libllm::config::AuthOverrides {
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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn names(items: &[(&str, &str)]) -> HashMap<String, String> {
        items
            .iter()
            .map(|(s, n)| ((*s).to_owned(), (*n).to_owned()))
            .collect()
    }

    #[test]
    fn parse_talkativeness_overrides_simple() {
        let m = parse_talkativeness("alice=0.3,bob=0.8").unwrap();
        assert_eq!(m.get("alice"), Some(&0.3));
        assert_eq!(m.get("bob"), Some(&0.8));
    }

    #[test]
    fn parse_talkativeness_clamps_out_of_range() {
        let m = parse_talkativeness("alice=2.0,bob=-1.0").unwrap();
        assert_eq!(m.get("alice"), Some(&1.0));
        assert_eq!(m.get("bob"), Some(&0.0));
    }

    #[test]
    fn parse_talkativeness_rejects_malformed() {
        assert!(parse_talkativeness("alice").is_err());
        assert!(parse_talkativeness("alice=").is_err());
        assert!(parse_talkativeness("=0.5").is_err());
        assert!(parse_talkativeness("alice=abc").is_err());
    }

    #[test]
    fn parse_talkativeness_empty_returns_empty_map() {
        let m = parse_talkativeness("").unwrap();
        assert!(m.is_empty());
    }

    #[test]
    fn validate_rejects_over_cap() {
        let chars: Vec<String> = (0..9).map(|i| format!("c{i}")).collect();
        let card_names = chars
            .iter()
            .enumerate()
            .map(|(i, s)| (s.clone(), format!("N{i}")))
            .collect();
        let err = validate_group_chat_args(&chars, &Default::default(), &card_names).unwrap_err();
        assert!(err.to_string().contains("limited to"));
    }

    #[test]
    fn validate_rejects_unknown_talkativeness_slug() {
        let chars = vec!["alice".to_owned()];
        let card_names = names(&[("alice", "Alice")]);
        let mut talk = HashMap::new();
        talk.insert("ghost".to_owned(), 0.5);
        let err = validate_group_chat_args(&chars, &talk, &card_names).unwrap_err();
        assert!(err.to_string().contains("ghost"));
    }

    #[test]
    fn validate_rejects_missing_card() {
        let chars = vec!["alice".to_owned(), "bob".to_owned()];
        let card_names = names(&[("alice", "Alice")]);
        let err =
            validate_group_chat_args(&chars, &Default::default(), &card_names).unwrap_err();
        assert!(err.to_string().contains("bob"));
    }

    #[test]
    fn validate_rejects_duplicate_display_names_case_insensitive() {
        let chars = vec!["alice1".to_owned(), "alice2".to_owned()];
        let card_names = names(&[("alice1", "Alice"), ("alice2", "alice")]);
        let err =
            validate_group_chat_args(&chars, &Default::default(), &card_names).unwrap_err();
        assert!(err.to_string().contains("share display name"));
    }

    #[test]
    fn validate_accepts_valid_group() {
        let chars = vec!["alice".to_owned(), "bob".to_owned()];
        let card_names = names(&[("alice", "Alice"), ("bob", "Bob")]);
        let mut talk = HashMap::new();
        talk.insert("alice".to_owned(), 0.3);
        validate_group_chat_args(&chars, &talk, &card_names).unwrap();
    }

    #[test]
    fn single_character_parses_into_vec_of_one() {
        let args = Args::try_parse_from(["libllm", "-c", "alice", "-p", "me"]).unwrap();
        assert_eq!(args.character, vec!["alice".to_owned()]);
    }

    #[test]
    fn multiple_characters_parse_in_order() {
        let args = Args::try_parse_from(["libllm", "-c", "alice", "-c", "bob", "-c", "charlie", "-p", "me"]).unwrap();
        assert_eq!(args.character, vec!["alice", "bob", "charlie"]);
    }

    #[test]
    fn group_without_persona_is_rejected() {
        let result = Args::try_parse_from(["libllm", "-c", "alice", "-c", "bob"]);
        assert!(result.is_err());
    }

    #[test]
    fn chat_mode_default_action_value() {
        let args = Args::try_parse_from(["libllm", "-c", "alice", "-c", "bob", "-p", "me"]).unwrap();
        assert!(matches!(args.chat_mode, ChatModeArg::ActionValue));
    }

    #[test]
    fn chat_mode_weighted_random_parses() {
        let args = Args::try_parse_from(["libllm", "-c", "alice", "-c", "bob", "-p", "me", "--chat-mode", "weighted-random"]).unwrap();
        assert!(matches!(args.chat_mode, ChatModeArg::WeightedRandom));
    }

    #[test]
    fn chat_mode_directed_parses() {
        let args = Args::try_parse_from(["libllm", "-c", "alice", "-c", "bob", "-p", "me", "--chat-mode", "directed"]).unwrap();
        assert!(matches!(args.chat_mode, ChatModeArg::Directed));
    }
}
