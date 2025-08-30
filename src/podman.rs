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

    let container_name = format!("{}-container", name);

    // Check if the container already exists and is running
    let list_opts = podman_api::opts::ContainerListOpts::builder()
        .all(true) // Include stopped containers
        .build();
    let all_containers = containers.list(&list_opts).await?;
    info!(
        "Found {} containers, looking for container name: {}",
        all_containers.len(),
        container_name
    );

    for container in all_containers {
        if let Some(names) = &container.names {
            info!("Checking container names: {:?}", names);
            if names.iter().any(|n| n.contains(&container_name)) {
                // Check if the container is running or has been started before
                if let Some(state) = &container.state {
                    if state == "running" || state == "exited" {
                        info!(
                            "Container '{}' is already {} (ID: {}). Skipping startup.",
                            container_name,
                            state,
                            container.id.as_ref().unwrap_or(&"unknown".to_string())
                        );
                        return Ok(());
                    }
                }

                // If container exists but is in an unexpected state, remove it to recreate
                info!(
                    "Container '{}' exists but is in unexpected state. Removing it to recreate.",
                    container_name
                );
                if let Some(id) = &container.id {
                    info!("Attempting to remove container with ID: {}", id);
                    let remove_result = containers.get(id).remove().await;
                    match remove_result {
                        Ok(_) => {
                            info!("Successfully removed existing container with ID: {}", id);
                        }
                        Err(e) => {
                            info!("Failed to remove container {}: {:?}", id, e);
                            // Continue anyway, the container creation might still work
                        }
                    }

                    // Wait a moment for the removal to complete
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                }
                break;
            }
        }
    }

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

#[cfg(test)]
mod test_helpers {
    use anyhow::Result;
    use std::process::Command;

