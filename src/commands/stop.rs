use anyhow::{anyhow, Result};
use tracing::{info, instrument};

use crate::db;
use crate::models::AppState;
use crate::podman;

/// Stop an app using the supervisor
#[instrument]
pub async fn execute(app_name: &str) -> Result<()> {
    // Connect to database
    let pool = db::init_pool().await?;

    // Get app
    let app = db::app::get_by_name(&pool, app_name)
        .await?
        .ok_or_else(|| anyhow!("App '{}' not found", app_name))?;

    // Stop the app with podman
    info!("Stopping app '{}'", app_name);
    podman::stop(&app).await?;

    println!("Successfully stopped app '{}'", app_name);

    Ok(())
}
