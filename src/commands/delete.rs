use anyhow::Result;
use sqlx::{Pool, Sqlite};
use tracing::instrument;

use crate::db;
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
    #[error("Podman error: {0}")]
    PodmanError(#[from] crate::podman::PodmanError),
}

type DeleteResult<T> = Result<T, DeleteError>;

/// Delete an app
#[instrument(skip(pool))]
pub async fn execute(pool: &Pool<Sqlite>, app_name: &str) -> DeleteResult<()> {
    // Get app
    let app = db::app::get_by_name(pool, app_name)
        .await?
        .ok_or_else(|| DeleteError::AppNotFound(app_name.to_string()))?;

    // Check if app is running
    if app.is_running() {
        return Err(DeleteError::AppRunning(app_name.to_string()));
    }

    todo!("Teardown");
}
