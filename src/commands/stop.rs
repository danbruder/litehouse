use anyhow::{Result, anyhow};
use bollard::Docker;
use sqlx::{Pool, Sqlite};
use tracing::{info, instrument};

use crate::caddy;
use crate::db;
use crate::models::AppState;
use crate::docker;

/// Stop an app using the supervisor
#[instrument(skip(pool, docker_conn))]
pub async fn execute(pool: &Pool<Sqlite>, docker_conn: &Docker, app_name: &str) -> Result<()> {
    // Get app
    let app = db::app::get_by_name(pool, app_name)
        .await?
        .ok_or_else(|| anyhow!("App '{}' not found", app_name))?;

    // Stop the app with docker
    info!("Stopping app '{}'", app_name);
    docker::stop(&app).await?;

    // Update app state to Stopped
    let mut updated_app = app.clone();
    updated_app.state = AppState::Stopped;
    db::app::save(pool, &updated_app).await?;

    println!("Successfully stopped app '{}'", app_name);

    // Sync Caddy configuration
    if let Err(e) = caddy::sync_configuration(docker_conn, pool).await {
        tracing::warn!(
            "Failed to sync Caddy configuration after stopping app '{}': {}",
            app_name,
            e
        );
        // Don't fail the stop operation if Caddy sync fails
    }

    Ok(())
}
