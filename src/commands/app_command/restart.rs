use anyhow::{anyhow, Result};
use sqlx::{Pool, Sqlite};
use tracing::{info, instrument};

use crate::db;
use crate::supervisor::SUPERVISOR;

/// Start an app using the supervisor
#[instrument]
pub async fn execute(pool: &Pool<Sqlite>, app_name: &str) -> Result<()> {
    // Get app
    let app = db::apps::get_by_name(&pool, app_name)
        .await?
        .ok_or_else(|| anyhow!("App '{}' not found", app_name))?;

    // Check if app has been deployed
    if app.binary_path.is_none() {
        return Err(anyhow!("App '{}' has not been deployed yet", app_name));
    }

    // Get the supervisor from global state
    let supervisor = SUPERVISOR
        .get()
        .ok_or_else(|| anyhow!("Process supervisor not initialized"))?;

    // Start the app through the supervisor
    info!("Sending restart request for app '{}'", app_name);
    supervisor.restart_app(app_name).await?;

    Ok(())
}
