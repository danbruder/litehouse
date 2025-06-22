use anyhow::Result;
use tracing::instrument;

#[instrument]
pub async fn set_env(app_name: &str, key: &str, val: &str, delete: bool) -> Result<()> {
    /*
    // Connect to database
    let pool = db::init_pool().await?;

    // Get app
    let mut app = db::apps::get_by_name(&pool, app_name)
        .await?
        .ok_or_else(|| anyhow!("App '{}' not found", app_name))?;

    if delete {
        // Delete the environment variable
        app.environment.remove(key);
        info!("Deleted ENV var {} for app '{}'", key, app_name);
    } else {
        // Set the environment variable
        app.environment.insert(key.to_string(), val.to_string());
        info!("Set ENV var {} for app '{}'", key, app_name);
    }
    app.updated_at = Utc::now();

    // Save app to database
    db::apps::save(&pool, &app).await?;

    println!("Restart app '{}' to apply changes", app_name);

    Ok(())
    */
    panic!("todo");
}
