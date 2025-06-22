use crate::models::App;
use anyhow::Result;
use podman_api::Podman;
use std::collections::HashMap;
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

    let podman = Podman::unix("/run/podman/podman.sock")?;

    let dockerfile_path = Path::new(directory).join("Dockerfile");
    if !dockerfile_path.exists() {
        return Err(PodmanError::DockerfileNotFound(directory.to_string()).into());
    }

    let build_opts = podman_api::opts::ImageBuildOpts::builder()
        .dockerfile("Dockerfile")
        .context(directory)
        .tag(&format!(
            tag,
            Path::new(directory)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
        ))
        .build();

    info!("Starting container image build...");
    let build_stream = podman.images().build(&build_opts)?;

    for result in build_stream {
        match result {
            Ok(info) => {
                if let Some(stream) = info.stream {
                    info!("Build: {}", stream.trim());
                }
                if let Some(error) = info.error {
                    return Err(PodmanError::BuildError(error).into());
                }
            }
            Err(e) => return Err(PodmanError::BuildStreamError(e.to_string()).into()),
        }
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
pub async fn remove(app: &App) -> Result<()> {
    // Placeholder for actual teardown logic
    info!("Tearing down app: {}", app.name);

    Ok(())
}

#[instrument]
pub async fn stop(app: &App) -> Result<()> {
    // Placeholder for actual teardown logic
    info!("Tearing down app: {}", app.name);

    Ok(())
}
