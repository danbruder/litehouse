use litehouse::cli;
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    info!("Starting Litehouse v{}", env!("CARGO_PKG_VERSION"));

    // Parse command line arguments and run the appropriate command
    cli::run().await?;

    Ok(())
}
