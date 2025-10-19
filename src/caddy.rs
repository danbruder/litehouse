use anyhow::Result;
use futures_util::StreamExt;
use podman_api::opts::PullOpts;
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
    let images = podman.images();

    let container_name = "caddy-container";
    let image_name = "caddy";

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

    // Check if Caddy image exists, pull if it doesn't
    info!("Checking if Caddy image exists");
    let image_list_opts = podman_api::opts::ImageListOpts::builder().build();
    let image_list = images.list(&image_list_opts).await?;
    let mut caddy_image_exists = false;

    for image in image_list {
        if let Some(repo_tags) = &image.repo_tags {
            if repo_tags.iter().any(|tag| tag.starts_with(image_name)) {
                caddy_image_exists = true;
                break;
            }
        }
    }

    if !caddy_image_exists {
        info!("Pulling Caddy image");
        let pull_opts = PullOpts::builder().reference(image_name).build();
        let mut pull_stream = images.pull(&pull_opts);
        while let Some(result) = pull_stream.next().await {
            match result {
                Ok(report) => {
                    if let Some(stream) = report.stream {
                        info!("Pull stream: {}", stream);
                    }
                    if let Some(id) = report.id {
                        info!("Pull ID: {}", id);
                    }
                }
                Err(e) => {
                    tracing::error!("Error pulling image: {}", e);
                    return Err(e.into());
                }
            }
        }
        info!("Caddy image pulled successfully");
    } else {
        info!("Caddy image already exists");
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
        .image(image_name)
        .name(container_name)
        .restart_policy(podman_api::opts::ContainerRestartPolicy::UnlessStopped)
        .volumes(vec![
            podman_api::models::NamedVolume {
                name: Some(caddy_data_volume.to_string()),
                dest: Some("/data".to_string()),
                options: None,
                is_anonymous: Some(false),
            },
            podman_api::models::NamedVolume {
                name: Some(caddy_config_volume.to_string()),
                dest: Some("/config".to_string()),
                options: None,
                is_anonymous: Some(false),
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
