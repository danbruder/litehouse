use crate::models::App;
use anyhow::Result;
use futures_util::TryStreamExt;
use podman_api::Podman;
use std::path::Path;
use std::process::Command;
use tracing::{info, instrument};

#[derive(Debug, thiserror::Error)]
pub enum PodmanError {
    #[error("Dockerfile not found in directory: {0}")]
    DockerfileNotFound(String),
    #[error("Build error: {0}")]
    BuildError(String),
    #[error("Build stream error: {0}")]
    BuildStreamError(String),
}

#[instrument]
pub async fn build(directory: &str, tag: &str) -> Result<()> {
    info!("Building app in: {}", directory);

    let podman = Podman::unix(&resolve_podman_socket_path()?);

    let dockerfile_path = Path::new(directory).join("Dockerfile");
    if !dockerfile_path.exists() {
        return Err(PodmanError::DockerfileNotFound(directory.to_string()).into());
    }

    let build_opts = podman_api::opts::ImageBuildOpts::builder(directory)
        .dockerfile("Dockerfile")
        .tag(tag)
        .build();

    info!("Starting container image build...");
    let images = podman.images();
    let build_stream = images.build(&build_opts)?;

    let mut stream = build_stream;
    while let Some(result) = stream.try_next().await? {
        info!("Build: {}", result.stream.trim());
    }

    info!("Container image build completed successfully");
    Ok(())
}

#[instrument]
pub async fn run(name: &str, image_tag: &str) -> Result<()> {
    info!("Running app: {}", name);

    let podman = Podman::unix(&resolve_podman_socket_path()?);
    let containers = podman.containers();

    // If the container exists
    let all_containers = containers.list(&Default::default()).await?;
    for container in all_containers {
        if let Some(names) = &container.names {
            if names.iter().any(|n| n.contains(name)) {
                info!(
                    "Container with name '{}' already exists. Removing it.",
                    name
                );
                if let Some(id) = &container.id {
                    containers.get(id).remove().await?;
                    info!("Removed existing container with ID: {}", id);
                }
            }
        }
    }
    let container_name = format!("{}-container", name);

    let create_opts = podman_api::opts::ContainerCreateOpts::builder()
        .image(image_tag)
        .name(&container_name)
        .build();

    info!("Creating container: {}", container_name);
    let container_info = containers.create(&create_opts).await?;

    dbg!(&container_info);
    info!("Starting container: {}", container_info.id);
    containers.get(&container_info.id).start(None).await?;

    info!("Container {} started successfully", container_name);

    Ok(())
}

#[instrument]
pub async fn remove(tag: &str) -> Result<()> {
    info!("Removing container image with tag: {}", tag);

    let podman = Podman::unix(&resolve_podman_socket_path()?);
    let images = podman.images();

    match images.get(tag).remove().await {
        Ok(_) => {
            info!("Successfully removed image: {}", tag);
            Ok(())
        }
        Err(e) => {
            info!("Failed to remove image {}: {}", tag, e);
            Err(e.into())
        }
    }
}

#[instrument]
pub async fn stop(app: &App) -> Result<()> {
    info!("Stopping app: {}", app.name);

    let podman = Podman::unix(&resolve_podman_socket_path()?);
    let containers = podman.containers();
    let container_name = format!("{}-container", app.name);

    let list_opts = podman_api::opts::ContainerListOpts::builder()
        .all(true)
        .build();

    let container_list = containers.list(&list_opts).await?;

    for container in container_list {
        if let Some(names) = &container.names {
            if names.iter().any(|name| name.contains(&container_name)) {
                info!(
                    "Stopping container: {}",
                    container.id.as_ref().unwrap_or(&"unknown".to_string())
                );

                if let Some(id) = &container.id {
                    let stop_opts = podman_api::opts::ContainerStopOpts::builder()
                        .timeout(10)
                        .build();

                    containers.get(id).stop(&stop_opts).await?;
                    info!("Successfully stopped container: {}", id);
                }
            }
        }
    }

    Ok(())
}

fn resolve_podman_socket_path() -> Result<String> {
    // User-provided overrides
    if let Ok(sock) = std::env::var("PODMAN_SSH_SOCK") {
        return Ok(sock);
    }
    if let Ok(sock) = std::env::var("PODMAN_SOCK") {
        return Ok(sock);
    }
    if let Ok(ch) = std::env::var("CONTAINER_HOST") {
        if let Some(path) = ch.strip_prefix("unix://") {
            return Ok(path.to_string());
        }
    }

    // Discover from default connection
    let list = Command::new("podman")
        .args([
            "system",
            "connection",
            "ls",
            "--format",
            "{{.Name}} {{.Default}} {{.URI}}",
        ])
        .output()?;

    if list.status.success() {
        let stdout = String::from_utf8_lossy(&list.stdout);
        for line in stdout.lines() {
            let mut parts = line.split_whitespace();
            if let (Some(name), Some(default_flag), Some(uri)) =
                (parts.next(), parts.next(), parts.next())
            {
                if default_flag == "true" {
                    if let Some(path) = uri.strip_prefix("unix://") {
                        return Ok(path.to_string());
                    } else if uri.starts_with("ssh://") {
                        // On macOS with Podman Machine, use the local forwarded API socket
                        let inspect = Command::new("podman")
                            .args([
                                "machine",
                                "inspect",
                                name,
                                "--format",
                                "{{.ConnectionInfo.PodmanSocket.Path}}",
                            ])
                            .output()?;
                        if inspect.status.success() {
                            let path = String::from_utf8_lossy(&inspect.stdout).trim().to_string();
                            if !path.is_empty() {
                                return Ok(path);
                            }
                        }
                    }
                }
            }
        }
    }

    // Fallback to well-known default
    Ok("/run/podman/podman.sock".to_string())
}
