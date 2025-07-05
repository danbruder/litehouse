use crate::models::App;
use anyhow::Result;
use futures_util::TryStreamExt;
use podman_api::Podman;
use std::path::Path;
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

    let podman = Podman::unix("/run/podman/podman.sock");

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

    let podman = Podman::unix("/run/podman/podman.sock");
    let containers = podman.containers();

    let container_name = format!("{}-container", name);

    let create_opts = podman_api::opts::ContainerCreateOpts::builder()
        .image(image_tag)
        .name(&container_name)
        .build();

    info!("Creating container: {}", container_name);
    let container_info = containers.create(&create_opts).await?;

    info!("Starting container: {}", container_info.id);
    containers.get(&container_info.id).start(None).await?;

    info!("Container {} started successfully", container_name);

    Ok(())
}

#[instrument]
pub async fn remove(tag: &str) -> Result<()> {
    info!("Removing container image with tag: {}", tag);

    let podman = Podman::unix("/run/podman/podman.sock");
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

    let podman = Podman::unix("/run/podman/podman.sock");
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
