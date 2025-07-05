use sqlx::{Pool, Sqlite};
use tracing::{info, instrument};

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
#[instrument(skip(pool))]
pub async fn execute(pool: &Pool<Sqlite>, app_name: &str) -> Result<()> {
    // VALIDATION
    let app = db::apps::get_by_name(pool, app_name)
        .await?
        .ok_or_else(|| StartError::AppNotFound(app_name.to_string()))?;

    // Get latest build
    let build = db::build::get_latest_by_app(pool, app.id)
        .await?
        .ok_or_else(|| StartError::AppNotFound(app_name.to_string()))?;

    // Start the app with podman
    podman::run(&app.name, &build.image_tag)
        .await
        .map_err(|e| StartError::AppStartFailed(e.to_string()))?;

    info!("Started app '{}'", app.name);

    Ok(())
}

#[cfg(test)]
mod test {
    use crate::models::App;

    #[tokio::test]
    async fn test_starting_non_existant_app() {
        todo!("Implement test for starting non-existant app");
    }

    #[tokio::test]
    async fn test_starting_not_deployed_app() {
        todo!("Implement test for starting non-existant app");
    }

    #[tokio::test]
    async fn test_start_happy_path() {
        todo!("Implement test for starting non-existant app");
    }
}
