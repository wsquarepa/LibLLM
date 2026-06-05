#[tokio::main]
async fn main() -> anyhow::Result<()> {
    client::app::run().await
}
