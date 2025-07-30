use anyhow::{anyhow, Result};
use tracing::instrument;

use crate::db;

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

            println!("App: {}", app.name);
            println!("Status: {}", app.state);
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
                println!("{:<20} {:<10}", app.name, app.state,);
            }
        }
    }

    Ok(())
}
