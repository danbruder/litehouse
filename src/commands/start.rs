use bollard::Docker;
use sqlx::{Pool, Sqlite};
use tracing::{info, instrument};

use crate::caddy;
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

    // Get image tag for permission discovery
    let image_tag = build.image_tag.as_ref()
        .ok_or_else(|| StartError::AppBuildMissing(format!("Build {} has no image tag", build.id)))?;

    // Create app volume if it doesn't exist (idempotent)
    let volume_name = crate::volume::create_app_volume(docker, &app.id)
        .await
        .map_err(|e| StartError::AppStartFailed(format!("Failed to create volume: {}", e)))?;

    // Discover UID/GID from image
    let uid_gid = crate::volume::discover_image_user(docker, image_tag)
        .await
        .map_err(|e| StartError::AppStartFailed(format!("Failed to discover image user: {}", e)))?;

    // Initialize volume with correct permissions
    crate::volume::init_app_volume(docker, &app.id, &volume_name, uid_gid)
        .await
        .map_err(|e| StartError::AppStartFailed(format!("Failed to initialize volume: {}", e)))?;

    // Check if database needs restore
    crate::litestream::restore_if_needed(docker, pool, &app.id, &volume_name)
        .await
        .map_err(|e| StartError::AppStartFailed(format!("Failed to restore database: {}", e)))?;

    // Verify no other container is using this volume (single-writer guarantee)
    crate::volume::verify_volume_single_writer(docker, &app.id, &volume_name)
        .await
        .map_err(|e| StartError::AppStartFailed(format!("Volume already in use: {}", e)))?;

    // Verify image exists before trying to start
    match docker::image_exists(image_tag).await {
        Ok(false) => {
            return Err(StartError::AppBuildMissing(format!(
                "Docker image '{}' not found. The build record exists but the image is missing. \
                Run 'lh build {}' to rebuild.",
                image_tag, app_name
            )));
        }
        Err(e) => {
            tracing::warn!("Failed to check image existence: {}. Attempting to start anyway.", e);
        }
        Ok(true) => {
            // Image exists, proceed
        }
    }

    // Mount app volume at /data
    let volume_binds = vec![format!("{}:/data", volume_name)];

    tracing::info!("Mounting app data volume: {} -> /data", volume_name);

    // Start the app container
    tracing::info!("Running {} with image {}", &app.name, image_tag);
    docker::run(&app.name, image_tag, env_vars, volume_binds)
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

    // Sync Litestream configuration to include new app
    if let Err(e) = crate::litestream::sync_configuration(docker, pool).await {
        tracing::warn!(
            "Failed to sync Litestream configuration after starting app '{}': {}",
            app_name,
            e
        );
    }

    Ok(())
}
