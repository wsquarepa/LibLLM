//! Command-line argument parsing and CLI override definitions.

pub mod db;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use libllm::sampling::SamplingOverrides;

#[derive(Subcommand)]
pub enum RecoverCommand {
    /// List all backup points
    List,
    /// Verify backup chain integrity
    Verify {
        /// Run full content verification (slower)
        #[arg(long)]
        full: bool,
    },
    /// Restore database to a specific backup point
    Restore {
        /// Backup ID to restore to
        id: String,
        /// Skip confirmation prompt
        #[arg(long, short = 'y')]
        yes: bool,
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
    /// Direct database inspection and editing.
    #[command(alias = "database")]
    Db {
        #[command(subcommand)]
        command: DbSubcommand,
    },
}

/// CLI flag values that override the corresponding config fields; overridden fields display in red in `/config`.
pub struct CliOverrides {
    pub api_url: Option<String>,
    pub template: Option<String>,
    pub tls_skip_verify: bool,
    pub sampling: SamplingOverrides,
    pub system_prompt: Option<String>,
    pub persona: Option<String>,
    pub no_summarize: bool,
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
    #[arg(long, env = "LIBLLM_PASSKEY", hide_env_values = true, requires = "data")]
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

    /// Character card to use (name or path to .json/.png file, requires -p)
    #[arg(short = 'c', long, requires = "persona")]
    pub character: Option<String>,

    /// Skip TLS certificate verification for API connections
    #[arg(long)]
    pub tls_skip_verify: bool,

    /// Disable auto-summarization
    #[arg(long)]
    pub no_summarize: bool,

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
            persona: self
                .persona
                .as_deref()
                .map(libllm::character::slugify),
            no_summarize: self.no_summarize,
        }
    }
}
