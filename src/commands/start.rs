use bollard::Docker;
use sqlx::{Pool, Sqlite};
use tracing::{info, instrument};

use crate::backup::{BLOB_MOUNT_PATH, BLOB_PATH_ENV_VAR};
use crate::caddy;
use crate::db;
use crate::docker;
use crate::models::{App, AppState, EnvVar};

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

/// Give every app a stable place to write incrementally-backed-up blobs
/// (see `backup` module docs and `docs/superpowers/specs/2026-07-14-blob-backup-design.md`)
/// without hardcoding the path. Only appends the default if the app hasn't
/// explicitly set its own value for this key via `lh env set`.
fn ensure_blob_path_env_var(mut env_vars: Vec<EnvVar>, app_id: &str) -> Vec<EnvVar> {
    if !env_vars.iter().any(|e| e.key == BLOB_PATH_ENV_VAR) {
        env_vars.push(EnvVar::new(app_id, BLOB_PATH_ENV_VAR, BLOB_MOUNT_PATH));
    }
    env_vars
}

/// Create (or recreate) and start the container for `app` running `image_tag`,
/// including volume provisioning and environment variables. Does not touch
/// the app's database record or Caddy — callers are responsible for both,
/// since the desired sequencing differs between a plain `start` and a
/// deploy (which must replace an existing container outright).
///
/// Shared by [`execute`] and the deploy engine (`crate::deploy`) so the two
/// paths can't drift.
#[instrument(skip(pool, docker, app))]
pub async fn start_container(
    pool: &Pool<Sqlite>,
    docker: &Docker,
    app: &App,
    image_tag: &str,
) -> Result<()> {
    // Load environment variables
    tracing::info!("Loading environment variables for app {}", app.id);
    let env_vars = db::env_var::get_by_app(pool, &app.id)
        .await
        .map_err(|e| StartError::DatabaseError(e.to_string()))?;

    tracing::info!("Found {} environment variables", env_vars.len());
    let env_vars = ensure_blob_path_env_var(env_vars, &app.id);

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

    // Verify no other container is using this volume (single-writer guarantee)
    crate::volume::verify_volume_single_writer(docker, &app.id, &volume_name)
        .await
        .map_err(|e| StartError::AppStartFailed(format!("Volume already in use: {}", e)))?;

    // Verify image exists before trying to start
    match docker::image_exists(image_tag).await {
        Ok(false) => {
            return Err(StartError::AppBuildMissing(format!(
                "Docker image '{}' not found for app '{}'. Push to the app's GitHub repo (or run 'lh deploy') to produce a new image.",
                image_tag, app.name
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

    Ok(())
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

    // Get the last deployed image for this app
    tracing::info!("Getting deployed image for app id {}", app.id);
    let image_tag = app.image.clone().ok_or_else(|| {
        StartError::AppBuildMissing(format!(
            "App '{}' has no deployed image. Run 'lh deploy' first.",
            app_name
        ))
    })?;

    start_container(pool, docker, &app, &image_tag).await?;

    // Update app state to Running
    let mut updated_app = app.clone();
    updated_app.state = AppState::Running;
    db::app::save(pool, &updated_app).await?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::EnvVar;

    #[test]
    fn ensure_blob_path_env_var_adds_default_when_absent() {
        let env_vars = ensure_blob_path_env_var(vec![], "app-1");
        assert_eq!(env_vars.len(), 1);
        assert_eq!(env_vars[0].key, "LITEHOUSE_BLOB_PATH");
        assert_eq!(env_vars[0].value, "/data/blobs");
    }

    #[test]
    fn ensure_blob_path_env_var_respects_explicit_override() {
        let explicit = EnvVar::new("app-1", "LITEHOUSE_BLOB_PATH", "/data/custom-blobs");
        let env_vars = ensure_blob_path_env_var(vec![explicit], "app-1");
        assert_eq!(env_vars.len(), 1);
        assert_eq!(env_vars[0].value, "/data/custom-blobs");
    }

    #[test]
    fn ensure_blob_path_env_var_leaves_other_vars_untouched() {
        let other = EnvVar::new("app-1", "SOME_OTHER_KEY", "some-value");
        let env_vars = ensure_blob_path_env_var(vec![other], "app-1");
        assert_eq!(env_vars.len(), 2);
        assert!(env_vars.iter().any(|e| e.key == "SOME_OTHER_KEY"));
        assert!(env_vars.iter().any(|e| e.key == "LITEHOUSE_BLOB_PATH"));
    }
}
