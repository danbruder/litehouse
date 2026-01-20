use crate::config;
use anyhow::Result;
use sqlx::{Pool, Sqlite};
use tracing::{info, instrument};

use crate::db;
use crate::git;
use crate::models::AppState;
use crate::models::Build;
use crate::podman;

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

// Build an app
#[instrument(skip(pool, github_token))]
pub async fn execute(pool: &Pool<Sqlite>, app_name: &str, github_token: Option<&str>) -> BuildResult<Build> {
    // Get app
    let mut app = db::app::get_by_name(pool, app_name)
        .await?
        .ok_or_else(|| BuildError::AppNotFound(app_name.to_string()))?;

    // Check if already building
    if app.state == AppState::Building {
        return Err(BuildError::AlreadyBuilding(app_name.to_string()));
    }

    // Save original state to restore on failure
    let original_state = app.state;

    // Set state to Building
    app.state = AppState::Building;
    db::app::save(pool, &app).await?;

    // Run the build, capturing result to handle state transitions
    let build_result = do_build(pool, &app, github_token).await;

    // Restore state based on result
    match &build_result {
        Ok(_) => {
            // Keep the state as it was (or set to stopped if it was created)
            app.state = if original_state == AppState::Created {
                AppState::Stopped
            } else {
                original_state
            };
        }
        Err(_) => {
            app.state = AppState::Failed;
        }
    }
    db::app::save(pool, &app).await?;

    build_result
}

// Internal build logic
async fn do_build(
    pool: &Pool<Sqlite>,
    app: &crate::models::App,
    github_token: Option<&str>,
) -> BuildResult<Build> {
    use std::path::Path;

    let remote = db::remote::get_by_app(pool, &app.id).await.map_err(|_| {
        BuildError::AppNotConfigured(format!(
            "Remote configuration for app '{}' not found",
            app.name
        ))
    })?;

    let build_dir = config::get_app_build_dir(&app.name)?;

    let git_result = git::pull(&remote, &build_dir, github_token).await?;

    // Generate build ID upfront so we can create the log file
    let build_id = uuid::Uuid::new_v4().to_string();
    let log_path = config::get_build_log_path(&app.name, &build_id)?;
    let log_path_str = log_path.to_string_lossy().to_string();

    let tag = format!("{}:{}", app.name, &git_result.commit);
    let image_id = podman::build_with_log(&build_dir.to_str().unwrap(), &tag, Some(Path::new(&log_path)))
        .await
        .map_err(|e| BuildError::AppNotConfigured(format!("Build failed: {}", e)))?;

    info!("Built image with tag: {} and ID: {}", tag, image_id);

    let now = crate::models::now();
    let build = Build {
        id: build_id,
        app_id: app.id.clone(),
        image_id,
        image_tag: tag,
        git_commit: git_result.commit,
        log_path: Some(log_path_str),
        created_at: now.clone(),
        updated_at: now,
    };
    db::build::save(pool, &build).await?;

    // Cleanup old builds (keep last 30)
    cleanup_old_builds(pool, &app.id).await;

    Ok(build)
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
