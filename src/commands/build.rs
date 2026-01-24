use crate::config;
use crate::sse::SSEHub;
use sqlx::{Pool, Sqlite};
use std::sync::Arc;
use tracing::{error, info, instrument};

use crate::db;
use crate::docker;
use crate::git;
use crate::models::AppState;
use crate::models::Build;

#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error("App not found: {0}")]
    AppNotFound(String),
    #[error("App is already building: {0}")]
    AlreadyBuilding(String),
    #[error("App git configuration not set up: {0}")]
    AppNotConfigured(String),
    #[error("Database error: {0}")]
    DatabaseError(#[from] crate::db::DatabaseError),
    #[error("Config error: {0}")]
    ConfigError(#[from] crate::config::ConfigError),
    #[error("Git error: {0}")]
    GitError(#[from] crate::git::GitError),
}

type BuildResult<T> = Result<T, BuildError>;

/// Start an async build for an app. Returns immediately with a Build record that has status=building.
/// The actual build runs in a background task.
#[instrument(skip(pool, github_token, sse_hub))]
pub async fn execute(
    pool: &Pool<Sqlite>,
    app_name: &str,
    github_token: Option<&str>,
    sse_hub: Arc<SSEHub>,
) -> BuildResult<Build> {
    // Get app
    let mut app = db::app::get_by_name(pool, app_name)
        .await?
        .ok_or_else(|| BuildError::AppNotFound(app_name.to_string()))?;

    // Check if already building
    if app.state == AppState::Building {
        return Err(BuildError::AlreadyBuilding(app_name.to_string()));
    }

    // Verify remote is configured before we start
    let remote = db::remote::get_by_app(pool, &app.id).await.map_err(|_| {
        BuildError::AppNotConfigured(format!(
            "Remote configuration for app '{}' not found",
            app.name
        ))
    })?;

    // Save original state to restore on failure
    let original_state = app.state.clone();

    // Set state to Building
    app.state = AppState::Building;
    db::app::save(pool, &app).await?;

    // Create build record upfront with status=building
    let log_path = config::get_build_log_path(&app.name, &uuid::Uuid::new_v4().to_string())?;
    let log_path_str = log_path.to_string_lossy().to_string();
    let build = Build::new_building(app.id.clone(), log_path_str);
    db::build::save(pool, &build).await?;

    // Publish build started event
    sse_hub.publish(crate::sse::SSEMessage::BuildStatus {
        app_name: app.name.clone(),
        build_id: build.id.clone(),
        status: "building".to_string(),
    });

    // Spawn background task to do the actual build
    let pool_clone = pool.clone();
    let build_id = build.id.clone();
    let app_id = app.id.clone();
    let app_name_clone = app.name.clone();
    let github_token_owned = github_token.map(|s| s.to_string());
    let sse_hub_clone = sse_hub.clone();

    tokio::spawn(async move {
        let result = do_build(
            &pool_clone,
            &build_id,
            &app_id,
            &app_name_clone,
            &remote,
            github_token_owned.as_deref(),
            sse_hub_clone.clone(),
        )
        .await;

        // Update app state based on result
        if let Ok(Some(mut app)) = db::app::get_by_id(&pool_clone, &app_id).await {
            match result {
                Ok(_) => {
                    app.state = if original_state == AppState::Created {
                        AppState::Stopped
                    } else {
                        original_state
                    };
                    // Publish build success event
                    sse_hub_clone.publish(crate::sse::SSEMessage::BuildStatus {
                        app_name: app_name_clone.clone(),
                        build_id: build_id.clone(),
                        status: "success".to_string(),
                    });
                }
                Err(ref e) => {
                    error!("Build failed for app '{}': {}", app_name_clone, e);
                    app.state = AppState::Failed;
                    // Publish build failed event
                    sse_hub_clone.publish(crate::sse::SSEMessage::BuildStatus {
                        app_name: app_name_clone.clone(),
                        build_id: build_id.clone(),
                        status: "failed".to_string(),
                    });
                }
            }
            if let Err(e) = db::app::save(&pool_clone, &app).await {
                error!("Failed to update app state after build: {}", e);
            }
        }

        // Cleanup old builds
        cleanup_old_builds(&pool_clone, &app_id).await;
    });

    Ok(build)
}

/// Internal build logic - runs in background task
async fn do_build(
    pool: &Pool<Sqlite>,
    build_id: &str,
    _app_id: &str,
    app_name: &str,
    remote: &crate::models::Remote,
    github_token: Option<&str>,
    sse_hub: Arc<SSEHub>,
) -> BuildResult<()> {
    use std::path::Path;

    // Get the build record to get log path
    let mut build = db::build::get_by_id(pool, build_id)
        .await?
        .ok_or_else(|| BuildError::AppNotConfigured("Build record not found".to_string()))?;

    let log_path = build
        .log_path
        .clone()
        .ok_or_else(|| BuildError::AppNotConfigured("Build log path not set".to_string()))?;

    let build_dir = config::get_app_build_dir(app_name)?;

    // Spawn task to tail log file and publish to SSE
    let log_path_clone = log_path.clone();
    let app_name_clone = app_name.to_string();
    let build_id_clone = build_id.to_string();
    let sse_hub_clone = sse_hub.clone();
    tokio::spawn(async move {
        use tokio::fs::File;
        use tokio::io::{AsyncBufReadExt, BufReader};
        use tokio::time::{sleep, Duration};

        // Wait for log file to be created
        for _ in 0..30 {
            if tokio::fs::metadata(&log_path_clone).await.is_ok() {
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }

        // Tail the log file
        if let Ok(file) = File::open(&log_path_clone).await {
            let reader = BufReader::new(file);
            let mut lines = reader.lines();

            while let Ok(Some(line)) = lines.next_line().await {
                sse_hub_clone.publish(crate::sse::SSEMessage::BuildLogs {
                    app_name: app_name_clone.clone(),
                    build_id: build_id_clone.clone(),
                    event_type: "message".to_string(),
                    data: line,
                });
            }
        }
    });

    // Pull the git repo
    let git_result = match git::pull(remote, &build_dir, github_token).await {
        Ok(result) => result,
        Err(e) => {
            // Mark build as failed
            build.mark_failed();
            let _ = db::build::save(pool, &build).await;
            return Err(BuildError::GitError(e));
        }
    };

    // Build the Docker image
    let tag = format!("{}:{}", app_name, &git_result.commit);
    let image_id = match docker::build_with_log(
        build_dir.to_str().unwrap(),
        &tag,
        Some(Path::new(&log_path)),
    )
    .await
    {
        Ok(id) => id,
        Err(e) => {
            // Mark build as failed
            build.mark_failed();
            let _ = db::build::save(pool, &build).await;
            return Err(BuildError::AppNotConfigured(format!("Build failed: {}", e)));
        }
    };

    info!("Built image with tag: {} and ID: {}", tag, image_id);

    // Mark build as successful
    build.mark_success(image_id, tag, git_result.commit);
    db::build::save(pool, &build).await?;

    Ok(())
}

/// Cleanup old builds, keeping only the most recent MAX_BUILDS_TO_KEEP builds
const MAX_BUILDS_TO_KEEP: i64 = 30;

async fn cleanup_old_builds(pool: &Pool<Sqlite>, app_id: &str) {
    match db::build::delete_old_builds(pool, app_id, MAX_BUILDS_TO_KEEP).await {
        Ok(deleted_builds) => {
            for build in deleted_builds {
                // Delete log file if it exists
                if let Some(log_path) = &build.log_path {
                    if let Err(e) = std::fs::remove_file(log_path) {
                        // Log but don't fail - file might already be gone
                        info!("Failed to delete old build log {}: {}", log_path, e);
                    } else {
                        info!("Deleted old build log: {}", log_path);
                    }
                }
            }
        }
        Err(e) => {
            // Log but don't fail the build
            info!("Failed to cleanup old builds: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test::get_test_pool;
    use crate::models::{App, Remote};
    use tempfile::TempDir;

    /// Helper to set up test directories for config module
    fn setup_test_dirs() -> (TempDir, TempDir) {
        let data_dir = tempfile::tempdir().unwrap();
        let config_dir = tempfile::tempdir().unwrap();
        crate::config::set_test_dirs(data_dir.path().to_path_buf(), config_dir.path().to_path_buf());
        (data_dir, config_dir)
    }

    #[tokio::test]
    async fn test_build_app_not_found() {
        let pool = get_test_pool().await;

        let result = execute(&pool, "nonexistent-app", None).await;

        assert!(matches!(result, Err(BuildError::AppNotFound(_))));
    }

    #[tokio::test]
    async fn test_build_already_building() {
        let pool = get_test_pool().await;

        // Create app in Building state
        let mut app = App::new("test-building-app", 8000).unwrap();
        app.state = AppState::Building;
        db::app::save(&pool, &app).await.unwrap();

        let result = execute(&pool, "test-building-app", None).await;

        assert!(matches!(result, Err(BuildError::AlreadyBuilding(_))));
    }

    #[tokio::test]
    async fn test_build_no_remote_configured() {
        let pool = get_test_pool().await;

        // Create app without remote
        let app = App::new("test-no-remote-app", 8001).unwrap();
        db::app::save(&pool, &app).await.unwrap();

        let result = execute(&pool, "test-no-remote-app", None).await;

        assert!(matches!(result, Err(BuildError::AppNotConfigured(_))));
    }

    #[tokio::test]
    async fn test_build_starts_and_creates_build_record() {
        let pool = get_test_pool().await;
        let (_data_dir, _config_dir) = setup_test_dirs();

        // Create app
        let app = App::new("test-build-app", 8002).unwrap();
        db::app::save(&pool, &app).await.unwrap();

        // Create remote - use a fake git repo path
        let remote = Remote::new(
            &app.id,
            "origin",
            "https://github.com/test/repo.git",
            "main",
            "/tmp/test-build-dir",
        );
        db::remote::save(&pool, &remote).await.unwrap();

        // Execute build - this should start the build and return immediately
        let result = execute(&pool, "test-build-app", None).await;

        // Build should start successfully (returns a build record)
        assert!(result.is_ok(), "Build should start: {:?}", result.err());
        let build = result.unwrap();

        // Verify build record was created
        assert!(!build.id.is_empty());
        assert_eq!(build.app_id, app.id);
        assert!(build.log_path.is_some());

        // Verify app state changed to Building
        let updated_app = db::app::get_by_name(&pool, "test-build-app").await.unwrap().unwrap();
        assert_eq!(updated_app.state, AppState::Building);
    }
}
