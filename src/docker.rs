use crate::models::{App, EnvVar};
use bollard::auth::DockerCredentials;
use bollard::image::CreateImageOptions;
use bollard::Docker;
use futures_util::StreamExt;
use std::pin::Pin;
use tracing::{info, instrument};

type Result<T> = std::result::Result<T, DockerError>;

#[derive(Debug, thiserror::Error)]
pub enum DockerError {
    #[error("Build error: {0}")]
    BuildError(String),
    #[error("Log error: {0}")]
    LogError(String),
    #[error("Failed to list images: {0}")]
    ListImagesError(String),
    #[error("Pull error: {0}")]
    PullError(String),
    #[error("Bollard error: {0}")]
    BollardError(#[from] bollard::errors::Error),
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("Path strip prefix error: {0}")]
    StripPrefixError(#[from] std::path::StripPrefixError),
}

pub async fn connect() -> Result<Docker> {
    let docker = Docker::connect_with_unix(
        &resolve_docker_socket_path()?,
        120,
        bollard::API_DEFAULT_VERSION,
    )?;
    Ok(docker)
}

/// Get the exposed port from a Docker image
///
/// Inspects the Docker image and returns the first exposed port.
/// Defaults to "3000" if no EXPOSE directive is found.
/// Strips the "/tcp" suffix for a clean port number.
#[instrument]
pub async fn get_exposed_port(image_tag: &str) -> Result<String> {
    let docker = connect().await?;

    let image_inspect = docker.inspect_image(image_tag).await?;
    let exposed_ports = image_inspect
        .config
        .and_then(|c| c.exposed_ports)
        .unwrap_or_default();

    // Get the first exposed port, or default to 3000
    let port = if let Some(port_key) = exposed_ports.keys().next() {
        // Strip the "/tcp" suffix
        port_key.split('/').next().unwrap_or("3000").to_string()
    } else {
        "3000".to_string()
    };

    info!("Detected exposed port {} for image {}", port, image_tag);
    Ok(port)
}

#[instrument]
pub async fn run(
    name: &str,
    image_tag: &str,
    env_vars: Vec<EnvVar>,
    volume_binds: Vec<String>,
) -> Result<()> {
    // Validate input parameters
    if name.trim().is_empty() {
        return Err(DockerError::BuildError("App name cannot be empty".to_string()).into());
    }

    info!("Running app: {}", name);

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

    // Configure volume binds, restart policy, and network mode
    let host_config = {
        use bollard::models::{HostConfig, RestartPolicy, RestartPolicyNameEnum};

        let mut config = HostConfig::default();

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

        // Connect to the litehouse network for inter-container communication
        config.network_mode = Some("litehouse-network".to_string());

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
                .collect(),
        )
    };

    info!(
        "Container environment: {} variables",
        env.as_ref().map(|e| e.len()).unwrap_or(0)
    );

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
) -> Result<Pin<Box<dyn futures_util::Stream<Item = Result<String>> + Send>>> {
    info!("Getting logs stream for app: {}", app_name);

    let docker = Docker::connect_with_unix(
        &resolve_docker_socket_path()?,
        120,
        bollard::API_DEFAULT_VERSION,
    )?;
    let container_name = format!("{}-container", app_name);

    // Check if container exists (include stopped containers)
    let options = Some(bollard::container::ListContainersOptions::<String> {
        all: true,
        ..Default::default()
    });
    let all_containers = docker.list_containers::<String>(options).await?;

    let mut container_found = false;
    for container in all_containers {
        if let Some(names) = &container.names {
            // Use contains() because Docker returns names with leading "/" (e.g., "/app-container")
            if names.iter().any(|n| n.contains(&container_name)) {
                container_found = true;
                break;
            }
        }
    }

    if !container_found {
        return Err(DockerError::LogError(format!(
            "Container '{}' not found for app '{}'",
            container_name, app_name
        )));
    }

    // Create logs options
    let mut logs_opts = bollard::container::LogsOptions::<String> {
        stdout: true,
        stderr: true,
        follow: follow,
        ..Default::default()
    };
    
    // Set tail if lines > 0 (tail expects a String, not Option<String>)
    if lines > 0 {
        logs_opts.tail = lines.to_string();
    }

    // Create the Docker logs stream once
    let logs_stream = docker.logs::<String>(&container_name, Some(logs_opts));

    // Map the stream to extract log messages
    let mapped_stream = logs_stream.map(|result| {
        match result {
            Ok(log_result) => {
                let log_string = match log_result {
                    bollard::container::LogOutput::StdOut { message } => {
                        String::from_utf8_lossy(&message).to_string()
                    }
                    bollard::container::LogOutput::StdErr { message } => {
                        String::from_utf8_lossy(&message).to_string()
                    }
                    _ => String::new(),
                };
                Ok(log_string)
            }
            Err(e) => Err(DockerError::BollardError(e)),
        }
    });

    Ok(Box::pin(mapped_stream))
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
    // User-provided override via environment variable
    if let Ok(sock) = std::env::var("DOCKER_HOST") {
        if let Some(path) = sock.strip_prefix("unix://") {
            info!("Using DOCKER_HOST: {}", path);
            return Ok(path.to_string());
        }
        info!("Using DOCKER_HOST: {}", sock);
        return Ok(sock);
    }

    // Standard Docker socket path
    let docker_sock = "/var/run/docker.sock";
    if std::path::Path::new(docker_sock).exists() {
        info!("Using Docker socket: {}", docker_sock);
        return Ok(docker_sock.to_string());
    }

    // Fallback to default
    info!("Using default Docker socket path: {}", docker_sock);
    Ok(docker_sock.to_string())
}

