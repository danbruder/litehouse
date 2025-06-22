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

    let (git_remote, git_branch, git_directory) =
        match (&app.git_remote, &app.git_branch, &app.git_directory) {
            (Some(remote), Some(branch), Some(directory)) => (remote, branch, directory),
            _ => {
                return Err(BuildError::AppNotConfigured(
                    "App's git configuration is not set up yet".to_string(),
                ))
            }
        };

    let git_result = git::pull(git_remote, git_branch, git_directory)
        .await
        .map_err(|e| BuildError::AppNotFound(format!("Git pull failed: {}", e)))?;

    let tag = format!("{}:{}", app.name, &git_result.commit);
    podman::build(git_directory, &tag)
        .await
        .map_err(|e| BuildError::AppNotFound(format!("Build failed: {}", e)))?;

    // Update app with new image tag and commit
    // For now, just log success - you may want to update the app record
    info!("Built image with tag: {}", tag);

    info!("Built app '{}'", &app.name);

    Ok(())
}
