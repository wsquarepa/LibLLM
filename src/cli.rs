use std::path::PathBuf;

use clap::Parser;

#[derive(Parser)]
#[command(name = "libllm", about = "CLI chat client for llama.cpp completions API")]
pub struct Args {
    /// Session file for persisting conversation history
    #[arg(short = 's', long)]
    pub session: Option<PathBuf>,

    /// Send a single message and exit (use "-" to read from stdin)
    #[arg(short = 'm', long)]
    pub message: Option<String>,

    /// System prompt
    #[arg(short = 'p', long)]
    pub system_prompt: Option<String>,

    /// API base URL (without /completions suffix)
    #[arg(long, env = "LIBLLM_API_URL", default_value = "http://localhost:5001/v1")]
    pub api_url: String,
}