/// Check if a Docker image with the given tag exists
#[instrument]
pub async fn image_exists(tag: &str) -> Result<bool> {
    let docker = connect().await?;
    let images = docker
        .list_images::<String>(None)
        .await
        .map_err(|e| DockerError::ListImagesError(e.to_string()))?;

    Ok(images.iter().any(|image| {
        image
            .repo_tags
            .iter()
            .any(|t| t == tag)
    }))
}

/// Pull an image. For ghcr.io private images pass a GitHub token with read:packages.
#[instrument(skip(docker, registry_token))]
pub async fn pull(docker: &Docker, image: &str, registry_token: Option<&str>) -> Result<()> {
    let credentials = registry_token.map(|token| DockerCredentials {
        username: Some("litehouse".to_string()), // GHCR accepts any username with a PAT
        password: Some(token.to_string()),
        ..Default::default()
    });
    let options = CreateImageOptions {
        from_image: image,
        ..Default::default()
    };
    let mut stream = docker.create_image(Some(options), None, credentials);
    while let Some(progress) = stream.next().await {
        progress.map_err(|e| DockerError::PullError(format!("pull {image} failed: {e}")))?;
    }
    Ok(())
}

/// Stop and remove the container for `app_name` (i.e. `{app_name}-container`),
/// tolerating the case where it's already stopped or doesn't exist at all
/// (e.g. an app's first deploy). Used by the deploy engine to unconditionally
/// replace a container with a freshly-created one running a new image.
#[instrument(skip(docker))]
pub async fn stop_and_remove_container(docker: &Docker, app_name: &str) -> Result<()> {
    let container_name = format!("{}-container", app_name);

    match docker.stop_container(&container_name, None).await {
        Ok(_) => {
            info!("Stopped container '{}'", container_name);
        }
        Err(e) => {
            // 404 = no such container, 304 = already stopped. Both fine here.
            let msg = e.to_string();
            if msg.contains("404") || msg.contains("304") {
                info!("Container '{}' not running or absent: {}", container_name, msg);
            } else {
                return Err(e.into());
            }
        }
    }

    match docker.remove_container(&container_name, None).await {
        Ok(_) => {
            info!("Removed container '{}'", container_name);
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("404") {
                info!("Container '{}' already absent", container_name);
            } else {
                return Err(e.into());
            }
        }
    }

    Ok(())
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

    /// Remove a container using docker (force-remove so this works even if the
    /// container is running or stuck in a restart loop, e.g. left over from a
    /// previous failed/interrupted test run).
    pub fn docker_rm(container_name: &str) -> Result<()> {
        let output = Command::new("docker")
            .args(["rm", "-f", container_name])
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

    // `run()` always sets the container restart policy to `always` (production
    // behavior, not test-configurable). `alpine:latest`'s default command
    // (`/bin/sh` with no attached stdin) exits immediately, so under a restart
    // policy the container enters a restart loop, and any subsequent Docker
    // API call against it (start/stop/logs/inspect) can race a 409 "container
    // is restarting" error. Tests that need a container to actually stay
    // running use a long-lived default command instead (`redis-server`, which
    // runs in the foreground indefinitely) to sidestep the restart loop
    // entirely. Tests that only exercise error paths (invalid image, empty
    // name, etc.) don't start a real container and are unaffected, so they
    // keep using `alpine:latest`.
    const TEST_IMAGE: &str = "redis:7.4-alpine";

    // Test the happy path: run function creates and starts a container, then verify it's running
    #[tokio::test]
    async fn test_run_function_happy_path() -> Result<()> {
        let app_name = "test-run-app";
        let image_tag = TEST_IMAGE;
        let container_name = format!("{}-container", app_name);

        // Clean up any existing test container first
        let _ = cleanup_container(&container_name);

        // Step 1: Call the run function
        let run_result = run(app_name, image_tag, vec![], vec![]).await;
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
        let image_tag = TEST_IMAGE;
        let container_name = format!("{}-container", app_name);

        // Clean up any leftover container from a previous failed/interrupted run
        let _ = cleanup_container(&container_name);

        // First run: create a container
        let first_run = run(app_name, image_tag, vec![], vec![]).await;
        if let Err(e) = &first_run {
            println!("First run failed with error: {:?}", e);
        }
        assert!(first_run.is_ok());

        // Verify container exists
        assert!(is_container_started(&container_name)?);

        // Second run: should skip startup since container already exists
        let second_run = run(app_name, image_tag, vec![], vec![]).await;
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
        let result = run("invalid-test", "nonexistent-image:latest", vec![], vec![]).await;
        assert!(result.is_err(), "Should fail with invalid image");
        Ok(())
    }

    // Test error handling with empty app name
    #[tokio::test]
    async fn test_run_function_with_empty_app_name() -> Result<()> {
        let result = run("", "alpine:latest", vec![], vec![]).await;
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
        let image_tag = TEST_IMAGE;
        let container_name = format!("{}-container", app_name);

        // Clean up any leftover container from a previous failed/interrupted run
        let _ = cleanup_container(&container_name);

        // Run multiple instances concurrently
        let handles: Vec<_> = (0..3)
            .map(|_| {
                let app = app_name.to_string();
                let image = image_tag.to_string();
                tokio::spawn(async move { run(&app, &image, vec![], vec![]).await })
            })
            .collect();

        // Wait for all to complete
        for handle in handles {
            let result = handle.await?;
            // At least one should succeed, others might fail due to conflicts
            println!("Concurrent run result: {:?}", result);
        }

        // Clean up
        cleanup_container(&container_name)?;
        Ok(())
    }

    // Test cleanup on failure
    #[tokio::test]
    async fn test_run_function_cleanup_on_failure() -> Result<()> {
        let app_name = "cleanup-test";
        let container_name = format!("{}-container", app_name);

        // Try to run with invalid image (should fail)
        let result = run(app_name, "invalid-image:latest", vec![], vec![]).await;
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
        let image_tag = TEST_IMAGE;
        let container_name = format!("{}-container", app_name);

        // Clean up any leftover container from a previous failed/interrupted run
        let _ = cleanup_container(&container_name);

        // First run: create and start a container
        let first_run = run(app_name, image_tag, vec![], vec![]).await;
        assert!(first_run.is_ok());

        // Verify container exists
        assert!(is_container_started(&container_name)?);

        // Wait a moment for the container to be fully registered in podman
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        // Second run: should skip startup and return immediately
        let start_time = std::time::Instant::now();
        let second_run = run(app_name, image_tag, vec![], vec![]).await;
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
        let image_tag = TEST_IMAGE;
        let container_name = format!("{}-container", app_name);

        // Clean up any leftover container from a previous failed/interrupted run
        let _ = cleanup_container(&container_name);

        // First, create and start a container
        let run_result = run(app_name, image_tag, vec![], vec![]).await;
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
        let image_tag = TEST_IMAGE;
        let container_name = format!("{}-container", app_name);
        let container2_name = format!("{}-2-container", app_name);

        // Clean up any leftover containers from a previous failed/interrupted run
        let _ = cleanup_container(&container_name);
        let _ = cleanup_container(&container2_name);

        // Create multiple containers with similar names
        let run_result1 = run(app_name, image_tag, vec![], vec![]).await;
        assert!(run_result1.is_ok());

        // Create a second container with a slightly different name
        let run_result2 = run(&format!("{}-2", app_name), image_tag, vec![], vec![]).await;
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
        let image_tag = TEST_IMAGE;
        let container_name = format!("{}-container", app_name);

        // Clean up any leftover container from a previous failed/interrupted run
        let _ = cleanup_container(&container_name);

        // Create and start a container
        let run_result = run(app_name, image_tag, vec![], vec![]).await;
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
        assert!(
            duration.as_millis() < 5000,
            "Stop operation took too long: {}ms",
            duration.as_millis()
        );

        // Clean up
        cleanup_container(&container_name)?;
        Ok(())
    }

    // Test the remove function with existing image
    #[tokio::test]
    async fn test_remove_function_with_existing_image() -> Result<()> {
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

    // Test that logs_stream doesn't repeat logs (fixes the unfold bug)
    #[tokio::test]
    async fn test_logs_stream_no_duplicates() -> Result<()> {
        use futures_util::StreamExt;
        use std::collections::HashSet;
        use std::time::Duration;
        use tokio::time::timeout;

        let app_name = "test-logs-app";
        let image_tag = TEST_IMAGE;
        let container_name = format!("{}-container", app_name);

        // Clean up any existing test container first
        let _ = cleanup_container(&container_name);

        // Create and start a container that will produce some logs
        run(app_name, image_tag, vec![], vec![]).await?;

        // Wait a moment for container to start
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Generate some unique log lines by running commands in the container
        let docker = connect().await?;
        let exec_config = bollard::exec::CreateExecOptions {
            cmd: Some(vec!["sh", "-c", "echo 'log-line-1'; echo 'log-line-2'; echo 'log-line-3'"]),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            ..Default::default()
        };

        // Execute commands to generate logs
        let exec_id = docker
            .create_exec(&container_name, exec_config)
            .await?
            .id;
        docker.start_exec(&exec_id, None).await?;

        // Wait for logs to be written
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Get logs stream
        let mut stream = logs_stream(app_name, 10, false).await?;

        // Collect all log lines
        let mut log_lines = Vec::new();
        let mut seen_lines = HashSet::new();

        // Read logs with timeout to avoid hanging
        let timeout_duration = Duration::from_secs(5);
        loop {
            match timeout(timeout_duration, stream.next()).await {
                Ok(Some(Ok(line))) => {
                    let trimmed = line.trim().to_string();
                    if !trimmed.is_empty() {
                        log_lines.push(trimmed.clone());
                        seen_lines.insert(trimmed);
                    }
                }
                Ok(Some(Err(e))) => {
                    // Log error but continue - might be end of stream
                    tracing::debug!("Log stream error: {}", e);
                    break;
                }
                Ok(None) => {
                    // Stream ended
                    break;
                }
                Err(_) => {
                    // Timeout - assume stream is done
                    break;
                }
            }
        }

        // Verify we got some logs
        assert!(!log_lines.is_empty(), "Should receive at least some log lines");

        // Verify no duplicates: each line should only appear once
        let unique_lines: HashSet<String> = log_lines.iter().cloned().collect();
        assert_eq!(
            log_lines.len(),
            unique_lines.len(),
            "Log stream should not contain duplicates. Got {} total lines but only {} unique lines. Logs: {:?}",
            log_lines.len(),
            unique_lines.len(),
            log_lines
        );

        // Verify each line in seen_lines appears exactly once in log_lines
        for line in &seen_lines {
            let count = log_lines.iter().filter(|l| l.as_str() == line.as_str()).count();
            assert_eq!(
                count, 1,
                "Log line '{}' should appear exactly once, but appeared {} times",
                line, count
            );
        }

        // Clean up
        cleanup_container(&container_name)?;

        Ok(())
    }

    // Test that logs_stream handles non-existent container gracefully
    #[tokio::test]
    async fn test_logs_stream_nonexistent_container() {
        let result = logs_stream("nonexistent-app", 10, false).await;
        assert!(result.is_err(), "Should return error for non-existent container");
        
        if let Err(DockerError::LogError(msg)) = result {
            assert!(msg.contains("not found"), "Error message should mention 'not found', got: {}", msg);
        } else {
            panic!("Expected LogError for non-existent container");
        }
    }

    // Test pulling a public image from Docker Hub with no registry credentials
    #[tokio::test]
    async fn test_pull_public_image() {
        let docker = connect().await.unwrap();
        pull(&docker, "alpine:3.20", None).await.unwrap();
        assert!(image_exists("alpine:3.20").await.unwrap());
    }
}
