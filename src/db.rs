use sqlx::{sqlite::SqlitePoolOptions, Pool, Sqlite};
use std::path::PathBuf;
use tracing::{debug, info, instrument};

use crate::config;
use crate::models::{parse_app_state, App, AppState, AppStateChange};

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
    let config_dir = config::get_config_dir()?;
    let db_path = config_dir.join("binarydrop.db");
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

/// App database operations
pub mod apps {
    use super::*;

    /// Save an app to the database
    #[instrument(skip(pool, app))]
    pub async fn save(pool: &Pool<Sqlite>, app: &App) -> Result<()> {
        // Update or insert
        let state = app.state.to_string();
        let result = sqlx::query!(
            r#"
            INSERT INTO apps (
                id, name, created_at, updated_at, state 
            ) VALUES (?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                updated_at = excluded.updated_at,
                state = excluded.state
            "#,
            app.id,
            app.name,
            app.created_at,
            app.updated_at,
            state,
        )
        .execute(pool)
        .await?;

        debug!(
            "Saved app '{}' (affected rows: {})",
            app.name,
            result.rows_affected()
        );

        Ok(())
    }

    /// Get an app by name
    #[instrument(skip(pool))]
    pub async fn get_by_name(pool: &Pool<Sqlite>, name: &str) -> Result<Option<App>> {
        let record = sqlx::query!(
            r#"
            SELECT id, name, created_at, updated_at, state
            FROM apps 
            WHERE name = ?
            "#,
            name
        )
        .fetch_optional(pool)
        .await?;

        match record {
            Some(record) => {
                let state = parse_app_state(&record.state);
                Ok(Some(App {
                    id: record.id,
                    name: record.name,
                    created_at: record.created_at.parse()?,
                    updated_at: record.updated_at.parse()?,
                    state,
                }))
            }
            None => Ok(None),
        }
    }

    /// Get all apps with a specific state
    #[instrument(skip(pool))]
    pub async fn get_by_state(pool: &Pool<Sqlite>, state: AppState) -> Result<Vec<App>> {
        let state_str = state.to_string();

        let records = sqlx::query!(
            r#"
            SELECT id, name, created_at, updated_at, state
            FROM apps 
            WHERE state = ?
            "#,
            state_str
        )
        .fetch_all(pool)
        .await?;

        let mut apps = Vec::new();

        for record in records {
            let state = parse_app_state(&record.state);

            apps.push(App {
                id: record.id,
                name: record.name,
                created_at: record.created_at.parse()?,
                updated_at: record.updated_at.parse()?,
                state,
            });
        }

        Ok(apps)
    }

    /// Delete an app by ID
    #[instrument(skip(pool))]
    pub async fn delete_by_app_id(pool: &Pool<Sqlite>, id: &str) -> Result<()> {
        let _ = sqlx::query!(
            r#"
            DELETE FROM apps
            WHERE id = ?;
            "#,
            id
        )
        .execute(pool)
        .await?;

        Ok(())
    }

    /// Get all apps
    #[instrument(skip(pool))]
    pub async fn get_all(pool: &Pool<Sqlite>) -> Result<Vec<App>> {
        let records = sqlx::query!(
            r#"
            SELECT id, name, created_at, updated_at, state
            FROM apps 
            ORDER BY name
            "#
        )
        .fetch_all(pool)
        .await?;

        let mut apps = Vec::new();

        for record in records {
            let state = parse_app_state(&record.state);
            apps.push(App {
                id: record.id,
                name: record.name,
                created_at: record.created_at.parse()?,
                updated_at: record.updated_at.parse()?,
                state,
            });
        }

        Ok(apps)
    }
}

/// Process history repository
pub mod app_state_change {
    use super::*;

    /// Save a process history entry
    #[instrument(skip(pool, change))]
    pub async fn save(pool: &Pool<Sqlite>, change: &AppStateChange) -> Result<()> {
        // Insert new entry
        sqlx::query!(
            r#"
            INSERT INTO app_state_change (
                id, app_id, created_at, state, last_state, last_error
            ) VALUES (?, ?, ?, ?, ?, ?)
            "#,
            change.id,
            change.app_id,
            change.created_at,
            change.state.to_string(),
            change.last_state.map(|s| s.to_string()),
            change.last_error,
        )
        .execute(pool)
        .await?;

        Ok(())
    }

    /// Get process history for an app
    #[instrument(skip(pool))]
    pub async fn get_by_app_id(pool: &Pool<Sqlite>, app_id: &str) -> Result<Vec<AppStateChange>> {
        let records = sqlx::query!(
            r#"
            SELECT id, app_id, created_at, state, last_state, last_error
            FROM app_state_change
            WHERE app_id = ?
            ORDER BY started_at DESC
            "#,
            app_id
        )
        .fetch_all(pool)
        .await?;

        let mut changes = Vec::new();

        for record in records {
            changes.push(AppStateChange {
                id: record.id,
                app_id: record.app_id,
                created_at: record.created_at.and_utc(),
                state: parse_app_state(&record.state),
                last_state: record.last_state.map(parse_app_state),
                last_error: record.last_error,
            });
        }

        Ok(changes)
    }

    /// Delete process history for an app
    #[instrument(skip(pool))]
    pub async fn delete_by_app_id(pool: &Pool<Sqlite>, app_id: &str) -> Result<u64> {
        let result = sqlx::query!(
            r#"
            DELETE FROM process_history
            WHERE app_id = ?
            "#,
            app_id
        )
        .execute(pool)
        .await?;

        Ok(result.rows_affected())
    }
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
