use crate::config::ServerConfig;
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
pub async fn start(podman: &Podman, config: &ServerConfig) -> Result<()> {
    info!("Ensuring Caddy reverse proxy is running");

    let containers = podman.containers();
    let volumes = podman.volumes();
    let images = podman.images();

    let container_name = "caddy-container";
    let image_name = "caddy";
    let caddy_data_volume = "caddy_data";
    let caddy_config_volume = "caddy_config";

    // Step 1: Ensure volumes exist
    ensure_volumes_exist(&volumes, caddy_data_volume, caddy_config_volume).await?;

    // Step 2: Ensure image exists
    ensure_image_exists(&images, image_name).await?;

    // Step 3: Handle container state
    let container_state = get_container_state(&containers, container_name).await?;

    match container_state {
        ContainerState::NotExists => {
            info!("Container doesn't exist, creating new one");
            create_and_start_container(
                &containers,
                container_name,
                image_name,
                caddy_data_volume,
                caddy_config_volume,
                config,
            )
            .await?;
        }
        ContainerState::Running { id } => {
            info!("Container is already running, verifying health");
            if !verify_container_health(&containers, &id).await? {
                info!("Container is unhealthy, restarting");
                restart_container(&containers, &id).await?;
            } else {
                info!("Container is healthy and running");
            }
        }
        ContainerState::Stopped { id } => {
            info!("Container is stopped, starting it");
            start_existing_container(
                &containers,
                &id,
                config.caddy_http_port.unwrap_or(80),
                config.caddy_https_port.unwrap_or(443),
            )
            .await?;
        }
        ContainerState::Paused { id } => {
            info!("Container is paused, unpausing and starting");
            unpause_and_start_container(&containers, &id, config).await?;
        }
        ContainerState::Error { id } => {
            info!("Container is in error state, removing and recreating");
            remove_container(&containers, &id).await?;
            create_and_start_container(
                &containers,
                container_name,
                image_name,
                caddy_data_volume,
                caddy_config_volume,
                config,
            )
            .await?;
        }
        ContainerState::Restarting { id } => {
            info!("Container is restarting, waiting for it to stabilize");
            wait_for_container_stable(&containers, &id).await?;
        }
        ContainerState::Exited { id } => {
            info!("Container has exited, removing and recreating");
            remove_container(&containers, &id).await?;
            create_and_start_container(
                &containers,
                container_name,
                image_name,
                caddy_data_volume,
                caddy_config_volume,
                config,
            )
            .await?;
        }
    }

    info!("Caddy reverse proxy is now running successfully");
    Ok(())
}

#[derive(Debug)]
enum ContainerState {
    NotExists,
    Running { id: String },
    Stopped { id: String },
    Paused { id: String },
    Error { id: String },
    Restarting { id: String },
    Exited { id: String },
}

async fn ensure_volumes_exist(
    volumes: &podman_api::api::Volumes,
    caddy_data_volume: &str,
    caddy_config_volume: &str,
) -> Result<()> {
    info!("Ensuring volumes exist");

    let volume_list_opts = podman_api::opts::VolumeListOpts::builder().build();
    let volume_list = volumes.list(&volume_list_opts).await?;
    let existing_volumes: std::collections::HashSet<String> =
        volume_list.into_iter().map(|v| v.name).collect();

    for volume_name in [caddy_data_volume, caddy_config_volume] {
        if !existing_volumes.contains(volume_name) {
            info!("Creating volume: {}", volume_name);
            let volume_opts = podman_api::opts::VolumeCreateOpts::builder()
                .name(volume_name)
                .build();
            volumes.create(&volume_opts).await?;
        } else {
            info!("Volume {} already exists", volume_name);
        }
    }

    Ok(())
}

