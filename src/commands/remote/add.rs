use anyhow::Result;
use sqlx::{Pool, Sqlite};
use tracing::{info, instrument, warn};

use crate::config;
use crate::db;
use crate::git;
use crate::github;
use crate::models::{Remote, WebhookConfig, WebhookStatus};

#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error("App not found: {0}")]
    AppNotFound(String),
    #[error("App git configuration not set up: {0}")]
    AppNotConfigured(String),
    #[error("Database error: {0}")]
    DatabaseError(#[from] crate::db::DatabaseError),
    #[error("Config error: {0}")]
    ConfigError(#[from] crate::config::ConfigError),
    #[error("Git error: {0}")]
    GitError(#[from] crate::git::GitError),
}

type BuildResult<T> = Result<T, BuildError>;

// Add a remote to an app and clone the repository
#[instrument(skip(pool, github_token))]
pub async fn execute(
    pool: &Pool<Sqlite>,
    app_name: &str,
    remote_url: &str,
    github_token: Option<&str>,
    user_id: Option<&str>,
    webhook_url: Option<&str>,
) -> BuildResult<()> {
    // Get app
    let app = db::app::get_by_name(pool, app_name)
        .await?
        .ok_or_else(|| BuildError::AppNotFound(app_name.to_string()))?;

    // Clone the remote (with token if provided for private repos)
    let build_dir = config::get_app_build_dir(&app.name)?;
    git::clone(remote_url, &build_dir, github_token).await?;

    let remote = Remote::new(&app.id, "github", remote_url, "main", ".");
    db::remote::save(pool, &remote).await?;

    // Create webhook configuration if user_id and webhook_url are provided
    if let (Some(user_id), Some(webhook_url)) = (user_id, webhook_url) {
        create_github_webhook(pool, &app.id, app_name, remote_url, user_id, webhook_url).await?;
    }

    Ok(())
}

/// Create a GitHub webhook for the app
async fn create_github_webhook(
    pool: &Pool<Sqlite>,
    app_id: &str,
    app_name: &str,
    remote_url: &str,
    user_id: &str,
    webhook_base_url: &str,
) -> BuildResult<()> {
    // Create webhook configuration
    let webhook_config = WebhookConfig::new(app_id);
    db::webhook::save_webhook_config(pool, &webhook_config).await?;

    // Get user's GitHub connection
    let github_conn = match db::github_connection::get_by_user_id(pool, user_id).await? {
        Some(conn) => conn,
        None => {
            db::webhook::update_webhook_status(
                pool,
                app_id,
                WebhookStatus::Failed,
                None,
                Some("No GitHub connection found. Run 'lh github connect' first.".to_string()),
            )
            .await?;
            warn!("No GitHub connection found for user {}", user_id);
            return Ok(()); // Continue without webhook
        }
    };

    // Parse repository URL
    let (owner, repo_name) = match github::parse_repo_url(remote_url) {
        Ok(parsed) => parsed,
        Err(e) => {
            db::webhook::update_webhook_status(
                pool,
                app_id,
                WebhookStatus::Failed,
                None,
                Some(format!("Failed to parse repository URL: {}", e)),
            )
            .await?;
            warn!("Failed to parse repository URL {}: {}", remote_url, e);
            return Ok(()); // Continue without webhook
        }
    };

    // Construct webhook URL
    let webhook_url = format!("{}/api/webhooks/github", webhook_base_url);

    // Create webhook in GitHub
    match github::create_webhook(
        &github_conn.access_token,
        &owner,
        &repo_name,
        &webhook_url,
        &webhook_config.secret,
        &["push"],
    )
    .await
    {
        Ok(webhook_id) => {
            db::webhook::update_webhook_status(
                pool,
                app_id,
                WebhookStatus::Active,
                Some(webhook_id),
                None,
            )
            .await?;
            info!(
                "Successfully created webhook for app '{}' (webhook_id: {})",
                app_name, webhook_id
            );
        }
        Err(e) => {
            db::webhook::update_webhook_status(
                pool,
                app_id,
                WebhookStatus::Failed,
                None,
                Some(format!("Failed to create webhook: {}", e)),
            )
            .await?;
            warn!("Failed to create webhook for app '{}': {}", app_name, e);
        }
    }

    Ok(())
}
