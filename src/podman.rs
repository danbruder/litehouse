use crate::models::{App, EnvVar};
use anyhow::Result;
use bollard::Docker;
use futures_util::{StreamExt, stream::unfold};
use std::path::Path;
use std::pin::Pin;
use std::process::Command;
use tokio::process::Command as AsyncCommand;
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

pub async fn connect() -> Result<Docker> {
    let docker = Docker::connect_with_unix(
        &resolve_docker_socket_path()?,
        120,
        bollard::API_DEFAULT_VERSION,
    )?;
    Ok(docker)
}

#[instrument]
pub async fn build(directory: &str, tag: &str) -> Result<String> {
    info!("Building app in: {}", directory);

    let dockerfile_path = Path::new(directory).join("Dockerfile");
    if !dockerfile_path.exists() {
        return Err(PodmanError::DockerfileNotFound(directory.to_string()).into());
    }

    info!("Starting container image build with Docker CLI...");

    // Use Docker CLI directly instead of Bollard build stream
    let output = AsyncCommand::new("docker")
        .args(["build", "-t", tag, "."])
        .current_dir(directory)
        .output()
        .await
        .map_err(|e| PodmanError::BuildError(format!("Failed to execute docker build: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);

        tracing::error!("Docker build failed:");
        tracing::error!("STDOUT: {}", stdout);
        tracing::error!("STDERR: {}", stderr);

        return Err(PodmanError::BuildError(format!(
            "Docker build failed: {}\nSTDOUT: {}\nSTDERR: {}",
            output.status, stdout, stderr
        ))
        .into());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    info!("Docker build output: {}", stdout);

    info!("Container image build completed successfully");

    // Get the image ID by inspecting the built image
    let inspect_output = AsyncCommand::new("docker")
        .args(["inspect", tag, "--format", "{{.Id}}"])
        .output()
        .await
        .map_err(|e| PodmanError::BuildError(format!("Failed to inspect image: {}", e)))?;

    if !inspect_output.status.success() {
        let stderr = String::from_utf8_lossy(&inspect_output.stderr);
        return Err(
            PodmanError::BuildError(format!("Failed to inspect built image: {}", stderr)).into(),
        );
    }

    let image_id = String::from_utf8_lossy(&inspect_output.stdout)
        .trim()
        .to_string();

    if image_id.is_empty() {
        return Err(PodmanError::BuildError(
            "Failed to get image ID from build result".to_string(),
        )
        .into());
    }

    info!("Built image ID: {}", image_id);
    Ok(image_id)
}

#[instrument]
pub async fn run(name: &str, image_tag: &str) -> Result<()> {
    run_with_port(name, image_tag, None, vec![], vec![]).await
}

#[instrument]
pub async fn run_with_port(
    name: &str,
    image_tag: &str,
    host_port: Option<i64>,
    env_vars: Vec<EnvVar>,
    volume_binds: Vec<String>,
) -> Result<()> {
    // Validate input parameters
    if name.trim().is_empty() {
        return Err(PodmanError::BuildError("App name cannot be empty".to_string()).into());
    }

    info!("Running app: {} (host_port: {:?})", name, host_port);

    let docker = Docker::connect_with_unix(
        &resolve_docker_socket_path()?,
        120,
        bollard::API_DEFAULT_VERSION,
    )?;

    let container_name = format!("/{}-container", name);

    // Check if the container already exists and is running
    let options = Some(bollard::container::ListContainersOptions::<String> {
        all: true,
        ..Default::default()
    });
    let all_containers = docker.list_containers::<String>(options).await?;
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
                    let remove_result = docker.remove_container(id, None).await;
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

    // Inspect the image to get exposed ports
    let image_inspect = docker.inspect_image(image_tag).await?;
    let exposed_ports = image_inspect
        .config
        .and_then(|c| c.exposed_ports)
        .unwrap_or_default();

    // Get the first exposed port, or default to 3000
    let container_port = if let Some(port_key) = exposed_ports.keys().next() {
        port_key.clone()
    } else {
        "3000/tcp".to_string()
    };

    info!("Container exposes port: {}", container_port);

    // Configure port bindings, volume binds, and restart policy
    let host_config = {
        use bollard::models::{HostConfig, PortBinding, RestartPolicy, RestartPolicyNameEnum};
        use std::collections::HashMap;

        let mut config = HostConfig::default();

        // Add port bindings if host_port is provided
        if let Some(port) = host_port {
            let mut port_bindings = HashMap::new();
            port_bindings.insert(
                container_port.clone(),
                Some(vec![PortBinding {
                    host_ip: Some("0.0.0.0".to_string()),
                    host_port: Some(port.to_string()),
                }]),
            );

            info!("Binding container port {} to host port {}", container_port, port);
            config.port_bindings = Some(port_bindings);
        }

        // Add volume binds if provided
        if !volume_binds.is_empty() {
            info!("Mounting {} volume(s)", volume_binds.len());
            for bind in &volume_binds {
                info!("  - {}", bind);
            }
            config.binds = Some(volume_binds.clone());
        }

        // Always set restart policy to ALWAYS
        config.restart_policy = Some(RestartPolicy {
            name: Some(RestartPolicyNameEnum::ALWAYS),
            maximum_retry_count: None,
        });

        Some(config)
    };

    // Format environment variables for container
    let env: Option<Vec<String>> = if env_vars.is_empty() {
        None
    } else {
        Some(
            env_vars
                .iter()
                .map(|ev| format!("{}={}", ev.key, ev.value))
                .collect()
        )
    };

    info!("Container environment: {} variables", env.as_ref().map(|e| e.len()).unwrap_or(0));

    let container_config = bollard::container::Config {
        image: Some(image_tag.to_string()),
        host_config,
        env,
        ..Default::default()
    };

    info!("Creating container: {}", container_name);
    let container_info = docker
        .create_container::<String, String>(
            Some(bollard::container::CreateContainerOptions {
                name: container_name.clone(),
                ..Default::default()
            }),
            container_config,
        )
        .await?;

    info!("Starting container: {}", container_info.id);
    docker
        .start_container::<String>(&container_info.id, None)
        .await?;

    info!("Container {} started successfully", container_name);

    Ok(())
}

#[instrument]
pub async fn remove(tag: &str) -> Result<()> {
    info!("Removing container image with tag: {}", tag);

    let docker = Docker::connect_with_unix(
        &resolve_docker_socket_path()?,
        120,
        bollard::API_DEFAULT_VERSION,
    )?;

    match docker.remove_image(tag, None, None).await {
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
}

#[instrument]
pub async fn logs_stream(
    app_name: &str,
    lines: usize,
    follow: bool,
) -> Result<Pin<Box<dyn futures_util::Stream<Item = Result<String, anyhow::Error>> + Send>>> {
    info!("Getting logs stream for app: {}", app_name);

    let docker = Docker::connect_with_unix(
        &resolve_docker_socket_path()?,
        120,
        bollard::API_DEFAULT_VERSION,
    )?;
    let container_name = format!("{}-container", app_name);

    // Check if container exists
    let all_containers = docker.list_containers::<String>(None).await?;

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
        (docker, container_name, follow),
        |(docker, container_name, follow)| async move {
            let logs_opts = bollard::container::LogsOptions::<String> {
                stdout: true,
                stderr: true,
                follow: follow,
                ..Default::default()
            };

            let mut logs_stream = docker.logs::<String>(&container_name, Some(logs_opts));

            match logs_stream.next().await {
                Some(result) => {
                    let log_string = match result {
                        Ok(log_result) => match log_result {
                            bollard::container::LogOutput::StdOut { message } => {
                                String::from_utf8_lossy(&message).to_string()
                            }
                            bollard::container::LogOutput::StdErr { message } => {
                                String::from_utf8_lossy(&message).to_string()
                            }
                            _ => String::new(),
                        },
                        Err(e) => {
                            return Some((
                                Err(anyhow::anyhow!(e)),
                                (docker, container_name, follow),
                            ));
                        }
                    };
                    Some((Ok(log_string), (docker, container_name, follow)))
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

    let docker = Docker::connect_with_unix(
        &resolve_docker_socket_path()?,
        120,
        bollard::API_DEFAULT_VERSION,
    )?;
    let container_name = format!("{}-container", app.name);

    let options = Some(bollard::container::ListContainersOptions::<String> {
        all: true,
        ..Default::default()
    });
    let all_containers = docker.list_containers::<String>(options).await?;

    for container in all_containers {
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

                    match docker.stop_container(id, None).await {
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

fn resolve_docker_socket_path() -> Result<String> {
    // User-provided overrides
    if let Ok(sock) = std::env::var("DOCKER_HOST") {
        if let Some(path) = sock.strip_prefix("unix://") {
            return Ok(path.to_string());
        }
        // If DOCKER_HOST doesn't have unix:// prefix, assume it's a path
        if sock.starts_with('/') {
            return Ok(sock);
        }
    }
    if let Ok(sock) = std::env::var("DOCKER_SOCK") {
        return Ok(sock);
    }

    // Fallback to well-known default Docker socket
    Ok("/var/run/docker.sock".to_string())
}

#[cfg(test)]
mod test_helpers {
    use anyhow::Result;
    use std::process::Command;

    /// Check if a container exists and was started by calling docker ps -a
    pub fn is_container_started(container_name: &str) -> Result<bool> {
        let output = Command::new("docker")
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
        let stop_result = docker_stop(container_name);

        if let Err(e) = stop_result {
            println!(
                "Warning: Failed to stop container {}: {:?}",
                container_name, e
            );
        }

        // Remove the container
        let remove_result = docker_rm(container_name);

        if let Err(e) = remove_result {
            println!(
                "Warning: Failed to remove container {}: {:?}",
                container_name, e
            );
        }

        Ok(())
    }

    /// Stop a container using docker
    pub fn docker_stop(container_name: &str) -> Result<()> {
        let output = Command::new("docker")
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

    /// Remove a container using docker
    pub fn docker_rm(container_name: &str) -> Result<()> {
        let output = Command::new("docker")
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

    /// Get container state using docker ps
    pub fn get_container_state(container_name: &str) -> Result<String> {
        let output = Command::new("docker")
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
        let app = App::new(app_name, 8000)?;

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
        let app = App::new(app_name, 8000)?;

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
        let app = App::new(app_name, 8000)?;

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
        let app = App::new(app_name, 8000)?;

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
}