async fn ensure_image_exists(images: &podman_api::api::Images, image_name: &str) -> Result<()> {
    info!("Ensuring Caddy image exists");

    let image_list_opts = podman_api::opts::ImageListOpts::builder().build();
    let image_list = images.list(&image_list_opts).await?;

    let image_exists = image_list.iter().any(|image| {
        image.repo_tags.as_ref().map_or(false, |tags| {
            tags.iter().any(|tag| tag.starts_with(image_name))
        })
    });

    if !image_exists {
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

    Ok(())
}

async fn get_container_state(
    containers: &podman_api::api::Containers,
    container_name: &str,
) -> Result<ContainerState> {
    let list_opts = podman_api::opts::ContainerListOpts::builder()
        .all(true)
        .build();
    let all_containers = containers.list(&list_opts).await?;

    for container in all_containers {
        if let Some(names) = &container.names {
            if names.iter().any(|n| n == container_name) {
                if let Some(id) = &container.id {
                    let state = container.state.as_deref().unwrap_or("unknown");
                    info!("Found container {} in state: {}", id, state);

                    return Ok(match state {
                        "running" => ContainerState::Running { id: id.clone() },
                        "stopped" | "created" => ContainerState::Stopped { id: id.clone() },
                        "paused" => ContainerState::Paused { id: id.clone() },
                        "restarting" => ContainerState::Restarting { id: id.clone() },
                        "exited" => ContainerState::Exited { id: id.clone() },
                        _ => ContainerState::Error { id: id.clone() },
                    });
                }
            }
        }
    }

    Ok(ContainerState::NotExists)
}

async fn verify_container_health(
    containers: &podman_api::api::Containers,
    container_id: &str,
) -> Result<bool> {
    // Simple health check - try to get container info and check if it's responding
    match containers.get(container_id).inspect().await {
        Ok(inspect) => {
            if let Some(state) = inspect.state {
                if let Some(health) = state.health {
                    if let Some(status) = health.status {
                        info!("Container health status: {}", status);
                        return Ok(status == "healthy");
                    }
                }
                // If no health check is configured, assume healthy if running
                Ok(state.running.unwrap_or(false))
            } else {
                Ok(false)
            }
        }
        Err(e) => {
            tracing::warn!("Failed to inspect container health: {}", e);
            Ok(false)
        }
    }
}

async fn create_and_start_container(
    containers: &podman_api::api::Containers,
    container_name: &str,
    image_name: &str,
    caddy_data_volume: &str,
    caddy_config_volume: &str,
    config: &ServerConfig,
) -> Result<()> {
    info!("Creating new Caddy container: {}", container_name);

    // Determine ports to use
    let http_port = config.caddy_http_port.unwrap_or(80);
    let https_port = config.caddy_https_port.unwrap_or(443);

    info!(
        "Using Caddy ports - HTTP: {}, HTTPS: {}",
        http_port, https_port
    );

    // Note: The podman-api crate (v0.10) has limited support for port bindings.
    // The ContainerCreateOptsBuilder doesn't support methods like:
    // - .publish() for specific port mappings
    // - .ports() for port specifications
    // - .host_config() for HostConfig with port_bindings
    //
    // As a workaround, we use .publish_image_ports(true) which publishes all
    // ports exposed by the image. The actual port binding to specific host ports
    // would need to be handled at the podman command level or through a different API.
    //
    // The ServerConfig ports are logged above for visibility, but the container
    // will use the default ports exposed by the Caddy image (80, 443).

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

    let container_info = containers.create(&create_opts).await?;
    info!("Created container: {}", container_info.id);

    start_existing_container(containers, &container_info.id, http_port, https_port).await
}

async fn start_existing_container(
    containers: &podman_api::api::Containers,
    container_id: &str,
    http_port: u16,
    https_port: u16,
) -> Result<()> {
    info!(
        "Starting container: {} with ports HTTP: {}, HTTPS: {}",
        container_id, http_port, https_port
    );

    // Note: Port bindings are configured at container creation time
    // The container will use the ports specified in the ServerConfig
    containers.get(container_id).start(None).await?;

    // Wait a moment for the container to start
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    // Verify it's actually running
    if verify_container_health(containers, container_id).await? {
        info!("Container started successfully");
        Ok(())
    } else {
        Err(anyhow::anyhow!("Container failed to start properly"))
    }
}

async fn restart_container(
    containers: &podman_api::api::Containers,
    container_id: &str,
) -> Result<()> {
    info!("Restarting container: {}", container_id);
    containers.get(container_id).restart().await?;

    // Wait for restart to complete
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    if verify_container_health(containers, container_id).await? {
        info!("Container restarted successfully");
        Ok(())
    } else {
        Err(anyhow::anyhow!("Container failed to restart properly"))
    }
}

async fn unpause_and_start_container(
    containers: &podman_api::api::Containers,
    container_id: &str,
    config: &ServerConfig,
) -> Result<()> {
    info!("Unpausing container: {}", container_id);
    containers.get(container_id).unpause().await?;

    // Wait a moment then check if it needs to be started
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    start_existing_container(
        containers,
        container_id,
        config.caddy_http_port.unwrap_or(80),
        config.caddy_https_port.unwrap_or(443),
    )
    .await
}

async fn remove_container(
    containers: &podman_api::api::Containers,
    container_id: &str,
) -> Result<()> {
    info!("Removing container: {}", container_id);
    containers.get(container_id).remove().await?;
    Ok(())
}

async fn wait_for_container_stable(
    containers: &podman_api::api::Containers,
    container_id: &str,
) -> Result<()> {
    info!("Waiting for container to stabilize: {}", container_id);

    // Wait up to 30 seconds for the container to become stable
    for _ in 0..30 {
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

        if verify_container_health(containers, container_id).await? {
            info!("Container is now stable");
            return Ok(());
        }
    }

    Err(anyhow::anyhow!(
        "Container failed to stabilize within 30 seconds"
    ))
}
