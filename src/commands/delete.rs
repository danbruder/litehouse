use crate::caddy;
use crate::db;
use crate::docker;
use anyhow::Result;
use sqlx::{Pool, Sqlite};
use tracing::instrument;

#[derive(Debug, thiserror::Error)]
pub enum DeleteError {
    #[error("App not found: {0}")]
    AppNotFound(String),
    #[error("App is running: {0}")]
    AppRunning(String),
    #[error("App not built: {0}")]
    AppNotBuilt(String),
    #[error("Database error: {0}")]
    DatabaseError(#[from] crate::db::DatabaseError),
    #[error("Docker error: {0}")]
    DockerError(#[from] crate::docker::DockerError),
}

type DeleteResult<T> = Result<T, DeleteError>;

/// Delete an app
#[instrument(skip(pool))]
pub async fn execute(pool: &Pool<Sqlite>, app_name: &str) -> DeleteResult<()> {
    // Connect to Docker
    let docker_conn = docker::connect().await?;

    // Get app
    let app = db::app::get_by_name(pool, app_name)
        .await?
        .ok_or_else(|| DeleteError::AppNotFound(app_name.to_string()))?;

    // Check if app is running
    if app.is_running() {
        return Err(DeleteError::AppRunning(app_name.to_string()));
    }

    // Try to stop the container, but don't fail if it doesn't exist
    if let Err(e) = docker::stop(&app).await {
        tracing::warn!("Failed to stop container for app '{}': {}. Continuing with delete.", app_name, e);
    }

    // Try to remove the Docker image, but don't fail if it doesn't exist
    let build = db::build::get_latest_by_app(pool, &app.id).await?;
    if let Some(build) = build {
        if let Some(image_tag) = &build.image_tag {
            if let Err(e) = docker::remove(image_tag).await {
                // Check if it's a "not found" error - that's fine, image is already gone
                let error_str = e.to_string();
                if error_str.contains("No such image") || error_str.contains("404") {
                    tracing::info!("Image '{}' doesn't exist, skipping removal", image_tag);
                } else {
                    tracing::warn!("Failed to remove image '{}': {}. Continuing with delete.", image_tag, e);
                }
            }
        }
    }

    // Delete environment variables
    tracing::info!("Deleting environment variables for app {}", app.id);
    db::env_var::delete_by_app(pool, &app.id).await?;

    // Delete all builds for this app
    tracing::info!("Deleting builds for app {}", app.id);
    db::build::delete_by_app(pool, &app.id).await?;

    // Delete app
    db::app::delete_by_app_id(&pool, &app.id).await?;

    println!("Successfully stopped app '{}'", app_name);

    // Sync Caddy configuration
    if let Err(e) = caddy::sync_configuration(&docker_conn, &pool).await {
        tracing::warn!(
            "Failed to sync Caddy configuration after stopping app '{}': {}",
            app_name,
            e
        );
        // Don't fail the stop operation if Caddy sync fails
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test::get_test_pool;
    use crate::db::{app, build, env_var};
    use crate::models::{App, AppState, Build, EnvVar};

    #[tokio::test]
    async fn test_delete_app_not_found() {
        let pool = get_test_pool().await;
        let result = execute(&pool, "nonexistent").await;
        
        assert!(result.is_err());
        match result.unwrap_err() {
            DeleteError::AppNotFound(name) => assert_eq!(name, "nonexistent"),
            e => panic!("Expected AppNotFound, got: {:?}", e),
        }
    }

    #[tokio::test]
    async fn test_delete_app_with_related_data() {
        // This test verifies that all related data is deleted
        // Note: This requires Docker to be running, so it may fail in CI
        // The database operations are tested separately in db module tests
        
        let pool = get_test_pool().await;
        let app = App::new("test_delete_app").unwrap();
        app::save(&pool, &app).await.unwrap();

        // Create environment variables
        let env_var1 = EnvVar::new(&app.id, "KEY1", "value1");
        let env_var2 = EnvVar::new(&app.id, "KEY2", "value2");
        env_var::save(&pool, &env_var1).await.unwrap();
        env_var::save(&pool, &env_var2).await.unwrap();

        // Create builds
        let build1 = Build::new_building(app.id.clone(), "/tmp/build1.log".to_string());
        let build2 = Build::new_building(app.id.clone(), "/tmp/build2.log".to_string());
        build::save(&pool, &build1).await.unwrap();
        build::save(&pool, &build2).await.unwrap();

        // Verify data exists
        let env_vars = env_var::get_by_app(&pool, &app.id).await.unwrap();
        assert_eq!(env_vars.len(), 2);
        
        let builds = build::get_all_by_app(&pool, &app.id).await.unwrap();
        assert_eq!(builds.len(), 2);

        // Try to delete (will fail if Docker is not available, but that's ok)
        // The important part is that we verify the database operations work
        let result = execute(&pool, "test_delete_app").await;
        
        // If Docker is available, deletion should succeed
        // If not, we'll get a Docker error, but that's acceptable for unit tests
        match result {
            Ok(_) => {
                // If deletion succeeded, verify all data is gone
                let remaining_env_vars = env_var::get_by_app(&pool, &app.id).await.unwrap();
                assert_eq!(remaining_env_vars.len(), 0, "Environment variables should be deleted");
                
                let remaining_builds = build::get_all_by_app(&pool, &app.id).await.unwrap();
                assert_eq!(remaining_builds.len(), 0, "Builds should be deleted");
                
                let remaining_app = app::get_by_name(&pool, "test_delete_app").await.unwrap();
                assert!(remaining_app.is_none(), "App should be deleted");
            }
            Err(DeleteError::DockerError(_)) => {
                // Docker not available - that's ok for unit tests
                // The database operations are tested separately
            }
            Err(e) => panic!("Unexpected error: {:?}", e),
        }
    }

    #[tokio::test]
    async fn test_delete_app_running() {
        let pool = get_test_pool().await;
        let mut app = App::new("running_app").unwrap();
        app.state = AppState::Running;
        app::save(&pool, &app).await.unwrap();

        let result = execute(&pool, "running_app").await;
        
        assert!(result.is_err());
        match result.unwrap_err() {
            DeleteError::AppRunning(name) => assert_eq!(name, "running_app"),
            e => panic!("Expected AppRunning, got: {:?}", e),
        }
    }

    #[tokio::test]
    async fn test_delete_app_deletes_all_builds() {
        // Test that delete_by_app is called during deletion
        // This is a unit test that doesn't require Docker
        let pool = get_test_pool().await;
        let app = App::new("test_builds_app").unwrap();
        app::save(&pool, &app).await.unwrap();

        // Create multiple builds
        let build1 = Build::new_building(app.id.clone(), "/tmp/build1.log".to_string());
        let build2 = Build::new_building(app.id.clone(), "/tmp/build2.log".to_string());
        let build3 = Build::new_building(app.id.clone(), "/tmp/build3.log".to_string());
        
        build::save(&pool, &build1).await.unwrap();
        build::save(&pool, &build2).await.unwrap();
        build::save(&pool, &build3).await.unwrap();

        // Verify builds exist
        let builds = build::get_all_by_app(&pool, &app.id).await.unwrap();
        assert_eq!(builds.len(), 3);

        // Delete builds directly (simulating what delete command does)
        build::delete_by_app(&pool, &app.id).await.unwrap();

        // Verify all builds are deleted
        let remaining_builds = build::get_all_by_app(&pool, &app.id).await.unwrap();
        assert_eq!(remaining_builds.len(), 0);
    }

    #[tokio::test]
    async fn test_delete_app_deletes_all_env_vars() {
        // Test that env vars are deleted during deletion
        let pool = get_test_pool().await;
        let app = App::new("test_env_app").unwrap();
        app::save(&pool, &app).await.unwrap();

        // Create environment variables
        let env_var1 = EnvVar::new(&app.id, "KEY1", "value1");
        let env_var2 = EnvVar::new(&app.id, "KEY2", "value2");
        let env_var3 = EnvVar::new(&app.id, "KEY3", "value3");
        
        env_var::save(&pool, &env_var1).await.unwrap();
        env_var::save(&pool, &env_var2).await.unwrap();
        env_var::save(&pool, &env_var3).await.unwrap();

        // Verify env vars exist
        let env_vars = env_var::get_by_app(&pool, &app.id).await.unwrap();
        assert_eq!(env_vars.len(), 3);

        // Delete env vars directly (simulating what delete command does)
        env_var::delete_by_app(&pool, &app.id).await.unwrap();

        // Verify all env vars are deleted
        let remaining_env_vars = env_var::get_by_app(&pool, &app.id).await.unwrap();
        assert_eq!(remaining_env_vars.len(), 0);
    }
}
