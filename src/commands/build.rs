use crate::config;
use sqlx::{Pool, Sqlite};
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
#[instrument(skip(pool, github_token))]
pub async fn execute(
    pool: &Pool<Sqlite>,
    app_name: &str,
    github_token: Option<&str>,
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

    // Spawn background task to do the actual build
    let pool_clone = pool.clone();
    let build_id = build.id.clone();
    let app_id = app.id.clone();
    let app_name_clone = app.name.clone();
    let github_token_owned = github_token.map(|s| s.to_string());

    tokio::spawn(async move {
        let result = do_build(
            &pool_clone,
            &build_id,
            &app_id,
            &app_name_clone,
            &remote,
            github_token_owned.as_deref(),
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
                }
                Err(ref e) => {
                    error!("Build failed for app '{}': {}", app_name_clone, e);
                    app.state = AppState::Failed;
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
