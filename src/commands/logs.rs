use futures_util::StreamExt;
use std::pin::Pin;
use tracing::instrument;

use crate::db;
use crate::docker;

type Result<T> = std::result::Result<T, LogsError>;

#[derive(Debug, thiserror::Error)]
pub enum LogsError {
    #[error("App not found: {0}")]
    AppNotFound(String),
    #[error("Docker error: {0}")]
    DockerError(#[from] docker::DockerError),
    #[error("Database error: {0}")]
    DatabaseError(#[from] db::DatabaseError),
    #[error("Log stream error: {0}")]
    LogStreamError(String),
}

/// View app logs
#[instrument]
pub async fn execute(
    app_name: &str,
    lines: usize,
    follow: bool,
) -> Result<Pin<Box<dyn futures_util::Stream<Item = Result<String>> + Send>>> {
    // Connect to database
    let pool = db::init_pool().await?;

    // Check if app exists
    let _ = db::app::get_by_name(&pool, app_name)
        .await?
        .ok_or_else(|| LogsError::AppNotFound(app_name.to_string()))?;

    // Get logs from docker container and map errors to LogsError
    let stream = docker::logs_stream(app_name, lines, follow)
        .await
        .map_err(LogsError::DockerError)?;

    // Map stream items from Result<String, anyhow::Error> to Result<String, LogsError>
    let mapped_stream =
        stream.map(|item| item.map_err(|e| LogsError::LogStreamError(e.to_string())));

    Ok(Box::pin(mapped_stream))
}
