use crate::config;
use anyhow::Result;
use sqlx::{Pool, Sqlite};
use tracing::{info, instrument};

use crate::db;
use crate::git;
use crate::models::Build;
use crate::models::BuildInput;
use crate::podman;

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

// Build an app
#[instrument(skip(pool))]
pub async fn execute(pool: &Pool<Sqlite>, app_name: &str) -> BuildResult<()> {
    // Get app
    let app = db::app::get_by_name(pool, app_name)
        .await?
        .ok_or_else(|| BuildError::AppNotFound(app_name.to_string()))?;

    let remote = db::remote::get_by_app(pool, &app.id).await.map_err(|_| {
        BuildError::AppNotConfigured(format!(
            "Remote configuration for app '{}' not found",
            app_name
        ))
    })?;

    let build_dir = config::get_app_build_dir(&app.name)?;

    let git_result = git::pull(&remote, &build_dir).await?;

    let tag = format!("{}:{}", app.name, &git_result.commit);
    let image_id = podman::build(&build_dir.to_str().unwrap(), &tag)
        .await
        .map_err(|e| BuildError::AppNotFound(format!("Build failed: {}", e)))?;

    info!("Built image with tag: {} and ID: {}", tag, image_id);

    let build = Build::new(BuildInput {
        app_id: app.id,
        image_id: image_id,
        image_tag: tag,
        git_commit: git_result.commit,
    });
    db::build::save(pool, &build).await?;

    Ok(())
}
