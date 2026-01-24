use crate::api_client::ApiClient;
use crate::config::ClientConfig;
use anyhow::{Context, Result};

pub async fn execute(
    api_client: &ApiClient,
    email: &str,
    password: &str,
    full_name: Option<&str>,
    organization_name: Option<&str>,
) -> Result<()> {
    // Call register API
    let auth_response = api_client
        .register(email, password, full_name, organization_name)
        .await?;

    // Load current config
    let mut config = ClientConfig::load()
        .context("Failed to load client config")?;

    // Save tokens
    config.access_token = Some(auth_response.tokens.access_token);
    config.refresh_token = Some(auth_response.tokens.refresh_token);
    config.save()
        .context("Failed to save client config")?;

    println!("Account created and logged in successfully!");
    println!("Email: {}", auth_response.user.email);
    if let Some(full_name) = &auth_response.user.full_name {
        println!("Name: {}", full_name);
    }
    if !auth_response.organizations.is_empty() {
        println!("Organization: {}", auth_response.organizations[0].organization.name);
    }

    Ok(())
}
