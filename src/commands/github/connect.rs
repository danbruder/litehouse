use crate::api_client::ApiClient;
use anyhow::Result;

pub async fn execute(api_client: &ApiClient) -> Result<()> {
    println!("Starting GitHub device authorization flow...\n");

    // Start the device flow
    let device_response = api_client.github_connect_start().await?;

    // Display instructions
    println!("Visit: {}", device_response.verification_uri);
    println!("Enter code: {}\n", device_response.user_code);
    println!("Waiting for authorization...");

    // Poll for completion
    let connect_response = api_client
        .github_connect_poll(
            &device_response.device_code,
            device_response.interval,
            device_response.expires_in,
        )
        .await?;

    println!("\nConnected as @{}", connect_response.username);
    if let Some(email) = connect_response.email {
        println!("Email: {}", email);
    }

    Ok(())
}
