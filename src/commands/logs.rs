use anyhow::{anyhow, Result};
use std::pin::Pin;
use tracing::instrument;

use crate::db;
use crate::podman;

/// View app logs
#[instrument]
pub async fn execute(app_name: &str, lines: usize, follow: bool) ->  Result<Pin<Box<dyn futures_util::Stream<Item = Result<String, anyhow::Error>> + Send>>>{
    // Connect to database
    let pool = db::init_pool().await?;

    // Check if app exists
    let _ = db::app::get_by_name(&pool, app_name)
        .await?
        .ok_or_else(|| anyhow!("App '{}' not found", app_name))?;

    // Get logs from podman container
    podman::logs_stream(app_name, lines, follow).await
}
