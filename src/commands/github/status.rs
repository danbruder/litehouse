use crate::api_client::ApiClient;
use anyhow::Result;

pub async fn execute(api_client: &ApiClient) -> Result<()> {
    let status = api_client.github_status().await?;

    if status.connected {
        println!("GitHub: Connected");
        if let Some(username) = status.username {
            println!("  Username: @{}", username);
        }
        if let Some(email) = status.email {
            println!("  Email: {}", email);
        }
        if let Some(scopes) = status.scopes {
            println!("  Scopes: {}", scopes);
        }
    } else {
        println!("GitHub: Not connected");
        println!("Run 'lh github connect' to connect your GitHub account");
    }

    Ok(())
}
