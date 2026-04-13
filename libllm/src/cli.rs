use std::path::PathBuf;

use clap::{Parser, Subcommand};

use libllm_core::sampling::SamplingOverrides;

#[derive(Subcommand)]
pub enum Command {
    /// Edit a character card or worldbook in $EDITOR
    Edit {
        /// Type of content to edit: "character" or "worldbook"
        kind: String,
        /// Name of the character or worldbook
        name: String,
    },
    /// Update libllm to the latest build
    Update {
        /// Target branch name (omit for stable)
        branch: Option<String>,
        /// List available branch builds
        #[arg(long)]
        list: bool,
        /// Skip downgrade confirmation
        #[arg(long, short = 'y')]
        yes: bool,
    },
}

pub struct CliOverrides {
    pub api_url: Option<String>,
    pub template: Option<String>,
    pub tls_skip_verify: bool,
    pub sampling: SamplingOverrides,
    pub system_prompt: Option<String>,
    pub persona: Option<String>,
}

#[derive(Parser)]
#[command(
    name = "libllm",
    about = "CLI chat client for llama.cpp completions API"
)]
pub struct Args {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Data directory path (initializes libllm structure at this path)
    #[arg(short = 'd', long)]
    pub data: Option<PathBuf>,

    /// Continue a previous session by UUID (use with -m and -d)
    #[arg(long = "continue")]
    pub continue_session: Option<String>,

    /// Passkey for session encryption (or set LIBLLM_PASSKEY env var, requires -d)
    #[arg(long, env = "LIBLLM_PASSKEY", hide_env_values = true)]
    pub passkey: Option<String>,

    /// Disable session encryption (requires -d)
    #[arg(long)]
    pub no_encrypt: bool,

    /// Send a single message and exit (use "-" to read from stdin)
    #[arg(short = 'm', long)]
    pub message: Option<String>,

    /// System prompt (overrides all other system prompt sources)
    #[arg(short = 'r', long)]
    pub system_prompt: Option<String>,

    /// User persona to use (requires -c)
    #[arg(short = 'p', long)]
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
    #[arg(short = 'c', long)]
    pub character: Option<String>,

    /// Skip TLS certificate verification for API connections
    #[arg(long)]
    pub tls_skip_verify: bool,

    /// Write debug log to this path instead of a temp file
    #[arg(long)]
    pub debug: Option<PathBuf>,

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
            persona: self.persona.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::Args;

    #[test]
    fn long_help_includes_passkey_flag() {
        let mut command = Args::command();
        let help = command.render_long_help().to_string();

        assert!(
            help.contains("--passkey"),
            "long help was missing --passkey: {help}"
        );
    }
}
