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
pub async fn run(app: &App) -> Result<()> {
    // Placeholder for actual teardown logic
    info!("Running app: {}", app.name);

    Ok(())
}

#[instrument]
pub async fn remove(tag: &str) -> Result<()> {
    // Placeholder for actual teardown logic
    info!("Removing container with tag: {}", tag);

    Ok(())
}

#[instrument]
pub async fn stop(app: &App) -> Result<()> {
    // Placeholder for actual teardown logic
    info!("Stopping app: {}", app.name);

    Ok(())
}
