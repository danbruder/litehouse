use crate::config;
use crate::message_bus::{Message, MessageBus, SubscriptionFilter};
use sqlx::{Pool, Sqlite};
use std::sync::Arc;
use tracing::{error, info, instrument, warn};

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
#[instrument(skip(pool, github_token, message_bus))]
pub async fn execute(
    pool: &Pool<Sqlite>,
    app_name: &str,
    github_token: Option<&str>,
    message_bus: Arc<MessageBus>,
    force: bool,
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
    message_bus.publish(Message::BuildStatus {
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
    let message_bus_clone = message_bus.clone();

    tokio::spawn(async move {
        let result = do_build(
            &pool_clone,
            &build_id,
            &app_id,
            &app_name_clone,
            &remote,
            github_token_owned.as_deref(),
            message_bus_clone.clone(),
            force,
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
                    message_bus_clone.publish(Message::BuildStatus {
                        app_name: app_name_clone.clone(),
                        build_id: build_id.clone(),
                        status: "success".to_string(),
                    });
                }
                Err(ref e) => {
                    error!("Build failed for app '{}': {}", app_name_clone, e);
                    app.state = AppState::Failed;
                    // Publish build failed event
                    message_bus_clone.publish(Message::BuildStatus {
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
    app_id: &str,
    app_name: &str,
    remote: &crate::models::Remote,
    github_token: Option<&str>,
    message_bus: Arc<MessageBus>,
    force: bool,
) -> BuildResult<()> {
    use std::fs::OpenOptions;
    use std::io::Write;

    // Get the build record to get log path
    let mut build = db::build::get_by_id(pool, build_id)
        .await?
        .ok_or_else(|| BuildError::AppNotConfigured("Build record not found".to_string()))?;

    let log_path = build
        .log_path
        .clone()
        .ok_or_else(|| BuildError::AppNotConfigured("Build log path not set".to_string()))?;

    let build_dir = config::get_app_build_dir(app_name)?;

    // Spawn file writer subscriber
    let log_path_clone = log_path.clone();
    let build_id_clone = build_id.to_string();
    let app_name_clone = app_name.to_string();
    let message_bus_clone = message_bus.clone();
    tokio::spawn(async move {
        // Create filter for BuildLogs messages for this app
        let filter = SubscriptionFilter::new(None)
            .with_message_types(vec!["BuildLogs".to_string()])
            .with_app_names(vec![app_name_clone.clone()]);

        // Open log file for writing
        let mut log_file = match OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&log_path_clone)
        {
            Ok(file) => file,
            Err(e) => {
                error!("Failed to open log file {}: {}", log_path_clone, e);
                return;
            }
        };

        // Subscribe to message bus
        let mut rx = message_bus_clone.subscribe();
        let mut build_complete = false;

        while !build_complete {
            match rx.recv().await {
                Ok(msg) => {
                    // Check if this is a build status message indicating completion
                    if let Message::BuildStatus {
                        build_id,
                        status,
                        ..
                    } = &msg
                    {
                        if build_id == &build_id_clone
                            && (status == "success" || status == "failed")
                        {
                            build_complete = true;
                        }
                    }

                    // Filter and write BuildLogs messages
                    if filter.matches(&msg) {
                        if let Message::BuildLogs {
                            build_id,
                            data,
                            ..
                        } = msg
                        {
                            if build_id == build_id_clone {
                                if let Err(e) = writeln!(log_file, "{}", data) {
                                    error!("Failed to write to log file: {}", e);
                                    break;
                                }
                                if let Err(e) = log_file.flush() {
                                    error!("Failed to flush log file: {}", e);
                                    break;
                                }
                            }
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    warn!("File writer lagged, skipped {} messages", skipped);
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    info!("Message bus closed, stopping file writer");
                    break;
                }
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

    // Construct image tag based on the commit we just pulled
    let tag = format!("{}:{}", app_name, &git_result.commit);

    // Check if image already exists (unless force flag is set)
    let mut should_skip_build = false;

    if !force {
        match docker::image_exists(&tag).await {
            Ok(true) => {
                info!("Image {} already exists, skipping build", tag);

                // Check if we have a build record for this commit
                let existing_build = db::build::get_by_commit(pool, app_id, &git_result.commit).await?;

                if existing_build.is_none() {
                    // Image exists but no DB record - create one
                    info!("Creating build record for existing image {}", tag);
                    let mut success_build = Build::new_success(
                        app_id.to_string(),
                        tag.clone(),
                        git_result.commit.clone(),
                    );
                    // Set exposed port if we can get it
                    if let Ok(port) = docker::get_exposed_port(&tag).await {
                        success_build.set_exposed_port(port);
                    }
                    db::build::save(pool, &success_build).await?;
                }
                // Mark that we should skip the actual build
                should_skip_build = true;
            }
            Ok(false) => {
                // Image doesn't exist
                info!("Image {} does not exist, will proceed with build", tag);

                // Check if DB says we have a successful build (image was deleted/lost)
                if let Ok(Some(db_build)) = db::build::get_by_commit(pool, app_id, &git_result.commit).await {
                    if db_build.status.to_string() == "success" {
                        warn!(
                            "Build record exists for commit {} but image {} is missing. This can happen after VM restore or docker system prune. Rebuilding automatically...",
                            git_result.commit, tag
                        );
                    }
                }
                // Don't skip - fall through to rebuild
            }
            Err(e) => {
                // If image check fails, log warning and proceed with build (fail-safe)
                warn!("Failed to check if image exists: {}. Proceeding with build.", e);
            }
        }
    }

    // Skip the actual build if image already exists and is valid
    if should_skip_build {
        return Ok(());
    }

    // Build the Docker image
    let image_id = match docker::build_with_log(
        build_dir.to_str().unwrap(),
        &tag,
        app_name,
        build_id,
        message_bus.clone(),
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

    // Detect and store the exposed port from the Docker image
    let exposed_port = match docker::get_exposed_port(&tag).await {
        Ok(port) => {
            info!("Detected exposed port {} for image {}", port, tag);
            port
        }
        Err(e) => {
            warn!("Failed to detect exposed port, using default 3000: {}", e);
            "3000".to_string()
        }
    };
    build.set_exposed_port(exposed_port);

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
        let _ = crate::config::set_test_dirs(data_dir.path().to_path_buf(), config_dir.path().to_path_buf());
        (data_dir, config_dir)
    }

    #[tokio::test]
    async fn test_build_app_not_found() {
        let pool = get_test_pool().await;
        let message_bus = Arc::new(crate::message_bus::MessageBus::new());

        let result = execute(&pool, "nonexistent-app", None, message_bus, false).await;

        assert!(matches!(result, Err(BuildError::AppNotFound(_))));
    }

    #[tokio::test]
    async fn test_build_already_building() {
        let pool = get_test_pool().await;
        let message_bus = Arc::new(crate::message_bus::MessageBus::new());

        // Create app in Building state
        let mut app = App::new("test-building-app").unwrap();
        app.state = AppState::Building;
        db::app::save(&pool, &app).await.unwrap();

        let result = execute(&pool, "test-building-app", None, message_bus, false).await;

        assert!(matches!(result, Err(BuildError::AlreadyBuilding(_))));
    }

    #[tokio::test]
    async fn test_build_no_remote_configured() {
        let pool = get_test_pool().await;
        let message_bus = Arc::new(crate::message_bus::MessageBus::new());

        // Create app without remote
        let app = App::new("test-no-remote-app").unwrap();
        db::app::save(&pool, &app).await.unwrap();

        let result = execute(&pool, "test-no-remote-app", None, message_bus, false).await;

        assert!(matches!(result, Err(BuildError::AppNotConfigured(_))));
    }

    #[tokio::test]
    async fn test_build_starts_and_creates_build_record() {
        let pool = get_test_pool().await;
        let (_data_dir, _config_dir) = setup_test_dirs();

        // Create app
        let app = App::new("test-build-app").unwrap();
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
        let message_bus = Arc::new(crate::message_bus::MessageBus::new());
        let result = execute(&pool, "test-build-app", None, message_bus, false).await;

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
