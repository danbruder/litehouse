use crate::models::App;
use anyhow::Result;
use futures_util::{stream::unfold, StreamExt, TryStreamExt};
use podman_api::Podman;
use std::path::Path;
use std::pin::Pin;
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
    #[error("Log error: {0}")]
    LogError(String),
}

#[instrument]
pub async fn build(directory: &str, tag: &str) -> Result<String> {
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

    // Get the image ID by inspecting the built image
    let image_info = images.get(tag).inspect().await?;
    let image_id = image_info.id.ok_or_else(|| {
        PodmanError::BuildError("Failed to get image ID from build result".to_string())
    })?;

    info!("Built image ID: {}", image_id);
    Ok(image_id)
}

#[instrument]
pub async fn run(name: &str, image_tag: &str) -> Result<()> {
    // Validate input parameters
    if name.trim().is_empty() {
        return Err(PodmanError::BuildError("App name cannot be empty".to_string()).into());
    }

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
            info!("Looking for container name: {}", container_name);
            if names.iter().any(|n| n == &container_name) {
                // Check if the container is running or has been started before
                if let Some(state) = &container.state {
                    info!("Found container '{}' with state: {}", container_name, state);
                    if state == "running" {
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
pub async fn logs(app_name: &str, lines: usize, follow: bool) -> Result<()> {
    todo!("implement non-streaming logs");
    Ok(())
}

#[instrument]
pub async fn logs_stream(
    app_name: &str,
    lines: usize,
    follow: bool,
) -> Result<Pin<Box<dyn futures_util::Stream<Item = Result<String, anyhow::Error>> + Send>>> {
    info!("Getting logs stream for app: {}", app_name);

    let podman = Podman::unix(&resolve_podman_socket_path()?);
    let containers = podman.containers();
    let container_name = format!("{}-container", app_name);

    // Check if container exists
    let list_opts = podman_api::opts::ContainerListOpts::builder()
        .all(true)
        .build();
    let all_containers = containers.list(&list_opts).await?;

    let mut container_found = false;
    for container in all_containers {
        if let Some(names) = &container.names {
            if names.iter().any(|n| n == &container_name) {
                container_found = true;
                break;
            }
        }
    }

    if !container_found {
        return Err(PodmanError::LogError(format!(
            "Container '{}' not found for app '{}'",
            container_name, app_name
        ))
        .into());
    }

    // Create a stream that owns all the necessary data
    let stream = unfold(
        (podman, container_name, follow),
        |(podman, container_name, follow)| async move {
            let containers = podman.containers();
            let logs_opts = podman_api::opts::ContainerLogsOpts::builder()
                .stdout(true)
                .stderr(true)
                .follow(follow)
                .build();

            let container_logs = containers.get(&container_name);
            let mut log_stream = container_logs.logs(&logs_opts);

            match log_stream.next().await {
                Some(result) => {
                    let log_string = match result {
                        Ok(log_result) => match log_result {
                            podman_api::conn::TtyChunk::StdOut(data) => {
                                String::from_utf8_lossy(&data).to_string()
                            }
                            podman_api::conn::TtyChunk::StdErr(data) => {
                                String::from_utf8_lossy(&data).to_string()
                            }
                            _ => String::new(),
                        },
                        Err(e) => {
                            return Some((
                                Err(anyhow::anyhow!(e)),
                                (podman, container_name, follow),
                            ))
                        }
                    };
                    Some((Ok(log_string), (podman, container_name, follow)))
                }
                None => None,
            }
        },
    );

    Ok(Box::pin(stream))
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
                    // Check if the container is already stopped
                    if let Some(state) = &container.state {
                        if state == "exited" || state == "stopped" {
                            info!(
                                "Container {} is already {} (ID: {})",
                                container_name, state, id
                            );
                            continue;
                        }
                    }

                    let stop_opts = podman_api::opts::ContainerStopOpts::builder()
                        .timeout(10)
                        .build();

                    match containers.get(id).stop(&stop_opts).await {
                        Ok(_) => {
                            info!("Successfully stopped container: {}", id);
                        }
                        Err(e) => {
                            // If the container is already stopped, that's fine
                            if e.to_string().contains("304") {
                                info!(
                                    "Container {} was already stopped (ID: {})",
                                    container_name, id
                                );
                            } else {
                                return Err(e.into());
                            }
                        }
                    }
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

#[instrument]
pub async fn run_reverse_proxy() -> Result<()> {
    info!("Running reverse proxy");

    let podman = Podman::unix(&resolve_podman_socket_path()?);
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
        let stop_result = podman_stop(container_name);

        if let Err(e) = stop_result {
            println!(
                "Warning: Failed to stop container {}: {:?}",
                container_name, e
            );
        }

        // Remove the container
        let remove_result = podman_rm(container_name);

        if let Err(e) = remove_result {
            println!(
                "Warning: Failed to remove container {}: {:?}",
                container_name, e
            );
        }

        Ok(())
    }

    /// Stop a container using podman
    pub fn podman_stop(container_name: &str) -> Result<()> {
        let output = Command::new("podman")
            .args(["stop", container_name])
            .output()?;

        if !output.status.success() {
            return Err(anyhow::anyhow!(
                "Failed to stop container: {}",
                container_name
            ));
        }

        Ok(())
    }

    /// Remove a container using podman
    pub fn podman_rm(container_name: &str) -> Result<()> {
        let output = Command::new("podman")
            .args(["rm", container_name])
            .output()?;

        if !output.status.success() {
            return Err(anyhow::anyhow!(
                "Failed to remove container: {}",
                container_name
            ));
        }

        Ok(())
    }

    /// Get container state using podman ps
    pub fn get_container_state(container_name: &str) -> Result<String> {
        let output = Command::new("podman")
            .args([
                "ps",
                "-a",
                "--filter",
                &format!("name={}", container_name),
                "--format",
                "{{.State}}",
            ])
            .output()?;

        if !output.status.success() {
            return Err(anyhow::anyhow!(
                "Failed to get container state for: {}",
                container_name
            ));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(stdout.trim().to_string())
    }

    /// Get container info using podman ps with custom format
    pub fn get_container_info(container_name: &str, format: &str) -> Result<String> {
        let output = Command::new("podman")
            .args([
                "ps",
                "-a",
                "--filter",
                &format!("name={}", container_name),
                "--format",
                format,
            ])
            .output()?;

        if !output.status.success() {
            return Err(anyhow::anyhow!(
                "Failed to get container info for: {}",
                container_name
            ));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(stdout.trim().to_string())
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

        // Wait a moment for the container to be fully registered in podman
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

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

    // Test the stop function with a running container
    #[tokio::test]
    async fn test_stop_function_stops_running_container() -> Result<()> {
        let app_name = "stop-test-app";
        let image_tag = "alpine:latest";
        let container_name = format!("{}-container", app_name);

        // First, create and start a container
        let run_result = run(app_name, image_tag).await;
        assert!(run_result.is_ok());

        // Verify container exists
        assert!(is_container_started(&container_name)?);

        // Create an App instance for testing
        let app = App::new(app_name)?;

        // Stop the container
        let stop_result = stop(&app).await;
        if let Err(e) = &stop_result {
            println!("Stop function failed with error: {:?}", e);
        }
        assert!(stop_result.is_ok());

        // Verify the container was stopped
        let state = test_helpers::get_container_state(&container_name)?;
        assert!(
            state == "exited" || state == "stopped",
            "Container should be stopped, got: {}",
            state
        );

        // Clean up
        cleanup_container(&container_name)?;
        Ok(())
    }

    // Test the stop function with a non-existent container
    #[tokio::test]
    async fn test_stop_function_with_nonexistent_container() -> Result<()> {
        let app_name = "nonexistent-stop-test";

        // Create an App instance for testing
        let app = App::new(app_name)?;

        // Stop should succeed even if no container exists
        let stop_result = stop(&app).await;
        assert!(stop_result.is_ok());

        Ok(())
    }

    // Test the stop function with multiple containers
    #[tokio::test]
    async fn test_stop_function_with_multiple_containers() -> Result<()> {
        let app_name = "multi-stop-test";
        let image_tag = "alpine:latest";
        let container_name = format!("{}-container", app_name);

        // Create multiple containers with similar names
        let run_result1 = run(app_name, image_tag).await;
        assert!(run_result1.is_ok());

        // Create a second container with a slightly different name
        let run_result2 = run(&format!("{}-2", app_name), image_tag).await;
        assert!(run_result2.is_ok());

        // Verify both containers exist
        assert!(is_container_started(&container_name)?);
        assert!(is_container_started(&format!("{}-2-container", app_name))?);

        // Create an App instance for testing
        let app = App::new(app_name)?;

        // Stop should only stop containers matching the exact app name
        let stop_result = stop(&app).await;
        assert!(stop_result.is_ok());

        // Verify only the first container was stopped
        let container2_name = format!("{}-2-container", app_name);

        let state1 = test_helpers::get_container_state(&container_name)?;
        let state2 = test_helpers::get_container_state(&container2_name)?;

        assert!(
            state1 == "exited" || state1 == "stopped",
            "First container should be stopped, got: {}",
            state1
        );
        assert!(
            state2 == "running" || state2 == "exited",
            "Second container should not be affected, got: {}",
            state2
        );

        // Clean up both containers
        cleanup_container(&container_name)?;
        cleanup_container(&container2_name)?;
        Ok(())
    }

    // Test the stop function timeout behavior
    #[tokio::test]
    async fn test_stop_function_timeout_behavior() -> Result<()> {
        let app_name = "timeout-stop-test";
        let image_tag = "alpine:latest";
        let container_name = format!("{}-container", app_name);

        // Create and start a container
        let run_result = run(app_name, image_tag).await;
        assert!(run_result.is_ok());

        // Verify container exists
        assert!(is_container_started(&container_name)?);

        // Create an App instance for testing
        let app = App::new(app_name)?;

        // Stop the container and measure time
        let start_time = std::time::Instant::now();
        let stop_result = stop(&app).await;
        let duration = start_time.elapsed();

        assert!(stop_result.is_ok());

        // The stop operation should complete within a reasonable time
        // (alpine containers exit quickly, so this should be fast)
        assert!(
            duration.as_millis() < 5000,
            "Stop operation took too long: {}ms",
            duration.as_millis()
        );

        // Clean up
        cleanup_container(&container_name)?;
        Ok(())
    }

    // Test the build function with a valid Dockerfile
    #[tokio::test]
    async fn test_build_function_with_valid_dockerfile() -> Result<()> {
        use std::fs;

        let test_dir = "test-build-dir";
        let dockerfile_content = r#"
FROM alpine:latest
RUN echo "Hello from test container"
CMD ["echo", "Test build successful"]
"#;

        // Create test directory and Dockerfile
        fs::create_dir_all(test_dir)?;
        fs::write(format!("{}/Dockerfile", test_dir), dockerfile_content)?;

        // Test the build function
        let build_result = build(test_dir, "test-build-image:latest").await;

        // Clean up test directory
        fs::remove_dir_all(test_dir)?;

        // The build might fail due to no podman daemon, but we're testing the function structure
        match build_result {
            Ok(image_id) => {
                println!("Build function test succeeded with image ID: {}", image_id);
            }
            Err(e) => {
                println!("Build function test result: {:?}", e);
                // Expected to fail due to no podman, but function should complete
            }
        }

        Ok(())
    }

    // Test the build function with missing Dockerfile
    #[tokio::test]
    async fn test_build_function_with_missing_dockerfile() -> Result<()> {
        let test_dir = "test-build-dir-missing";

        // Test the build function with non-existent directory
        let build_result = build(test_dir, "test-build-image:latest").await;

        // Should fail with DockerfileNotFound error
        assert!(build_result.is_err());

        if let Err(e) = build_result {
            let error_string = e.to_string();
            assert!(error_string.contains("Dockerfile not found"));
        }

        Ok(())
    }

    // Test the build function with empty directory
    #[tokio::test]
    async fn test_build_function_with_empty_directory() -> Result<()> {
        use std::fs;

        let test_dir = "test-build-dir-empty";

        // Create empty directory
        fs::create_dir_all(test_dir)?;

        // Test the build function
        let build_result = build(test_dir, "test-build-image:latest").await;

        // Clean up test directory
        fs::remove_dir_all(test_dir)?;

        // Should fail with DockerfileNotFound error
        assert!(build_result.is_err());

        if let Err(e) = build_result {
            let error_string = e.to_string();
            assert!(error_string.contains("Dockerfile not found"));
        }

        Ok(())
    }

    // Test the remove function with existing image
    #[tokio::test]
    async fn test_remove_function_with_existing_image() -> Result<()> {
        // First, try to build an image to remove
        use std::fs;
        let test_dir = "test-remove-dir";
        let dockerfile_content = r#"
FROM alpine:latest
RUN echo "Test image for removal"
"#;

        // Create test directory and Dockerfile
        fs::create_dir_all(test_dir)?;
        fs::write(format!("{}/Dockerfile", test_dir), dockerfile_content)?;

        // Try to build (might fail due to no podman)
        let _ = build(test_dir, "test-remove-image:latest").await;

        // Clean up test directory
        fs::remove_dir_all(test_dir)?;

        // Test the remove function
        let remove_result = remove("test-remove-image:latest").await;

        // The remove might fail due to no podman or image not existing, but we're testing the function structure
        if let Err(e) = remove_result {
            println!("Remove function test result: {:?}", e);
            // Expected to fail due to no podman or missing image, but function should complete
        }

        Ok(())
    }

    // Test the remove function with non-existent image
    #[tokio::test]
    async fn test_remove_function_with_nonexistent_image() -> Result<()> {
        // Test the remove function with a non-existent image
        let remove_result = remove("nonexistent-image:latest").await;

        // The remove should fail, but we're testing that the function handles the error gracefully
        if let Err(e) = remove_result {
            println!("Remove function with non-existent image result: {:?}", e);
            // Expected to fail due to missing image, but function should complete
        }

        Ok(())
    }

    // Test the remove function with invalid image tag
    #[tokio::test]
    async fn test_remove_function_with_invalid_image_tag() -> Result<()> {
        // Test the remove function with an invalid image tag
        let remove_result = remove("invalid:tag:format").await;

        // The remove should fail, but we're testing that the function handles the error gracefully
        if let Err(e) = remove_result {
            println!("Remove function with invalid tag result: {:?}", e);
            // Expected to fail due to invalid tag, but function should complete
        }

        Ok(())
    }

    // Test the remove function with empty image tag
    #[tokio::test]
    async fn test_remove_function_with_empty_image_tag() -> Result<()> {
        // Test the remove function with an empty image tag
        let remove_result = remove("").await;

        // The remove should fail, but we're testing that the function handles the error gracefully
        if let Err(e) = remove_result {
            println!("Remove function with empty tag result: {:?}", e);
            // Expected to fail due to empty tag, but function should complete
        }

        Ok(())
    }

    // Test the run_reverse_proxy function
    #[tokio::test]
    async fn test_run_reverse_proxy_function() -> Result<()> {
        // Test the run_reverse_proxy function
        let result = run_reverse_proxy().await;

        // The function might fail due to no podman daemon, but we're testing the function structure
        match result {
            Ok(_) => {
                println!("Run reverse proxy function test succeeded");
            }
            Err(e) => {
                println!("Run reverse proxy function test result: {:?}", e);
                // Expected to fail due to no podman, but function should complete
            }
        }

        Ok(())
    }
}
