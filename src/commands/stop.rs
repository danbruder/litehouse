use anyhow::{Result, anyhow};
use tracing::{info, instrument};

use crate::caddy;
use crate::db;
use crate::models::AppState;
use crate::podman;

/// Stop an app using the supervisor
#[instrument]
pub async fn execute(app_name: &str) -> Result<()> {
    // Connect to database
    let pool = db::init_pool().await?;

    // Connect to Docker
    let docker = podman::connect().await?;

    // Get app
    let app = db::app::get_by_name(&pool, app_name)
        .await?
        .ok_or_else(|| anyhow!("App '{}' not found", app_name))?;

    // Stop the app with podman
    info!("Stopping app '{}'", app_name);
    podman::stop(&app).await?;

    // Update app state to Stopped
    let mut updated_app = app.clone();
    updated_app.state = AppState::Stopped;
    db::app::save(&pool, &updated_app).await?;

    println!("Successfully stopped app '{}'", app_name);

    // Sync Caddy configuration
    if let Err(e) = caddy::sync_configuration(&docker, &pool).await {
        tracing::warn!(
            "Failed to sync Caddy configuration after stopping app '{}': {}",
            app_name,
            e
        );
        // Don't fail the stop operation if Caddy sync fails
    }

    Ok(())
}
