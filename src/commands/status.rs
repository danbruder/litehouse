use anyhow::{anyhow, Result};
use tracing::instrument;

use crate::{db, docker, models::AppState};

/// Show app status
#[instrument]
pub async fn execute(app_name: Option<&str>) -> Result<()> {
    // Connect to database
    let pool = db::init_pool().await?;

    match app_name {
        Some(name) => {
            // Show status for a specific app
            let app = db::app::get_by_name(&pool, name)
                .await?
                .ok_or_else(|| anyhow!("App '{}' not found", name))?;

            // Read-through: reflect the live Docker container state rather
            // than the (possibly stale) cached DB column.
            let state = live_state(&app.name).await;

            println!("App: {}", app.name);
            println!("Status: {}", state);
        }
        None => {
            // Show status for all apps
            let apps = db::app::get_all(&pool).await?;

            if apps.is_empty() {
                println!("No apps found");
                return Ok(());
            }

            println!("{:<20} {:<10}", "NAME", "STATUS",);
            println!("{:<20} {:<10}", "----", "------",);

            for app in apps {
                let state = live_state(&app.name).await;
                println!("{:<20} {:<10}", app.name, state,);
            }
        }
    }

    Ok(())
}

/// Resolve an app's display state from the live Docker container, falling
/// back to the DB-cached desired state if Docker can't be reached.
async fn live_state(app_name: &str) -> AppState {
    match docker::live_state(app_name).await {
        Ok(state) => state,
        // Docker unreachable → fall back to the DB-cached desired state.
        Err(_) => {
            if let Ok(pool) = db::init_pool().await {
                if let Ok(Some(app)) = db::app::get_by_name(&pool, app_name).await {
                    return app.state;
                }
            }
            AppState::Stopped
        }
    }
}
