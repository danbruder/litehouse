use anyhow::Result as AnyhowResult;
use sqlx::{Pool, Sqlite};
use tracing::{info, instrument};

use crate::db;
use crate::models::EnvVar;

#[derive(Debug, thiserror::Error)]
pub enum AppEnvError {
    #[error("App not found: {0}")]
    AppNotFound(String),
    #[error("Database error: {0}")]
    DatabaseError(#[from] crate::db::DatabaseError),
}

type Result<T> = AnyhowResult<T, AppEnvError>;

#[instrument(skip(pool))]
pub async fn set_env(
    pool: &Pool<Sqlite>,
    app_name: &str,
    key: &str,
    val: &str,
    delete: bool,
) -> Result<()> {
    // Get app
    let app = db::app::get_by_name(pool, app_name)
        .await?
        .ok_or_else(|| AppEnvError::AppNotFound(app_name.to_string()))?;

    if delete {
        // Delete environment variable
        db::env_var::delete_by_key(pool, &app.id, key).await?;
        info!("Deleted ENV var '{}' for app '{}'", key, app_name);
    } else {
        // Create and save environment variable
        let env_var = EnvVar::new(&app.id, key, val);
        db::env_var::save(pool, &env_var).await?;
        info!("Set ENV var '{}' for app '{}'", key, app_name);
    }

    Ok(())
}
