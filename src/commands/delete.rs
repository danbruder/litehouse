use crate::caddy;
use crate::db;
use crate::docker;
use anyhow::Result;
use sqlx::{Pool, Sqlite};
use tracing::instrument;

#[derive(Debug, thiserror::Error)]
pub enum DeleteError {
    #[error("App not found: {0}")]
    AppNotFound(String),
    #[error("App is running: {0}")]
    AppRunning(String),
    #[error("App not built: {0}")]
    AppNotBuilt(String),
    #[error("Database error: {0}")]
    DatabaseError(#[from] crate::db::DatabaseError),
    #[error("Docker error: {0}")]
    DockerError(#[from] crate::docker::DockerError),
}

type DeleteResult<T> = Result<T, DeleteError>;

/// Delete an app
#[instrument(skip(pool))]
pub async fn execute(pool: &Pool<Sqlite>, app_name: &str) -> DeleteResult<()> {
    // Connect to Docker
    let docker_conn = docker::connect().await?;

    // Get app
    let app = db::app::get_by_name(pool, app_name)
        .await?
        .ok_or_else(|| DeleteError::AppNotFound(app_name.to_string()))?;

    // Check if app is running
    if app.is_running() {
        return Err(DeleteError::AppRunning(app_name.to_string()));
    }

    docker::stop(&app).await?;

    let build = db::build::get_latest_by_app(pool, &app.id).await?;
    if let Some(build) = build {
        if let Some(image_tag) = &build.image_tag {
            docker::remove(image_tag).await?;
        }
    }

    // Delete environment variables
    tracing::info!("Deleting environment variables for app {}", app.id);
    db::env_var::delete_by_app(pool, &app.id).await?;

    // Delete app
    db::app::delete_by_app_id(&pool, &app.id).await?;

    println!("Successfully stopped app '{}'", app_name);

    // Sync Caddy configuration
    if let Err(e) = caddy::sync_configuration(&docker_conn, &pool).await {
        tracing::warn!(
            "Failed to sync Caddy configuration after stopping app '{}': {}",
            app_name,
            e
        );
        // Don't fail the stop operation if Caddy sync fails
    }

    Ok(())
}
