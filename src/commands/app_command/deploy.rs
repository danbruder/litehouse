use crate::providers::podman;
use sha2::{Digest, Sha256};
use sqlx::{Pool, Sqlite};
use std::{fs, path::PathBuf};
use tracing::{info, instrument};

use crate::config;
use crate::db;

#[derive(Debug, thiserror::Error)]
pub enum DeployError {
    #[error("App not found: {0}")]
    AppNotFound(String),
    #[error("Failed to copy binary: {0}")]
    CopyError(String),
    #[error("Failed to set permissions: {0}")]
    PermissionError(String),
    #[error("ConfigError error: {0}")]
    ConfigError(#[from] crate::config::ConfigError),
    #[error("DatabaseError: {0}")]
    DatabaseError(#[from] crate::db::DatabaseError),
    #[error("Dockerfile error: {0}")]
    DockerfileError(String),
    #[error("Podman error: {0}")]
    PodmanError(String),
}

type Result<T> = anyhow::Result<T, DeployError>;

/// Deploy a binary to an app
#[instrument(skip(pool, binary_data))]
pub async fn execute(pool: &Pool<Sqlite>, app_name: &str, binary_data: &[u8]) -> Result<()> {
    info!("Deploying binary to app '{}'", app_name);

    // Get app
    let app = db::apps::get_by_name(pool, app_name)
        .await?
        .ok_or_else(|| DeployError::AppNotFound(app_name.to_string()))?;

    let hash = hash_binary(binary_data);

    if !app.is_hash_changed(&hash) {
        info!("Binary is identical to the currently deployed version.");
        return Ok(());
    }

    let target_path = config::get_app_binary_path(app_name)?
        .to_string_lossy()
        .to_string();

    let app_dir = config::get_app_dir(app_name)?;
    let binary_path = app_dir.join("app");
    let dockerfile_path = app_dir.join("Dockerfile");

    info!("Target path for deployment: {}", target_path);
    copy_and_set_permissions(&binary_path.to_string_lossy(), binary_data)?;
    create_dockerfile(&dockerfile_path, &binary_path)?;
    podman::build_image(&app_dir, &app_name).await.map_err(|e| DeployError::PodmanError(e.to_string()))?;

    // Update and save to database
    let app = app.deployed(target_path, hash);
    db::apps::save(&pool, &app).await?;

    info!("Deployed binary to app '{}'", app_name);

    Ok(())
}

#[instrument(skip(binary_data))]
fn hash_binary(binary_data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(binary_data);
    hex::encode(hasher.finalize())
}

#[instrument(skip(binary_data))]
fn copy_and_set_permissions(target_path: &str, binary_data: &[u8]) -> Result<()> {
    // Save binary to app directory
    fs::write(target_path, binary_data).map_err(|err| DeployError::CopyError(err.to_string()))?;

    // Make binary executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(target_path)
            .map_err(|err| DeployError::PermissionError(err.to_string()))?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&target_path, perms)
            .map_err(|err| DeployError::PermissionError(err.to_string()))?;
    }
    Ok(())
}

fn create_dockerfile(dockerfile_path: &PathBuf, binary_path: &PathBuf) -> Result<()> {
    let dockerfile = format!(
        r#"
    FROM alpine:latest
    COPY {} /binary
    ENTRYPOINT ["/binary"]
    "#,
        binary_path.display()
    );

    fs::write(dockerfile_path, dockerfile)
        .map_err(|err| DeployError::DockerfileError(err.to_string()))?;

    Ok(())
}