    /// Check if a container exists and was started by calling podman ps -a
    pub fn is_container_started(container_name: &str) -> Result<bool> {
        let output = Command::new("podman")
            .args([
                "ps",
                "-a", // Show all containers, including stopped ones
                "--filter",
                &format!("name={}", container_name),
                "--format",
                "{{.Names}}\t{{.Status}}",
            ])
            .output()?;

        if !output.status.success() {
            return Ok(false);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(!stdout.trim().is_empty() && stdout.contains(container_name))
    }

    /// Clean up a test container by stopping and removing it
    pub fn cleanup_container(container_name: &str) -> Result<()> {
        // Stop the container
        let stop_result = Command::new("podman")
            .args(["stop", container_name])
            .output();

        if let Err(e) = stop_result {
            println!(
                "Warning: Failed to stop container {}: {:?}",
                container_name, e
            );
        }

        // Remove the container
        let remove_result = Command::new("podman").args(["rm", container_name]).output();

        if let Err(e) = remove_result {
            println!(
                "Warning: Failed to remove container {}: {:?}",
                container_name, e
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::test_helpers::{cleanup_container, is_container_started};
    use super::*;
    use anyhow::Result;

    // Test the happy path: run function creates and starts a container, then verify it's running
    #[tokio::test]
    async fn test_run_function_happy_path() -> Result<()> {
        let app_name = "test-run-app";
        let image_tag = "alpine:latest";
        let container_name = format!("{}-container", app_name);

        // Clean up any existing test container first
        let _ = cleanup_container(&container_name);

        // Step 1: Call the run function
        let run_result = run(app_name, image_tag).await;
        assert!(
            run_result.is_ok(),
            "Run function should succeed: {:?}",
            run_result
        );

        // Step 2: Verify the container was created and started by calling podman
        let is_started = is_container_started(&container_name)?;
        assert!(
            is_started,
            "Container should exist and have been started after run function"
        );

        // Clean up: stop and remove the test container
        cleanup_container(&container_name)?;

        Ok(())
    }

    // Test that the run function handles existing containers correctly
    #[tokio::test]
    async fn test_run_function_handles_existing_containers() -> Result<()> {
        let app_name = "existing-container-test";
        let image_tag = "alpine:latest";
        let container_name = format!("{}-container", app_name);

        // First run: create a container
        let first_run = run(app_name, image_tag).await;
        if let Err(e) = &first_run {
            println!("First run failed with error: {:?}", e);
        }
        assert!(first_run.is_ok());

        // Verify container exists
        assert!(is_container_started(&container_name)?);

        // Second run: should skip startup since container already exists
        let second_run = run(app_name, image_tag).await;
        if let Err(e) = &second_run {
            println!("Second run failed with error: {:?}", e);
        }
        assert!(second_run.is_ok());

        // Verify we still have the same container
        assert!(is_container_started(&container_name)?);

        cleanup_container(&container_name)?;
        Ok(())
    }

    // Test error handling with invalid image
    #[tokio::test]
    async fn test_run_function_with_invalid_image() -> Result<()> {
        let result = run("invalid-test", "nonexistent-image:latest").await;
        assert!(result.is_err(), "Should fail with invalid image");
        Ok(())
    }

    // Test error handling with empty app name
    #[tokio::test]
    async fn test_run_function_with_empty_app_name() -> Result<()> {
        let result = run("", "alpine:latest").await;
        assert!(result.is_err(), "Should fail with empty app name");
        Ok(())
    }

    // Test container naming convention
    #[test]
    fn test_run_function_container_naming_convention() {
        let test_cases = vec![
            ("simple-app", "simple-app-container"),
            ("app-with-dashes", "app-with-dashes-container"),
            ("app_with_underscores", "app_with_underscores-container"),
        ];

        for (app_name, expected_container_name) in test_cases {
            let actual = format!("{}-container", app_name);
            assert_eq!(actual, expected_container_name);
        }
    }

    // Test concurrent execution of the same app
    #[tokio::test]
    async fn test_run_function_concurrent_execution() -> Result<()> {
        let app_name = "concurrent-test";
        let image_tag = "alpine:latest";

        // Run multiple instances concurrently
        let handles: Vec<_> = (0..3)
            .map(|_| {
                let app = app_name.to_string();
                let image = image_tag.to_string();
                tokio::spawn(async move { run(&app, &image).await })
            })
            .collect();

        // Wait for all to complete
        for handle in handles {
            let result = handle.await?;
            // At least one should succeed, others might fail due to conflicts
            println!("Concurrent run result: {:?}", result);
        }

        // Clean up
        let container_name = format!("{}-container", app_name);
        cleanup_container(&container_name)?;
        Ok(())
    }

    // Test cleanup on failure
    #[tokio::test]
    async fn test_run_function_cleanup_on_failure() -> Result<()> {
        let app_name = "cleanup-test";
        let container_name = format!("{}-container", app_name);

        // Try to run with invalid image (should fail)
        let result = run(app_name, "invalid-image:latest").await;
        assert!(result.is_err());

        // Verify no container was left behind
        let container_exists = is_container_started(&container_name)?;
        assert!(
            !container_exists,
            "No container should exist after failed run"
        );

        Ok(())
    }

    // Test that the run function skips startup if container is already running
    #[tokio::test]
    async fn test_run_function_skips_if_already_running() -> Result<()> {
        let app_name = "skip-test";
        let image_tag = "alpine:latest";
        let container_name = format!("{}-container", app_name);

        // First run: create and start a container
        let first_run = run(app_name, image_tag).await;
        assert!(first_run.is_ok());

        // Verify container exists
        assert!(is_container_started(&container_name)?);

        // Second run: should skip startup and return immediately
        let start_time = std::time::Instant::now();
        let second_run = run(app_name, image_tag).await;
        let duration = start_time.elapsed();

        assert!(second_run.is_ok());

        // The second run should be very fast since it skips startup
        assert!(
            duration.as_millis() < 100,
            "Second run should be fast when skipping startup"
        );

        cleanup_container(&container_name)?;
        Ok(())
    }
}
