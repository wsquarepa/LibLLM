use std::path::PathBuf;

use clap::Parser;

use crate::sampling::SamplingOverrides;

#[derive(Parser)]
#[command(name = "libllm", about = "CLI chat client for llama.cpp completions API")]
pub struct Args {
    /// Explicit session file path (plaintext JSON, bypasses encryption)
    #[arg(short = 's', long)]
    pub session: Option<PathBuf>,

    /// Passkey for session encryption (or set LIBLLM_PASSKEY env var)
    #[arg(long, env = "LIBLLM_PASSKEY", hide = true)]
    pub passkey: Option<String>,

    /// Disable session encryption
    #[arg(long)]
    pub no_encrypt: bool,

    /// Send a single message and exit (use "-" to read from stdin)
    #[arg(short = 'm', long)]
    pub message: Option<String>,

    /// System prompt
    #[arg(short = 'p', long)]
    pub system_prompt: Option<String>,

    /// API base URL (without /completions suffix)
    #[arg(long, env = "LIBLLM_API_URL")]
    pub api_url: Option<String>,

    /// Prompt template to use (llama2, chatml, mistral, phi, raw)
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

    /// Character card to use (name or path to .json/.png file)
    #[arg(short = 'c', long)]
    pub character: Option<String>,
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
}
