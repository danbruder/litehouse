use crate::api_client::ApiClient;
use crate::config::ClientConfig;
use anyhow::{Context, Result};

pub async fn execute(api_client: &ApiClient) -> Result<()> {
    // Try to logout on server (may fail if not authenticated, that's ok)
    let _ = api_client.logout().await;

    // Load current config
    let mut config = ClientConfig::load()
        .context("Failed to load client config")?;

    // Clear tokens
    config.access_token = None;
    config.refresh_token = None;
    config.save()
        .context("Failed to save client config")?;

    println!("Logged out successfully");

    Ok(())
}
