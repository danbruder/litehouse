use anyhow::Result;
use podman_api::Podman;
use tracing::{info, instrument};

#[derive(Debug, thiserror::Error)]
pub enum PodmanError {
    #[error("Dockerfile not found in directory: {0}")]
    DockerfileNotFound(String),
    #[error("Build error: {0}")]
    BuildError(String),
    #[error("Build stream error: {0}")]
    BuildStreamError(String),
    #[error("Log error: {0}")]
    LogError(String),
}

#[instrument]
pub async fn start(podman: &Podman) -> Result<()> {
    info!("Running reverse proxy");

    let containers = podman.containers();
    let volumes = podman.volumes();

    let container_name = "caddy-container";

    // Create volumes if they don't exist
    let caddy_data_volume = "caddy_data";
    let caddy_config_volume = "caddy_config";

    // Check if volumes exist, create if they don't
    let volume_list_opts = podman_api::opts::VolumeListOpts::builder().build();
    let volume_list = volumes.list(&volume_list_opts).await?;
    let mut caddy_data_exists = false;
    let mut caddy_config_exists = false;

    for volume in volume_list {
        if volume.name == caddy_data_volume {
            caddy_data_exists = true;
        } else if volume.name == caddy_config_volume {
            caddy_config_exists = true;
        }
    }

    if !caddy_data_exists {
        info!("Creating volume: {}", caddy_data_volume);
        let volume_opts = podman_api::opts::VolumeCreateOpts::builder()
            .name(caddy_data_volume)
            .build();
        volumes.create(&volume_opts).await?;
    }

    if !caddy_config_exists {
        info!("Creating volume: {}", caddy_config_volume);
        let volume_opts = podman_api::opts::VolumeCreateOpts::builder()
            .name(caddy_config_volume)
            .build();
        volumes.create(&volume_opts).await?;
    }

    // Check if container already exists and is running
    let list_opts = podman_api::opts::ContainerListOpts::builder()
        .all(true)
        .build();
    let all_containers = containers.list(&list_opts).await?;

    for container in all_containers {
        if let Some(names) = &container.names {
            if names.iter().any(|n| n == container_name) {
                if let Some(state) = &container.state {
                    if state == "running" {
                        info!("Caddy container is already running");
                        return Ok(());
                    }
                }

                // Remove existing container if it's not running
                if let Some(id) = &container.id {
                    info!("Removing existing caddy container: {}", id);
                    containers.get(id).remove().await?;
                }
                break;
            }
        }
    }

    // Create container with proper configuration
    let create_opts = podman_api::opts::ContainerCreateOpts::builder()
        .image("caddy:latest")
        .name(container_name)
        .restart_policy(podman_api::opts::ContainerRestartPolicy::UnlessStopped)
        .mounts(vec![
            podman_api::models::ContainerMount {
                destination: Some("/data".to_string()),
                source: Some(caddy_data_volume.to_string()),
                _type: Some("volume".to_string()),
                options: None,
                uid_mappings: None,
                gid_mappings: None,
            },
            podman_api::models::ContainerMount {
                destination: Some("/config".to_string()),
                source: Some(caddy_config_volume.to_string()),
                _type: Some("volume".to_string()),
                options: None,
                uid_mappings: None,
                gid_mappings: None,
            },
        ])
        .publish_image_ports(true)
        .build();

    info!("Creating caddy container: {}", container_name);
    let container_info = containers.create(&create_opts).await?;

    info!("Starting caddy container: {}", container_info.id);
    containers.get(&container_info.id).start(None).await?;

    info!("Caddy reverse proxy started successfully");
    Ok(())
}
