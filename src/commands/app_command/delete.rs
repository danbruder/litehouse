use anyhow::Result;
use sqlx::{Pool, Sqlite};
use tracing::{info, instrument};

use crate::db;
use crate::podman;
use crate::providers::{Handle, Provider};

#[derive(Debug, thiserror::Error)]
pub enum DeleteError {
    #[error("App not found: {0}")]
    AppNotFound(String),
    #[error("App is running: {0}")]
    AppRunning(String),
    #[error("Failed to remove app directory: {0}")]
    DirectoryError(String),
    #[error("Database error: {0}")]
    DatabaseError(#[from] crate::db::DatabaseError),
    #[error("Provider error: {0}")]
    CmdProviderError(#[from] anyhow::Error),
}

type DeleteResult<T> = Result<T, DeleteError>;

/// Delete an app
#[instrument(skip(pool))]
pub async fn execute(pool: &Pool<Sqlite>, app_name: &str) -> DeleteResult<()> {
    // Get app
    let app = db::apps::get_by_name(pool, app_name)
        .await?
        .ok_or_else(|| DeleteError::AppNotFound(app_name.to_string()))?;

    // Check if app is running
    if app.is_running() {
        return Err(DeleteError::AppRunning(app_name.to_string()));
    }

    // Do the provider teardown here
    podman::remove(&app).await?;

    // Delete app from database
    db::apps::delete_by_app_id(pool, &app.id).await?;

    info!("Deleted app '{}'", &app.name);

    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::db::test::get_test_pool;
    use crate::models::App;
    use crate::providers::test::TestProvider;

    #[tokio::test]
    async fn test_deleting_non_existant_app() {
        let pool = get_test_pool().await;
        let got = execute(&pool, "non_existant_app").await.unwrap_err();
        let want = DeleteError::AppNotFound("non_existant_app".to_string());

        assert_eq!(format!("{}", got), format!("{}", want));
    }

    #[tokio::test]
    async fn test_deleting_running_app() {
        let pool = get_test_pool().await;
        let app_name = "test_app";

        // Create and save a running app
        let app = App::new(app_name).unwrap().running(1234);
        db::apps::save(&pool, &app).await.unwrap();

        let got = execute(&pool, app_name).await.unwrap_err();
        let want = DeleteError::AppRunning(app_name.to_string());

        assert_eq!(format!("{}", got), format!("{}", want));
    }

    #[tokio::test]
    async fn test_delete_happy_path() {
        let pool = get_test_pool().await;
        let app_name = "test_app";

        // Create and save a stopped app
        let app = App::new(app_name).unwrap();
        db::apps::save(&pool, &app).await.unwrap();

        // Delete the app
        execute(&pool, app_name).await.unwrap();

        // Verify app is deleted from database
        let app = db::apps::get_by_name(&pool, app_name).await.unwrap();
        assert!(app.is_none());
    }
}
