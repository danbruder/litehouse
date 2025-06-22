use anyhow::Result;
use sqlx::{Pool, Sqlite};
use tracing::{info, instrument};

use crate::db;
use crate::git;
use crate::models::{App, AppBuild};
use crate::podman;
use crate::providers::{Handle, Provider};

#[derive(Debug, thiserror::Error)]
pub enum DeleteError {
    #[error("App not found: {0}")]
    AppNotFound(String),
    #[error("Database error: {0}")]
    DatabaseError(#[from] crate::db::DatabaseError),
}

type BuildResult<T> = Result<T, DeleteError>;

// Build an app
#[instrument(skip(pool))]
pub async fn execute(pool: &Pool<Sqlite>, app_name: &str) -> BuildResult<()> {
    // Get app
    let app = db::apps::get_by_name(pool, app_name)
        .await?
        .ok_or_else(|| DeleteError::AppNotFound(app_name.to_string()))?;

    let git_result = git::pull(&app.remote, &app.branch, &app.directory).await?;

    let tag = format!("{}:{}", app.name, &git_result.commit);
    let result = podman::build(&app.directory, &tag).await?;

    // Update app with new image ID and last built time
    let build_config = AppBuild {
        image_id: Some(result.image_id),
        image_tag: tag,
        git_commit: git_result.commit,
    };

    let app = app.built(build_config);
    let updated_app = db::apps::save(pool, &app).await?;

    info!("Built app '{}'", &app.name);

    Ok(())
}
