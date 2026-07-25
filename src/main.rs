use litehouse::cli;
use tracing::info;
use tracing_subscriber::{fmt, EnvFilter};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging with clean output
    // Use RUST_LOG env var for verbose output (e.g., RUST_LOG=debug)
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    fmt()
        .with_env_filter(filter)
        .with_target(false)
        .without_time()
        .with_level(true)
        .with_writer(std::io::stderr)
        .compact()
        .init();

    info!("Starting Litehouse v{}", env!("CARGO_PKG_VERSION"));

    // Parse command line arguments and run the appropriate command
    cli::run().await?;

    Ok(())
}
