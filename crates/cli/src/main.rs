#[tokio::main]
async fn main() -> anyhow::Result<()> {
    libllm_cli::app::run().await
}
