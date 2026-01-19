use crate::api_client::ApiClient;
use anyhow::Result;

pub async fn execute(api_client: &ApiClient) -> Result<()> {
    api_client.github_disconnect().await?;
    println!("GitHub account disconnected");
    Ok(())
}
