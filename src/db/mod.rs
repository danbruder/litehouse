use sqlx::{sqlite::SqlitePoolOptions, Pool, Sqlite};
use std::path::PathBuf;
use tracing::{debug, info, instrument};

pub mod app;
pub mod build;
pub mod env_var;
pub mod remote;
pub mod state_change;
pub mod system_config;
pub mod user;
pub mod organization;
pub mod organization_member;
pub mod refresh_token;
pub mod github_connection;

use crate::config;
use crate::models::{parse_app_state, App, AppState, StateChange};

#[derive(Debug, thiserror::Error)]
pub enum DatabaseError {
    #[error("SQLx error: {0}")]
    SqlxError(#[from] sqlx::Error),
    #[error("Migrate error: {0}")]
    MigrationError(#[from] sqlx::migrate::MigrateError),
    #[error("Config error: {0}")]
    ConfigError(#[from] config::ConfigError),
    #[error("Io error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("Serialization Error: {0}")]
    SerdeError(#[from] serde_json::Error),
    #[error("Date parse error: {0}")]
    ChronoError(#[from] chrono::ParseError),
}

type Result<T> = anyhow::Result<T, DatabaseError>;

/// Get the database file path
pub fn get_db_path() -> Result<PathBuf> {
    // First check if DATABASE_URL environment variable is set
    if let Ok(db_url) = std::env::var("DATABASE_URL") {
        info!("Using database path from DATABASE_URL: {}", db_url);
        return Ok(PathBuf::from(db_url));
    }

    // Fall back to default config directory path
    let config_dir = config::get_config_dir()?;
    let db_path = config_dir.join("litehouse.db");
    Ok(db_path)
}

/// Initialize the database connection pool
#[instrument(skip_all)]
pub async fn init_pool() -> Result<Pool<Sqlite>> {
    let db_path = get_db_path()?;

    // Create parent directory if it doesn't exist
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    };
    // Check if database file exists
    if !db_path.exists() {
        // Create an empty file
        std::fs::File::create(&db_path)?;
        info!("Created new database file at {}", db_path.display());
    }

    // Connect to the database
    let db_url = format!("sqlite:{}", db_path.display());
    info!("Connecting to database at {}", db_path.display());

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await?;

    // Run migrations
    sqlx::migrate!("./migrations").run(&pool).await?;

    info!("Database initialized successfully");

    Ok(pool)
}

pub async fn seed() {
    use crate::models::Build;

    let pool = init_pool().await.unwrap();

    let app = App::new("caddy", 8000).unwrap();
    crate::db::app::save(&pool, &app).await.unwrap();

    let mut build = Build::new_building(app.id, "/tmp/caddy.log".to_string());
    build.mark_success("1234".to_string(), "caddy".to_string(), "hey".to_string());
    crate::db::build::save(&pool, &build).await.unwrap();
}

#[cfg(test)]
pub mod test {
    use super::*;

    pub async fn get_test_pool() -> Pool<Sqlite> {
        let pool = SqlitePoolOptions::new()
            .max_connections(3)
            .connect(":memory:")
            .await
            .unwrap();

        // Run migrations
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();

        pool
    }
}
