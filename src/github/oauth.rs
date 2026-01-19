use anyhow::Result;
use reqwest::Client;
use std::time::Duration;
use tokio::time::sleep;

use super::models::{DeviceAuthResponse, TokenResponse};

const DEVICE_AUTH_URL: &str = "https://github.com/login/device/code";
const TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const DEFAULT_SCOPES: &str = "repo read:user";

#[derive(Debug, thiserror::Error)]
pub enum OAuthError {
    #[error("Device authorization failed: {0}")]
    DeviceAuthFailed(String),
    #[error("Token request failed: {0}")]
    TokenRequestFailed(String),
    #[error("Authorization timed out")]
    AuthorizationTimeout,
    #[error("Authorization was denied by user")]
    AccessDenied,
    #[error("HTTP error: {0}")]
    HttpError(#[from] reqwest::Error),
}

/// Start the GitHub device authorization flow
pub async fn start_device_flow(client_id: &str) -> Result<DeviceAuthResponse, OAuthError> {
    let client = Client::new();

    let response = client
        .post(DEVICE_AUTH_URL)
        .header("Accept", "application/json")
        .form(&[
            ("client_id", client_id),
            ("scope", DEFAULT_SCOPES),
        ])
        .send()
        .await?;

    if !response.status().is_success() {
        let error_text = response.text().await.unwrap_or_default();
        return Err(OAuthError::DeviceAuthFailed(error_text));
    }

    let auth_response: DeviceAuthResponse = response.json().await?;
    Ok(auth_response)
}

/// Poll GitHub for the access token after user authorizes
pub async fn poll_for_token(
    client_id: &str,
    device_code: &str,
    interval: u64,
    expires_in: u64,
) -> Result<(String, String), OAuthError> {
    let client = Client::new();
    let poll_interval = Duration::from_secs(interval.max(5)); // GitHub recommends at least 5 seconds
    let timeout = Duration::from_secs(expires_in);
    let start = std::time::Instant::now();

    loop {
        if start.elapsed() > timeout {
            return Err(OAuthError::AuthorizationTimeout);
        }

        sleep(poll_interval).await;

        let response = client
            .post(TOKEN_URL)
            .header("Accept", "application/json")
            .form(&[
                ("client_id", client_id),
                ("device_code", device_code),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ])
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(OAuthError::TokenRequestFailed(error_text));
        }

        let token_response: TokenResponse = response.json().await?;

        match token_response {
            TokenResponse::Success {
                access_token,
                scope,
                ..
            } => {
                return Ok((access_token, scope));
            }
            TokenResponse::Pending { error, .. } => {
                match error.as_str() {
                    "authorization_pending" => {
                        // User hasn't authorized yet, keep polling
                        continue;
                    }
                    "slow_down" => {
                        // We're polling too fast, add 5 seconds
                        sleep(Duration::from_secs(5)).await;
                        continue;
                    }
                    "expired_token" => {
                        return Err(OAuthError::AuthorizationTimeout);
                    }
                    "access_denied" => {
                        return Err(OAuthError::AccessDenied);
                    }
                    _ => {
                        return Err(OAuthError::TokenRequestFailed(error));
                    }
                }
            }
        }
    }
}
