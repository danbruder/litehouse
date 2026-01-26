use anyhow::Result;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, instrument};

const GITHUB_API_BASE: &str = "https://api.github.com";

#[derive(Debug, thiserror::Error)]
pub enum GitHubApiError {
    #[error("HTTP error: {0}")]
    HttpError(#[from] reqwest::Error),
    #[error("GitHub API error: {status} - {message}")]
    ApiError { status: u16, message: String },
    #[error("Invalid repository URL: {0}")]
    InvalidRepoUrl(String),
    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GitHubWebhook {
    pub id: i64,
    pub url: String,
    pub active: bool,
    pub events: Vec<String>,
    pub config: WebhookConfig,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WebhookConfig {
    pub url: String,
    pub content_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
    pub insecure_ssl: String,
}

#[derive(Debug, Serialize)]
struct CreateWebhookRequest {
    name: String,
    active: bool,
    events: Vec<String>,
    config: WebhookConfig,
}

/// Parse a GitHub repository URL into (owner, repo) tuple
///
/// Supports:
/// - https://github.com/owner/repo
/// - https://github.com/owner/repo.git
/// - git@github.com:owner/repo.git
#[instrument]
pub fn parse_repo_url(url: &str) -> Result<(String, String), GitHubApiError> {
    let url = url.trim();

    // Handle SSH URLs: git@github.com:owner/repo.git
    if url.starts_with("git@github.com:") {
        let path = url.strip_prefix("git@github.com:")
            .ok_or_else(|| GitHubApiError::InvalidRepoUrl(url.to_string()))?;
        let path = path.trim_end_matches(".git");
        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() != 2 {
            return Err(GitHubApiError::InvalidRepoUrl(url.to_string()));
        }
        return Ok((parts[0].to_string(), parts[1].to_string()));
    }

    // Handle HTTPS URLs: https://github.com/owner/repo or https://github.com/owner/repo.git
    if url.starts_with("https://github.com/") || url.starts_with("http://github.com/") {
        let path = url
            .trim_start_matches("https://github.com/")
            .trim_start_matches("http://github.com/");
        let path = path.trim_end_matches(".git");
        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() < 2 {
            return Err(GitHubApiError::InvalidRepoUrl(url.to_string()));
        }
        return Ok((parts[0].to_string(), parts[1].to_string()));
    }

    Err(GitHubApiError::InvalidRepoUrl(url.to_string()))
}

/// Create a webhook in a GitHub repository
#[instrument(skip(token, secret))]
pub async fn create_webhook(
    token: &str,
    owner: &str,
    repo: &str,
    webhook_url: &str,
    secret: &str,
    events: &[&str],
) -> Result<i64, GitHubApiError> {
    let client = Client::new();
    let url = format!("{}/repos/{}/{}/hooks", GITHUB_API_BASE, owner, repo);

    let request = CreateWebhookRequest {
        name: "web".to_string(),
        active: true,
        events: events.iter().map(|s| s.to_string()).collect(),
        config: WebhookConfig {
            url: webhook_url.to_string(),
            content_type: "json".to_string(),
            secret: Some(secret.to_string()),
            insecure_ssl: "0".to_string(),
        },
    };

    debug!("Creating webhook for {}/{} at {}", owner, repo, webhook_url);

    let response = client
        .post(&url)
        .header("Accept", "application/vnd.github+json")
        .header("Authorization", format!("Bearer {}", token))
        .header("User-Agent", format!("litehouse/{}", env!("CARGO_PKG_VERSION")))
        .header("X-GitHub-Api-Version", "2022-11-28")
        .json(&request)
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        let error_text = response.text().await.unwrap_or_default();
        return Err(GitHubApiError::ApiError {
            status: status.as_u16(),
            message: error_text,
        });
    }

    let webhook: GitHubWebhook = response.json().await?;
    debug!("Successfully created webhook with id: {}", webhook.id);

    Ok(webhook.id)
}

/// Delete a webhook from a GitHub repository
#[instrument(skip(token))]
pub async fn delete_webhook(
    token: &str,
    owner: &str,
    repo: &str,
    webhook_id: i64,
) -> Result<(), GitHubApiError> {
    let client = Client::new();
    let url = format!("{}/repos/{}/{}/hooks/{}", GITHUB_API_BASE, owner, repo, webhook_id);

    debug!("Deleting webhook {} from {}/{}", webhook_id, owner, repo);

    let response = client
        .delete(&url)
        .header("Accept", "application/vnd.github+json")
        .header("Authorization", format!("Bearer {}", token))
        .header("User-Agent", format!("litehouse/{}", env!("CARGO_PKG_VERSION")))
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() && status.as_u16() != 404 {
        // Ignore 404 - webhook already deleted
        let error_text = response.text().await.unwrap_or_default();
        return Err(GitHubApiError::ApiError {
            status: status.as_u16(),
            message: error_text,
        });
    }

    debug!("Successfully deleted webhook {}", webhook_id);
    Ok(())
}

/// List all webhooks for a GitHub repository
#[instrument(skip(token))]
pub async fn list_webhooks(
    token: &str,
    owner: &str,
    repo: &str,
) -> Result<Vec<GitHubWebhook>, GitHubApiError> {
    let client = Client::new();
    let url = format!("{}/repos/{}/{}/hooks", GITHUB_API_BASE, owner, repo);

    debug!("Listing webhooks for {}/{}", owner, repo);

    let response = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .header("Authorization", format!("Bearer {}", token))
        .header("User-Agent", format!("litehouse/{}", env!("CARGO_PKG_VERSION")))
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        let error_text = response.text().await.unwrap_or_default();
        return Err(GitHubApiError::ApiError {
            status: status.as_u16(),
            message: error_text,
        });
    }

    let webhooks: Vec<GitHubWebhook> = response.json().await?;
    debug!("Found {} webhooks", webhooks.len());

    Ok(webhooks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_repo_url_https() {
        let (owner, repo) = parse_repo_url("https://github.com/user/repo").unwrap();
        assert_eq!(owner, "user");
        assert_eq!(repo, "repo");
    }

    #[test]
    fn test_parse_repo_url_https_with_git() {
        let (owner, repo) = parse_repo_url("https://github.com/user/repo.git").unwrap();
        assert_eq!(owner, "user");
        assert_eq!(repo, "repo");
    }

    #[test]
    fn test_parse_repo_url_ssh() {
        let (owner, repo) = parse_repo_url("git@github.com:user/repo.git").unwrap();
        assert_eq!(owner, "user");
        assert_eq!(repo, "repo");
    }

    #[test]
    fn test_parse_repo_url_ssh_without_git() {
        let (owner, repo) = parse_repo_url("git@github.com:user/repo").unwrap();
        assert_eq!(owner, "user");
        assert_eq!(repo, "repo");
    }

    #[test]
    fn test_parse_repo_url_invalid() {
        assert!(parse_repo_url("not-a-url").is_err());
        assert!(parse_repo_url("https://example.com/repo").is_err());
    }
}
