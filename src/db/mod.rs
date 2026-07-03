use sqlx::{sqlite::SqlitePoolOptions, Pool, Sqlite};
use std::path::PathBuf;
use tracing::{debug, info, instrument};

pub mod app;
pub mod deploy;
pub mod env_var;
pub mod system_config;

use crate::config;
use crate::models::{App, AppState};

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
        // Ensure parent directory is writable
        let metadata = std::fs::metadata(parent)?;
        let mut perms = metadata.permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Set to 755 (rwxr-xr-x) to allow owner write, others read/execute
            perms.set_mode(0o755);
        }
        std::fs::set_permissions(parent, perms)?;
    }
    
    // Check if database file exists
    if !db_path.exists() {
        // Create an empty file
        let file = std::fs::File::create(&db_path)?;
        // Ensure the file is writable
        let metadata = file.metadata()?;
        let mut perms = metadata.permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Set to 644 (rw-r--r--) to allow owner read/write, others read
            perms.set_mode(0o644);
        }
        std::fs::set_permissions(&db_path, perms)?;
        info!("Created new database file at {}", db_path.display());
    } else {
        // Ensure existing database file is writable
        let metadata = std::fs::metadata(&db_path)?;
        let mut perms = metadata.permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Ensure owner has write permission
            let mode = perms.mode();
            if mode & 0o200 == 0 {
                // Owner doesn't have write permission, add it
                perms.set_mode(mode | 0o200);
                std::fs::set_permissions(&db_path, perms)?;
                info!("Fixed write permissions on database file at {}", db_path.display());
            }
        }
    }

    // Connect to the database with WAL mode for better concurrency
    let db_url = format!("sqlite:{}?mode=rwc", db_path.display());
    info!("Connecting to database at {}", db_path.display());

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await?;

    // Enable WAL mode for better concurrency (allows multiple readers and one writer)
    sqlx::query("PRAGMA journal_mode=WAL")
        .execute(&pool)
        .await?;
    info!("Enabled WAL mode for database");

    // Run migrations
    sqlx::migrate!("./migrations").run(&pool).await?;

    info!("Database initialized successfully");

    Ok(pool)
}

pub async fn seed() {
    let pool = init_pool().await.unwrap();

    let mut app = App::new("caddy").unwrap();
    app.image = Some("caddy:latest".to_string());
    crate::db::app::save(&pool, &app).await.unwrap();
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
