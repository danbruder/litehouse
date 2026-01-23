use bollard::Docker;
use sqlx::{Pool, Sqlite};
use tracing::{info, instrument};

use crate::caddy;
use crate::config;
use crate::db;
use crate::models::AppState;
use crate::docker;

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
    tracing::info!("Geting app {app_name}");
    let app = db::app::get_by_name(pool, app_name)
        .await?
        .ok_or_else(|| StartError::AppNotFound(app_name.to_string()))?;

    // If started
    if app.is_running() {
        tracing::info!("App {} is already running", app_name);
        return Ok(());
    }

    // Get latest build
    tracing::info!("Geting build for app id {}", app.id);
    let build = db::build::get_latest_by_app(pool, &app.id)
        .await?
        .ok_or_else(|| StartError::AppBuildMissing(app_name.to_string()))?;

    // Load environment variables
    tracing::info!("Loading environment variables for app {}", app.id);
    let env_vars = db::env_var::get_by_app(pool, &app.id)
        .await
        .map_err(|e| StartError::DatabaseError(e.to_string()))?;

    tracing::info!("Found {} environment variables", env_vars.len());

    // Prepare volume binds for SQLite database
    let data_dir = config::get_app_data_dir(&app.name)?;
    let volume_binds = vec![
        format!("{}:/app/data", data_dir.display())
    ];

    tracing::info!("Mounting app data directory: {} -> /app/data", data_dir.display());

    // Start the app with docker
    let image_tag = build.image_tag.as_ref()
        .ok_or_else(|| StartError::AppBuildMissing(format!("Build {} has no image tag", build.id)))?;
    tracing::info!("Running {} for {} on port {:?}", &app.name, image_tag, app.port);
    docker::run_with_port(&app.name, image_tag, app.port, env_vars, volume_binds)
        .await
        .map_err(|e| StartError::AppStartFailed(e.to_string()))?;

    // Update app state to Running
    let mut updated_app = app.clone();
    updated_app.state = AppState::Running;
    db::app::save(pool, &updated_app).await?;

    dbg!(&updated_app);

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
