use crate::api_client::ApiClient;
use crate::config::ClientConfig;
use anyhow::{Context, Result};

pub async fn execute(api_client: &ApiClient) -> Result<()> {
    // Check if we have tokens
    let config = ClientConfig::load()
        .context("Failed to load client config")?;

    if config.access_token.is_none() {
        println!("Not authenticated. Run 'lh auth login' to log in.");
        return Ok(());
    }

    // Try to get current user to verify token is valid
    match api_client.get_current_user().await {
        Ok(user) => {
            println!("Authenticated as {}", user.user.email);
            if let Some(full_name) = &user.user.full_name {
                println!("Name: {}", full_name);
            }
            if !user.organizations.is_empty() {
                println!("Organizations:");
                for org in &user.organizations {
                    println!("  - {} ({})", org.organization.name, org.role);
                }
            }
            Ok(())
        }
        Err(e) => {
            println!("Authentication token is invalid or expired.");
            println!("Error: {}", e);
            println!("Run 'lh auth login' to log in again.");
            Ok(())
        }
    }
}
