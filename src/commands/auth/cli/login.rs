use crate::api_client::ApiClient;
use crate::config::ClientConfig;
use anyhow::{Context, Result};

pub async fn execute(api_client: &ApiClient, email: &str, password: &str) -> Result<()> {
    // Call login API
    let auth_response = api_client.login(email, password).await?;

    // Load current config
    let mut config = ClientConfig::load()
        .context("Failed to load client config")?;

    // Save tokens
    config.access_token = Some(auth_response.tokens.access_token);
    config.refresh_token = Some(auth_response.tokens.refresh_token);
    config.save()
        .context("Failed to save client config")?;

    println!("Logged in successfully as {}", auth_response.user.email);
    if let Some(full_name) = &auth_response.user.full_name {
        println!("Welcome, {}!", full_name);
    }

    Ok(())
}
