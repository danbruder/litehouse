use anyhow::Result;
use sqlx::{Pool, Sqlite};
use tracing::{info, instrument};

use crate::db;
use crate::git;
use crate::podman;

#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error("App not found: {0}")]
    AppNotFound(String),
    #[error("App git configuration not set up: {0}")]
    AppNotConfigured(String),
    #[error("Database error: {0}")]
    DatabaseError(#[from] crate::db::DatabaseError),
}

type BuildResult<T> = Result<T, BuildError>;

// Build an app
#[instrument(skip(pool))]
pub async fn execute(pool: &Pool<Sqlite>, app_name: &str) -> BuildResult<()> {
    // Get app
    let app = db::apps::get_by_name(pool, app_name)
        .await?
        .ok_or_else(|| BuildError::AppNotFound(app_name.to_string()))?;

    let remote = db::apps::get_remote_by_app_id(pool, &app.id)
        .await?
        .ok_or_else(|| {
            BuildError::AppNotConfigured(format!(
                "Remote configuration for app '{}' not found",
                app_name
            ))
        })?;

    let git_result = git::pull(remote)
        .await
        .map_err(|e| BuildError::AppNotFound(format!("Git pull failed: {}", e)))?;

    let tag = format!("{}:{}", app.name, &git_result.commit);
    podman::build(remote.git_directory, &tag)
        .await
        .map_err(|e| BuildError::AppNotFound(format!("Build failed: {}", e)))?;

    info!("Built image with tag: {}", tag);

    Ok(())
}
