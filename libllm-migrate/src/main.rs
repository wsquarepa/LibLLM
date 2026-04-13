use anyhow::Result;
use clap::Parser;

#[derive(Parser)]
#[command(name = "libllm-migrate", about = "Migrate LibLLM data from file-based storage to SQLite database")]
struct Args {
    #[arg(short, long)]
    data: Option<std::path::PathBuf>,
    #[arg(long)]
    no_encrypt: bool,
    #[arg(long, env = "LIBLLM_PASSKEY")]
    passkey: Option<String>,
}

fn main() -> Result<()> {
    let _args = Args::parse();
    eprintln!("libllm-migrate: not yet implemented");
    std::process::exit(1);
}
