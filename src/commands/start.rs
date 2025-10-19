use bollard::Docker;
use sqlx::{Pool, Sqlite};
use tracing::{info, instrument};

use crate::caddy;
use crate::db;
use crate::podman;

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum StartError {
    #[error("App not found: {0}")]
    AppNotFound(String),
    #[error("App already running: {0}")]
    AppAlreadyRunning(String),
    #[error("App not deployed: {0}")]
    AppNotDeployed(String),
    #[error("App Build missing: {0}")]
    AppBuildMissing(String),
    #[error("Failed to start app: {0}")]
    AppStartFailed(String),
    #[error("App log broken: {0}")]
    AppLogBroken(String),
    #[error("Invalid binary path: {0}")]
    InvalidBinaryPath(String),
    #[error("Database error")]
    DatabaseError(String),
    #[error("Config error: {0}")]
    ConfigError(#[from] crate::config::ConfigError),
}

type Result<T> = anyhow::Result<T, StartError>;

impl From<crate::db::DatabaseError> for StartError {
    fn from(err: crate::db::DatabaseError) -> Self {
        StartError::DatabaseError(err.to_string())
    }
}

/// Start an app using the supervisor
#[instrument(skip(pool, docker))]
pub async fn execute(pool: &Pool<Sqlite>, docker: &Docker, app_name: &str) -> Result<()> {
    // VALIDATION
    tracing::debug!("Geting app {app_name}");
    let app = db::app::get_by_name(pool, app_name)
        .await?
        .ok_or_else(|| StartError::AppNotFound(app_name.to_string()))?;

    // Get latest build
    tracing::debug!("Geting build for app id {}", app.id);
    let build = db::build::get_latest_by_app(pool, &app.id)
        .await?
        .ok_or_else(|| StartError::AppBuildMissing(app_name.to_string()))?;

    // Start the app with podman
    tracing::debug!("Running {} for {}", &app.name, &build.image_tag);
    podman::run(&app.name, &build.image_tag)
        .await
        .map_err(|e| StartError::AppStartFailed(e.to_string()))?;

    info!("Started app '{}'", app.name);

    // Sync Caddy configuration
    if let Err(e) = caddy::sync_configuration(docker, pool).await {
        tracing::warn!(
            "Failed to sync Caddy configuration after starting app '{}': {}",
            app_name,
            e
        );
        // Don't fail the start operation if Caddy sync fails
    }

    Ok(())
}
