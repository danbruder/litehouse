use anyhow::Result;
use bollard::container::{Config, CreateContainerOptions};
use bollard::models::HostConfig;
use bollard::volume::CreateVolumeOptions;
use bollard::Docker;
use std::collections::HashMap;
use tracing::{info, instrument, warn};

/// Utility image for one-shot volume-management containers. Pinned (and
/// pre-pulled during `lh install`); callers ensure it exists via
/// `ensure_utility_image` before running one-shots.
pub const UTILITY_IMAGE: &str = "alpine:3.20";

/// Pull the utility image if it isn't present locally.
pub async fn ensure_utility_image(docker: &Docker) -> Result<()> {
    if !crate::docker::image_exists(UTILITY_IMAGE).await.unwrap_or(false) {
        crate::docker::pull(docker, UTILITY_IMAGE, None).await?;
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum VolumeError {
    #[error("Volume error: {0}")]
    VolumeError(String),
    #[error("Docker error: {0}")]
    DockerError(#[from] bollard::errors::Error),
    #[error("Permission error: {0}")]
    PermissionError(String),
    #[error("Volume in use: {0}")]
    VolumeInUse(String),
}

/// Get the volume name for an app
pub fn get_app_volume_name(app_id: &str) -> String {
    format!("litehouse-db-{}", app_id)
}

/// Get the mount path inside the app container for the database
pub fn get_app_data_mount_path() -> &'static str {
    "/data"
}

/// Create a per-app volume with naming convention litehouse-db-{app_id}
/// Returns the volume name
#[instrument]
pub async fn create_app_volume(docker: &Docker, app_id: &str) -> Result<String> {
    let volume_name = get_app_volume_name(app_id);

    info!("Creating volume for app {}: {}", app_id, volume_name);

    // Check if volume already exists
    let volumes = docker.list_volumes::<String>(None).await?;
    let volume_exists = volumes
        .volumes
        .map(|vols| {
            vols.iter().any(|v| {
                v.name == volume_name
            })
        })
        .unwrap_or(false);

    if volume_exists {
        info!("Volume {} already exists, skipping creation", volume_name);
        return Ok(volume_name);
    }

    // Create the volume
    let mut labels = HashMap::new();
    labels.insert("litehouse.managed".to_string(), "true".to_string());
    labels.insert("litehouse.app_id".to_string(), app_id.to_string());

    let options = CreateVolumeOptions {
        name: volume_name.clone(),
        labels,
        ..Default::default()
    };

    docker.create_volume(options).await?;
    info!("Created volume: {}", volume_name);

    Ok(volume_name)
}

/// Delete an app volume
#[instrument]
pub async fn delete_app_volume(docker: &Docker, app_id: &str) -> Result<()> {
    let volume_name = get_app_volume_name(app_id);

    info!("Deleting volume for app {}: {}", app_id, volume_name);

    // Check if volume exists before attempting to delete
    let volumes = docker.list_volumes::<String>(None).await?;
    let volume_exists = volumes
        .volumes
        .map(|vols| {
            vols.iter().any(|v| {
                v.name == volume_name
            })
        })
        .unwrap_or(false);

    if !volume_exists {
        info!("Volume {} does not exist, skipping deletion", volume_name);
        return Ok(());
    }

    // Remove the volume
    docker.remove_volume(&volume_name, None).await?;
    info!("Deleted volume: {}", volume_name);

    Ok(())
}

/// List all app volumes with their app IDs
/// Returns a vector of (volume_name, app_id) tuples
#[instrument]
pub async fn list_app_volumes(docker: &Docker) -> Result<Vec<(String, String)>> {
    info!("Listing all app volumes");

    let volumes = docker.list_volumes::<String>(None).await?;

    let mut app_volumes = Vec::new();

    if let Some(vols) = volumes.volumes {
        for volume in vols {
            let name = volume.name;
            // Check if this is an app volume (starts with litehouse-db-)
            if name.starts_with("litehouse-db-") {
                // Extract app_id from volume name
                let app_id = name.strip_prefix("litehouse-db-").unwrap_or("");
                if !app_id.is_empty() {
                    app_volumes.push((name.clone(), app_id.to_string()));
                }
            }
        }
    }

    info!("Found {} app volumes", app_volumes.len());
    Ok(app_volumes)
}

/// Inspect Docker image to discover runtime UID/GID from Config.User
/// Returns Some((uid, gid)) if numeric user found, None otherwise
#[instrument]
pub async fn discover_image_user(docker: &Docker, image_tag: &str) -> Result<Option<(u32, u32)>> {
    info!("Discovering user permissions for image: {}", image_tag);

    let image_inspect = docker.inspect_image(image_tag).await?;

    let user_string = image_inspect
        .config
        .and_then(|c| c.user)
        .unwrap_or_default();

    if user_string.is_empty() {
        info!("No User field found in image config, will use fallback permissions");
        return Ok(None);
    }

    // Parse user string - formats: "uid:gid", "uid", or "username"
    // We only support numeric formats
    let parts: Vec<&str> = user_string.split(':').collect();

    let uid: u32 = match parts.get(0) {
        Some(uid_str) => match uid_str.parse() {
            Ok(uid) => uid,
            Err(_) => {
                info!("User field '{}' is not numeric, will use fallback permissions", user_string);
                return Ok(None);
            }
        },
        None => return Ok(None),
    };

    let gid: u32 = match parts.get(1) {
        Some(gid_str) => match gid_str.parse() {
            Ok(gid) => gid,
            Err(_) => uid, // If GID is not numeric, use UID for both
        },
        None => uid, // If no GID specified, use UID for both
    };

    info!("Discovered numeric user: {}:{}", uid, gid);
    Ok(Some((uid, gid)))
}

/// Run ephemeral Alpine container to initialize volume permissions
/// If uid_gid provided: chown uid:gid /data && chmod 0770 /data
/// If None (fallback): chmod 1777 /data (world-writable with sticky bit)
#[instrument]
pub async fn init_app_volume(
    docker: &Docker,
    app_id: &str,
    volume_name: &str,
    uid_gid: Option<(u32, u32)>,
) -> Result<()> {
    ensure_utility_image(docker).await?;
    info!("Initializing volume {} with permissions", volume_name);

    let container_name = format!("litehouse-init-{}", app_id);

    // Build the command based on whether we have uid/gid
    let cmd = if let Some((uid, gid)) = uid_gid {
        info!("Setting permissions for uid:gid {}:{}", uid, gid);
        vec![
            "sh".to_string(),
            "-c".to_string(),
            format!(
                "mkdir -p /data && chown {}:{} /data && chmod 0770 /data",
                uid, gid
            ),
        ]
    } else {
        warn!("No UID/GID discovered, using fallback world-writable permissions");
        vec![
            "sh".to_string(),
            "-c".to_string(),
            "mkdir -p /data && chmod 1777 /data".to_string(),
        ]
    };

    let container_config = Config {
        image: Some(UTILITY_IMAGE.to_string()),
        cmd: Some(cmd),
        host_config: Some(HostConfig {
            binds: Some(vec![format!("{}:/data", volume_name)]),
            auto_remove: Some(true),
            ..Default::default()
        }),
        ..Default::default()
    };

    // Create the container
    let create_options = CreateContainerOptions {
        name: container_name.clone(),
        ..Default::default()
    };

    let container_info = docker
        .create_container(Some(create_options), container_config)
        .await?;

    info!("Created init container: {}", container_info.id);

    // Start the container
    docker
        .start_container::<String>(&container_info.id, None)
        .await?;

    // Wait for container to finish (with timeout)
    let timeout = tokio::time::Duration::from_secs(30);
    let start_time = tokio::time::Instant::now();

    loop {
        if start_time.elapsed() > timeout {
            return Err(anyhow::anyhow!("Init container timed out after 30 seconds"));
        }

        let container = docker.inspect_container(&container_info.id, None).await?;

        if let Some(state) = container.state {
            if let Some(running) = state.running {
                if !running {
                    // Container has finished
                    if let Some(exit_code) = state.exit_code {
                        if exit_code == 0 {
                            info!("Init container completed successfully");
                            return Ok(());
                        } else {
                            return Err(anyhow::anyhow!(
                                "Init container failed with exit code: {}",
                                exit_code
                            ));
                        }
                    }
                }
            }
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }
}

/// Initialize database file in app volume
/// Creates an empty SQLite database at /data/app.db with proper permissions
#[instrument]
pub async fn init_app_database_in_volume(
    docker: &Docker,
    app_id: &str,
    volume_name: &str,
    uid_gid: Option<(u32, u32)>,
) -> Result<()> {
    ensure_utility_image(docker).await?;
    info!("Initializing database file in volume {}", volume_name);

    let container_name = format!("litehouse-init-db-{}", app_id);

    // Build command to create empty SQLite database
    let cmd = if let Some((uid, gid)) = uid_gid {
        info!("Creating database with ownership {}:{}", uid, gid);
        vec![
            "sh".to_string(),
            "-c".to_string(),
            format!(
                "apk add --no-cache sqlite && \
                 mkdir -p /data && \
                 sqlite3 /data/app.db 'VACUUM;' && \
                 chown {}:{} /data/app.db && \
                 chmod 0660 /data/app.db",
                uid, gid
            ),
        ]
    } else {
        info!("Creating database with default permissions");
        vec![
            "sh".to_string(),
            "-c".to_string(),
            "apk add --no-cache sqlite && \
             mkdir -p /data && \
             sqlite3 /data/app.db 'VACUUM;' && \
             chmod 0666 /data/app.db".to_string(),
        ]
    };

    let host_config = HostConfig {
        binds: Some(vec![format!("{}:/data", volume_name)]),
        ..Default::default()
    };

    let config = Config {
        image: Some(UTILITY_IMAGE.to_string()),
        cmd: Some(cmd),
        host_config: Some(host_config),
        ..Default::default()
    };

    let options = CreateContainerOptions {
        name: container_name.clone(),
        platform: None,
    };

    // Create and start container
    let container_info = docker.create_container(Some(options), config).await?;
    docker.start_container::<String>(&container_info.id, None).await?;

    // Wait for container to complete (with timeout)
    let timeout = std::time::Duration::from_secs(60);
    let start = std::time::Instant::now();

    let exit_code = loop {
        if start.elapsed() > timeout {
            warn!("Database initialization timed out after 60s");
            // Clean up container on timeout
            let _ = docker.remove_container(&container_info.id, None).await;
            return Err(VolumeError::PermissionError(
                "Database initialization timed out".to_string()
            ).into());
        }

        let inspect = docker.inspect_container(&container_info.id, None).await?;

        if let Some(state) = inspect.state {
            if let Some(running) = state.running {
                if !running {
                    // Container finished
                    break state.exit_code.unwrap_or(-1);
                }
            }
        }

        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    };

    // Remove the container now that we're done with it
    docker.remove_container(&container_info.id, None).await?;

    if exit_code != 0 {
        return Err(VolumeError::PermissionError(
            format!("Database initialization failed with exit code {}", exit_code)
        ).into());
    }

    info!("Database file initialized successfully");
    Ok(())
}

/// Verify no running container has RW mount on this volume
/// Checks all running containers, returns error if volume is already mounted RW
/// Read-only mounts are allowed
#[instrument]
pub async fn verify_volume_single_writer(
    docker: &Docker,
    app_id: &str,
    volume_name: &str,
) -> Result<()> {
    info!(
        "Verifying single-writer guarantee for volume: {}",
        volume_name
    );

    // List all containers (including stopped ones to be safe)
    let containers = docker
        .list_containers::<String>(Some(bollard::container::ListContainersOptions {
            all: false, // Only check running containers
            ..Default::default()
        }))
        .await?;

    for container in containers {
        let container_id = container.id.unwrap_or_default();

        // Inspect the container to get its mounts. A container that vanished
        // between list and inspect is not a competing writer — skip it.
        let inspect = match docker.inspect_container(&container_id, None).await {
            Ok(inspect) => inspect,
            Err(_) => continue,
        };

        if let Some(mounts) = inspect.mounts {
            for mount in mounts {
                // Check if this mount uses our volume
                if let Some(name) = mount.name {
                    if name == volume_name {
                        // Check if it's read-write (RW is the default, RO means read-only)
                        let is_read_only = mount.rw.map(|rw| !rw).unwrap_or(false);

                        let container_name = inspect
                            .name
                            .clone()
                            .unwrap_or_else(|| container_id.clone());

                        if !is_read_only {
                            // Found a RW mount - this is a conflict
                            return Err(VolumeError::VolumeInUse(format!(
                                "Volume {} is already mounted read-write by container {}",
                                volume_name, container_name
                            ))
                            .into());
                        } else {
                            info!(
                                "Volume {} has read-only mount in container {}, which is allowed",
                                volume_name,
                                container_name
                            );
                        }
                    }
                }
            }
        }
    }

    info!("Single-writer verification passed for volume: {}", volume_name);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docker;

    #[tokio::test]
    async fn test_get_app_volume_name() {
        let volume_name = get_app_volume_name("test-app-123");
        assert_eq!(volume_name, "litehouse-db-test-app-123");
    }

    #[test]
    fn test_get_app_data_mount_path() {
        assert_eq!(get_app_data_mount_path(), "/data");
    }

    #[tokio::test]
    async fn test_create_and_delete_app_volume() -> Result<()> {
        let docker = docker::connect().await?;
        let app_id = "test-volume-create-delete";

        // Create volume
        let volume_name = create_app_volume(&docker, app_id).await?;
        assert_eq!(volume_name, format!("litehouse-db-{}", app_id));

        // Verify volume exists
        let volumes = docker.list_volumes::<String>(None).await?;
        let exists = volumes
            .volumes
            .map(|vols| vols.iter().any(|v| v.name == volume_name))
            .unwrap_or(false);
        assert!(exists, "Volume should exist after creation");

        // Create again (idempotent)
        let volume_name2 = create_app_volume(&docker, app_id).await?;
        assert_eq!(volume_name, volume_name2, "Should return same volume name");

        // Delete volume
        delete_app_volume(&docker, app_id).await?;

        // Verify volume is deleted
        let volumes = docker.list_volumes::<String>(None).await?;
        let exists = volumes
            .volumes
            .map(|vols| vols.iter().any(|v| v.name == volume_name))
            .unwrap_or(false);
        assert!(!exists, "Volume should not exist after deletion");

        // Delete again (idempotent)
        delete_app_volume(&docker, app_id).await?;

        Ok(())
    }

    #[tokio::test]
    async fn test_list_app_volumes() -> Result<()> {
        let docker = docker::connect().await?;

        // Create a few test volumes
        let app_ids = vec!["test-list-1", "test-list-2", "test-list-3"];
        for app_id in &app_ids {
            create_app_volume(&docker, app_id).await?;
        }

        // List volumes
        let volumes = list_app_volumes(&docker).await?;

        // Verify our test volumes are in the list
        for app_id in &app_ids {
            let found = volumes
                .iter()
                .any(|(_, id)| id == app_id);
            assert!(found, "Should find volume for app {}", app_id);
        }

        // Clean up
        for app_id in &app_ids {
            delete_app_volume(&docker, app_id).await?;
        }

        Ok(())
    }

    #[tokio::test]
    async fn test_discover_image_user_alpine() -> Result<()> {
        let docker = docker::connect().await?;

        // Test with alpine image (has no user set)
        let user = discover_image_user(&docker, UTILITY_IMAGE).await?;
        assert_eq!(user, None, "Alpine should have no user set");

        Ok(())
    }

    #[tokio::test]
    async fn test_init_app_volume_fallback() -> Result<()> {
        let docker = docker::connect().await?;
        let app_id = "test-init-fallback";

        // Create volume
        let volume_name = create_app_volume(&docker, app_id).await?;

        // Initialize with fallback permissions (no uid/gid)
        init_app_volume(&docker, app_id, &volume_name, None).await?;

        // Wait a moment for auto-remove to complete
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        // Clean up
        delete_app_volume(&docker, app_id).await?;

        Ok(())
    }

    #[tokio::test]
    async fn test_init_app_volume_with_uid_gid() -> Result<()> {
        let docker = docker::connect().await?;
        let app_id = "test-init-with-uid";

        // Create volume
        let volume_name = create_app_volume(&docker, app_id).await?;

        // Initialize with specific uid/gid
        init_app_volume(&docker, app_id, &volume_name, Some((1000, 1000))).await?;

        // Wait a moment for auto-remove to complete
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        // Clean up
        delete_app_volume(&docker, app_id).await?;

        Ok(())
    }

    #[tokio::test]
    async fn test_verify_volume_single_writer() -> Result<()> {
        let docker = docker::connect().await?;
        let app_id = "test-single-writer";

        // Create volume
        let volume_name = create_app_volume(&docker, app_id).await?;

        // Should pass when no containers use the volume
        verify_volume_single_writer(&docker, app_id, &volume_name).await?;

        // Clean up
        delete_app_volume(&docker, app_id).await?;

        Ok(())
    }

    #[tokio::test]
    async fn test_init_app_database_in_volume() -> Result<()> {
        let docker = docker::connect().await?;
        let app_id = "test-db-init";

        // Create volume
        let volume_name = create_app_volume(&docker, app_id).await?;

        // Initialize database in volume (this will complete when the container finishes)
        // The function internally waits for completion, so if it returns Ok, the database was created
        init_app_database_in_volume(&docker, app_id, &volume_name, None).await?;

        // Verify database file exists by running a simple test container
        let verify_container = format!("litehouse-verify-db-{}", app_id);
        let verify_config = Config {
            image: Some(UTILITY_IMAGE.to_string()),
            cmd: Some(vec![
                "sh".to_string(),
                "-c".to_string(),
                "test -f /data/app.db && echo 'Database exists'".to_string(),
            ]),
            host_config: Some(HostConfig {
                binds: Some(vec![format!("{}:/data", volume_name)]),
                auto_remove: Some(true),
                ..Default::default()
            }),
            ..Default::default()
        };

        let container_info = docker.create_container(
            Some(CreateContainerOptions {
                name: verify_container.clone(),
                ..Default::default()
            }),
            verify_config,
        ).await?;

        docker.start_container::<String>(&container_info.id, None).await?;

        // Wait for verification container to finish
        let timeout = tokio::time::Duration::from_secs(10);
        let start_time = tokio::time::Instant::now();

        loop {
            if start_time.elapsed() > timeout {
                delete_app_volume(&docker, app_id).await?;
                return Err(anyhow::anyhow!("Verification timed out"));
            }

            let inspect = docker.inspect_container(&container_info.id, None).await;
            if let Ok(container) = inspect {
                if let Some(state) = container.state {
                    if let Some(running) = state.running {
                        if !running {
                            let exit_code = state.exit_code.unwrap_or(-1);
                            delete_app_volume(&docker, app_id).await?;
                            if exit_code == 0 {
                                return Ok(());
                            } else {
                                return Err(anyhow::anyhow!("Database file not found in volume"));
                            }
                        }
                    }
                }
            }

            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }
    }

    #[tokio::test]
    async fn test_init_app_database_with_uid_gid() -> Result<()> {
        let docker = docker::connect().await?;
        let app_id = "test-db-init-uid";

        // Create volume
        let volume_name = create_app_volume(&docker, app_id).await?;

        // Initialize database with specific uid/gid
        // The function internally waits for completion, so if it returns Ok, the database was created
        init_app_database_in_volume(&docker, app_id, &volume_name, Some((1000, 1000))).await?;

        // If we get here without error, the database was created successfully
        // Clean up
        delete_app_volume(&docker, app_id).await?;

        Ok(())
    }
}
