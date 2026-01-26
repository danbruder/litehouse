use sqlx::{Pool, Sqlite};
use tracing::{info, instrument};

use crate::config;
use crate::db;
use crate::docker;
use crate::models::App;

#[derive(Debug, thiserror::Error)]
pub enum AppCreateError {
    #[error("App already exists: {0}")]
    AppAlreadyExists(String),
    #[error("Failed to create app: {0}")]
    AppError(#[from] crate::models::AppError),
    #[error("Failed to create app: {0}")]
    DatabaseError(#[from] crate::db::DatabaseError),
    #[error("Config error: {0}")]
    ConfigError(#[from] crate::config::ConfigError),
    #[error("Docker error: {0}")]
    DockerError(String),
}

pub type Result<T> = std::result::Result<T, AppCreateError>;

/// Create a new app
#[instrument(skip(pool))]
pub async fn execute(pool: &Pool<Sqlite>, app_name: &str) -> Result<()> {
    if let Some(_) = db::app::get_by_name(pool, app_name).await? {
        return Err(AppCreateError::AppAlreadyExists(app_name.to_string()));
    }

    // Create app
    let app = App::new(app_name)?;
    db::app::save(pool, &app).await?;

    // Initialize default environment variables
    db::env_var::init_default_env_vars(pool, &app.id, &app.name)
        .await
        .map_err(|e| AppCreateError::DatabaseError(e.into()))?;

    info!("Initialized default environment variables for app '{}'", app.name);

    // Initialize SQLite database for the app
    config::init_app_database(app_name)?;

    // Create dedicated volume for app database
    let docker = docker::connect().await
        .map_err(|e| AppCreateError::DockerError(e.to_string()))?;
    crate::volume::create_app_volume(&docker, &app.id).await
        .map_err(|e| AppCreateError::DockerError(format!("Failed to create app volume: {}", e)))?;

    // Initialize empty database in volume
    // Note: We don't have the image yet, so we can't discover user permissions
    // The database will be created with permissive settings, then corrected when the app first starts
    crate::volume::init_app_database_in_volume(&docker, &app.id, &crate::volume::get_app_volume_name(&app.id), None)
        .await
        .map_err(|e| AppCreateError::DockerError(format!("Failed to initialize database in volume: {}", e)))?;

    info!("Initialized SQLite database in volume for app '{}'", app.name);

    info!("Created app '{}' with SQLite database and volume litehouse-db-{}", app.name, app.id);

    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::db::test::get_test_pool;
    use crate::models::App;

    #[tokio::test]
    async fn test_create_app_already_exists() {
        let pool = get_test_pool().await;
        let app_name = "test_app";
        let app = App::new(app_name).unwrap();
        db::app::save(&pool, &app).await.unwrap();

        let got = execute(&pool, app_name).await.unwrap_err();
        match got {
            AppCreateError::AppAlreadyExists(ref n) if n == app_name => {}
            _ => panic!("Expected AppAlreadyExists, got: {:?}", got),
        }
    }

    #[tokio::test]
    async fn test_create_app_happy_path() {
        let pool = get_test_pool().await;
        let app_name = "test_app_happy";
        execute(&pool, app_name).await.unwrap();
        let app = db::app::get_by_name(&pool, app_name).await.unwrap();
        assert!(app.is_some());
    }

    #[tokio::test]
    async fn test_create_app_invalid_name() {
        let pool = get_test_pool().await;
        let app_name = "";
        let got = execute(&pool, app_name).await.unwrap_err();
        match got {
            AppCreateError::AppError(_) => {}
            _ => panic!("Expected AppError for invalid name, got: {:?}", got),
        }
    }
}
