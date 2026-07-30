use anyhow::Result;
use bollard::Docker;
use sqlx::{Pool, Sqlite};
use tracing::{info, instrument};

use crate::caddy;
use crate::db;
use crate::models::is_valid_health_check_path;

#[derive(Debug, thiserror::Error)]
pub enum HealthCheckError {
    #[error("App not found: {0}")]
    AppNotFound(String),
    #[error(
        "Invalid health check path: {0}. Paths must start with '/' and have no scheme, host, or spaces."
    )]
    InvalidPath(String),
    #[error("Database error: {0}")]
    DatabaseError(#[from] crate::db::DatabaseError),
}

type Result_<T> = Result<T, HealthCheckError>;

/// Set (or replace) `app_name`'s health check path and resync Caddy.
#[instrument(skip(pool, docker))]
pub async fn set(pool: &Pool<Sqlite>, docker: &Docker, app_name: &str, path: &str) -> Result_<()> {
    let path = path.trim();
    if !is_valid_health_check_path(path) {
        return Err(HealthCheckError::InvalidPath(path.to_string()));
    }

    let mut app = db::app::get_by_name(pool, app_name)
        .await?
        .ok_or_else(|| HealthCheckError::AppNotFound(app_name.to_string()))?;

    app.health_check_path = Some(path.to_string());

    db::app::save(pool, &app).await?;
    info!("Set health check path '{}' on app '{}'", path, app_name);

    if let Err(e) = caddy::sync_configuration(docker, pool).await {
        tracing::warn!(
            "Failed to sync Caddy configuration after setting health check path on app '{}': {}",
            app_name,
            e
        );
        // Don't fail the operation if Caddy sync fails -- the DB write
        // already succeeded and a later sync (e.g. next deploy) will pick
        // it up.
    }

    Ok(())
}

/// Clear `app_name`'s health check path and resync Caddy.
#[instrument(skip(pool, docker))]
pub async fn unset(pool: &Pool<Sqlite>, docker: &Docker, app_name: &str) -> Result_<()> {
    let mut app = db::app::get_by_name(pool, app_name)
        .await?
        .ok_or_else(|| HealthCheckError::AppNotFound(app_name.to_string()))?;

    app.health_check_path = None;

    db::app::save(pool, &app).await?;
    info!("Cleared health check path on app '{}'", app_name);

    if let Err(e) = caddy::sync_configuration(docker, pool).await {
        tracing::warn!(
            "Failed to sync Caddy configuration after clearing health check path on app '{}': {}",
            app_name,
            e
        );
    }

    Ok(())
}

/// Get `app_name`'s configured health check path, if any.
#[instrument(skip(pool))]
pub async fn get(pool: &Pool<Sqlite>, app_name: &str) -> Result_<Option<String>> {
    let app = db::app::get_by_name(pool, app_name)
        .await?
        .ok_or_else(|| HealthCheckError::AppNotFound(app_name.to_string()))?;

    Ok(app.health_check_path)
}
