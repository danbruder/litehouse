use crate::config::ServerConfig;
use crate::db::app as db_app;
use crate::models::App;
use anyhow::Result;
use bollard::container::Config;
use bollard::models::{HostConfig, PortBinding, RestartPolicy, RestartPolicyNameEnum};
use bollard::Docker;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sqlx::Pool;
use sqlx::Sqlite;
use std::collections::HashMap;
use tracing::{error, info, instrument};

#[derive(Debug, thiserror::Error)]
pub enum DockerError {
    #[error("Dockerfile not found in directory: {0}")]
    DockerfileNotFound(String),
    #[error("Build error: {0}")]
    BuildError(String),
    #[error("Build stream error: {0}")]
    BuildStreamError(String),
    #[error("Log error: {0}")]
    LogError(String),
}

// Caddy JSON Configuration Structures
#[derive(Serialize, Deserialize)]
struct CaddyConfig {
    apps: CaddyApps,
}

#[derive(Serialize, Deserialize)]
struct CaddyApps {
    http: HttpApp,
}

#[derive(Serialize, Deserialize)]
struct HttpApp {
    servers: HashMap<String, Server>,
}

#[derive(Serialize, Deserialize)]
struct Server {
    listen: Vec<String>,
    routes: Vec<Route>,
    #[serde(skip_serializing_if = "Option::is_none")]
    automatic_https: Option<AutomaticHttps>,
}

#[derive(Serialize, Deserialize)]
struct AutomaticHttps {
    disable: bool,
}

#[derive(Serialize, Deserialize)]
struct Route {
    #[serde(rename = "match")]
    match_rules: Vec<HostMatcher>,
    handle: Vec<Handler>,
}

#[derive(Serialize, Deserialize)]
struct HostMatcher {
    host: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct Handler {
    handler: String,
    upstreams: Vec<Upstream>,
}

#[derive(Serialize, Deserialize)]
struct Upstream {
    dial: String,
}

#[instrument]
pub async fn start(docker: &Docker, config: &ServerConfig) -> Result<()> {
    info!("Ensuring Caddy reverse proxy is running");

    let container_name = "caddy-container";
    let image_name = "caddy";
    let caddy_data_volume = "caddy_data";
    let caddy_config_volume = "caddy_config";
    let network_name = "litehouse-network";

    // Step 0: Ensure the litehouse network exists
    ensure_network_exists(docker, network_name).await?;

    // Step 1: Ensure volumes exist
    ensure_volumes_exist(docker, caddy_data_volume, caddy_config_volume).await?;

    // Step 2: Ensure image exists
    ensure_image_exists(docker, image_name).await?;

    // Step 3: Handle container state
    let container_state = get_container_state(docker, container_name).await?;

    match container_state {
        ContainerState::NotExists => {
            info!("Container doesn't exist, creating new one");
            create_and_start_container(
                docker,
                container_name,
                image_name,
                caddy_data_volume,
                caddy_config_volume,
                config,
            )
            .await?;
        }
        ContainerState::Running { id } => {
            info!("Container is already running, verifying health and port configuration");
            if !verify_container_health(docker, &id).await? {
                info!("Container is unhealthy, restarting");
                restart_container(docker, &id).await?;
            } else if !verify_container_ports(docker, &id, config).await? {
                info!("Container is running but on wrong ports, removing and recreating");
                remove_container(docker, &id).await?;
                create_and_start_container(
                    docker,
                    container_name,
                    image_name,
                    caddy_data_volume,
                    caddy_config_volume,
                    config,
                )
                .await?;
            } else {
                info!("Container is healthy and running on correct ports");
            }
        }
        ContainerState::Stopped { id } => {
            info!("Container is stopped, starting it");
            start_existing_container(
                docker,
                &id,
                config.caddy_http_port.unwrap_or(80),
                config.caddy_https_port.unwrap_or(443),
            )
            .await?;
        }
        ContainerState::Paused { id } => {
            info!("Container is paused, unpausing and starting");
            unpause_and_start_container(docker, &id, config).await?;
        }
        ContainerState::Error { id } => {
            info!("Container is in error state, removing and recreating");
            remove_container(docker, &id).await?;
            create_and_start_container(
                docker,
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
            wait_for_container_stable(docker, &id).await?;
        }
        ContainerState::Exited { id } => {
            info!("Container has exited, removing and recreating");
            remove_container(docker, &id).await?;
            create_and_start_container(
                docker,
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

async fn ensure_network_exists(docker: &Docker, network_name: &str) -> Result<()> {
    info!("Ensuring Docker network {} exists", network_name);

    let networks = docker.list_networks::<String>(None).await?;
    let network_exists = networks
        .iter()
        .any(|n| n.name.as_deref() == Some(network_name));

    if !network_exists {
        info!("Creating network: {}", network_name);
        docker
            .create_network(bollard::network::CreateNetworkOptions {
                name: network_name,
                driver: "bridge",
                ..Default::default()
            })
            .await?;
    } else {
        info!("Network {} already exists", network_name);
    }

    Ok(())
}

async fn ensure_volumes_exist(
    docker: &Docker,
    caddy_data_volume: &str,
    caddy_config_volume: &str,
) -> Result<()> {
    info!("Ensuring volumes exist");

    let volume_list = docker.list_volumes::<String>(None).await?;
    let existing_volumes: std::collections::HashSet<String> = volume_list
        .volumes
        .unwrap_or_default()
        .into_iter()
        .map(|v| v.name)
        .collect();

    for volume_name in [caddy_data_volume, caddy_config_volume] {
        if !existing_volumes.contains(volume_name) {
            info!("Creating volume: {}", volume_name);
            docker
                .create_volume(bollard::volume::CreateVolumeOptions {
                    name: volume_name,
                    ..Default::default()
                })
                .await?;
        } else {
            info!("Volume {} already exists", volume_name);
        }
    }

    Ok(())
}

async fn ensure_image_exists(docker: &Docker, image_name: &str) -> Result<()> {
    info!("Ensuring Caddy image exists");

    let image_list = docker.list_images::<String>(None).await?;

    let image_exists = image_list.iter().any(|image| {
        image
            .repo_tags
            .iter()
            .any(|tag| tag.starts_with(image_name))
    });

    if !image_exists {
        info!("Pulling Caddy image");
        let pull_stream = docker.create_image(
            Some(bollard::image::CreateImageOptions {
                from_image: image_name,
                ..Default::default()
            }),
            None,
            None,
        );

        // Process the pull stream
        let mut stream = pull_stream;
        while let Some(result) = stream.next().await {
            match result {
                Ok(report) => {
                    if let Some(status) = report.status {
                        info!("Pull status: {}", status);
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

async fn get_container_state(docker: &Docker, container_name: &str) -> Result<ContainerState> {
    // List ALL containers (including stopped/exited ones)
    let list_options = bollard::container::ListContainersOptions::<String> {
        all: true,
        ..Default::default()
    };
    let all_containers = docker.list_containers(Some(list_options)).await?;

    for container in all_containers {
        if let Some(names) = &container.names {
            if names
                .iter()
                .any(|n| n == container_name || n == &format!("/{}", container_name))
            {
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

async fn verify_container_health(docker: &Docker, container_id: &str) -> Result<bool> {
    // Simple health check - try to get container info and check if it's responding
    match docker.inspect_container(container_id, None).await {
        Ok(inspect) => {
            if let Some(state) = inspect.state {
                if let Some(health) = state.health {
                    if let Some(status) = health.status {
                        info!("Container health status: {:?}", status);
                        // EMPTY means no healthcheck configured, treat as healthy if running
                        if matches!(
                            status,
                            bollard::models::HealthStatusEnum::HEALTHY
                                | bollard::models::HealthStatusEnum::EMPTY
                        ) {
                            return Ok(state.running.unwrap_or(false));
                        }
                        return Ok(false);
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

async fn verify_container_ports(
    docker: &Docker,
    container_id: &str,
    config: &ServerConfig,
) -> Result<bool> {
    let expected_http_port = config.caddy_http_port.unwrap_or(80);
    let expected_https_port = config.caddy_https_port.unwrap_or(443);

    info!(
        "Verifying container ports - expected HTTP: {}, HTTPS: {}",
        expected_http_port, expected_https_port
    );

    match docker.inspect_container(container_id, None).await {
        Ok(inspect) => {
            if let Some(network_settings) = inspect.network_settings {
                if let Some(ports) = network_settings.ports {
                    let mut http_port_correct = false;
                    let mut https_port_correct = false;

                    for (container_port, host_bindings) in ports {
                        if let Some(bindings) = host_bindings {
                            for binding in bindings {
                                if let Some(host_port_str) = &binding.host_port {
                                    if let Ok(host_port) = host_port_str.parse::<u16>() {
                                        if container_port == "80/tcp"
                                            && host_port == expected_http_port
                                        {
                                            http_port_correct = true;
                                            info!(
                                                "HTTP port binding correct: {} -> {}",
                                                container_port, host_port
                                            );
                                        } else if container_port == "443/tcp"
                                            && host_port == expected_https_port
                                        {
                                            https_port_correct = true;
                                            info!(
                                                "HTTPS port binding correct: {} -> {}",
                                                container_port, host_port
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }

                    let ports_correct = http_port_correct && https_port_correct;
                    if !ports_correct {
                        info!(
                            "Port validation failed - HTTP correct: {}, HTTPS correct: {}",
                            http_port_correct, https_port_correct
                        );
                    }
                    Ok(ports_correct)
                } else {
                    info!("No port bindings found in container inspection");
                    Ok(false)
                }
            } else {
                info!("No network settings found in container inspection");
                Ok(false)
            }
        }
        Err(e) => {
            tracing::warn!("Failed to inspect container ports: {}", e);
            Ok(false)
        }
    }
}

async fn create_and_start_container(
    docker: &Docker,
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

    // Create port bindings: host port -> container port
    let mut port_bindings = HashMap::new();

    // Bind HTTP port (container port 80 -> host port from config)
    port_bindings.insert(
        "80/tcp".to_string(),
        Some(vec![PortBinding {
            host_ip: Some("0.0.0.0".to_string()),
            host_port: Some(http_port.to_string()),
        }]),
    );

    // Bind HTTPS port (container port 443 -> host port from config)
    port_bindings.insert(
        "443/tcp".to_string(),
        Some(vec![PortBinding {
            host_ip: Some("0.0.0.0".to_string()),
            host_port: Some(https_port.to_string()),
        }]),
    );

    // Bind Caddy admin API port (container port 2019 -> host port 2019)
    port_bindings.insert(
        "2019/tcp".to_string(),
        Some(vec![PortBinding {
            host_ip: Some("0.0.0.0".to_string()),
            host_port: Some("2019".to_string()),
        }]),
    );

    // Create container config with proper port bindings
    let container_config = Config {
        image: Some(image_name.to_string()),
        hostname: Some(container_name.to_string()),
        // Start Caddy in API-only mode for dynamic configuration
        cmd: Some(vec![
            "caddy".to_string(),
            "run".to_string(),
            "--resume".to_string(),
        ]),
        // Configure Caddy admin API to listen on 0.0.0.0 instead of localhost
        env: Some(vec!["CADDY_ADMIN=0.0.0.0:2019".to_string()]),
        host_config: Some(HostConfig {
            port_bindings: Some(port_bindings),
            restart_policy: Some(RestartPolicy {
                name: Some(RestartPolicyNameEnum::UNLESS_STOPPED),
                maximum_retry_count: None,
            }),
            binds: Some(vec![
                format!("{}:/data", caddy_data_volume),
                format!("{}:/config", caddy_config_volume),
            ]),
            // Connect to the litehouse network for inter-container communication
            network_mode: Some("litehouse-network".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    };

    let container_info = docker
        .create_container::<String, String>(
            Some(bollard::container::CreateContainerOptions {
                name: container_name.to_string(),
                ..Default::default()
            }),
            container_config,
        )
        .await?;

    info!("Created container: {}", container_info.id);
    start_existing_container(docker, &container_info.id, http_port, https_port).await
}

async fn start_existing_container(
    docker: &Docker,
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
    docker.start_container::<String>(container_id, None).await?;

    // Wait a moment for the container to start
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    // Verify it's actually running
    if verify_container_health(docker, container_id).await? {
        info!("Container started successfully");
        Ok(())
    } else {
        Err(anyhow::anyhow!("Container failed to start properly"))
    }
}

async fn restart_container(docker: &Docker, container_id: &str) -> Result<()> {
    info!("Restarting container: {}", container_id);
    docker.restart_container(container_id, None).await?;

    // Wait for restart to complete
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    if verify_container_health(docker, container_id).await? {
        info!("Container restarted successfully");
        Ok(())
    } else {
        Err(anyhow::anyhow!("Container failed to restart properly"))
    }
}

async fn unpause_and_start_container(
    docker: &Docker,
    container_id: &str,
    config: &ServerConfig,
) -> Result<()> {
    info!("Unpausing container: {}", container_id);
    docker.unpause_container(container_id).await?;

    // Wait a moment then check if it needs to be started
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    start_existing_container(
        docker,
        container_id,
        config.caddy_http_port.unwrap_or(80),
        config.caddy_https_port.unwrap_or(443),
    )
    .await
}

async fn remove_container(docker: &Docker, container_id: &str) -> Result<()> {
    info!("Removing container: {}", container_id);

    // First try to stop the container if it's running
    match docker.stop_container(container_id, None).await {
        Ok(_) => {
            info!("Successfully stopped container: {}", container_id);
        }
        Err(e) => {
            // If the container is already stopped, that's fine
            if e.to_string().contains("304") {
                info!("Container {} was already stopped", container_id);
            } else {
                info!("Failed to stop container {}: {}", container_id, e);
                // Continue with removal anyway, it might still work
            }
        }
    }

    // Wait a moment for the stop to complete
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // Now remove the container
    docker.remove_container(container_id, None).await?;
    info!("Successfully removed container: {}", container_id);
    Ok(())
}

async fn wait_for_container_stable(docker: &Docker, container_id: &str) -> Result<()> {
    info!("Waiting for container to stabilize: {}", container_id);

    // Wait up to 30 seconds for the container to become stable
    for _ in 0..30 {
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

        if verify_container_health(docker, container_id).await? {
            info!("Container is now stable");
            return Ok(());
        }
    }

    Err(anyhow::anyhow!(
        "Container failed to stabilize within 30 seconds"
    ))
}

// Caddy Configuration Management Functions

/// Build Caddy JSON configuration from apps
async fn build_caddy_config(
    apps: Vec<App>,
    local_dev: bool,
    domain: Option<&str>,
) -> Result<CaddyConfig> {
    let mut routes = Vec::new();

    // Add admin API route
    let admin_host = if local_dev {
        // Always add admin.localhost for local dev
        Some("admin.localhost".to_string())
    } else {
        // In production, only add if domain is configured
        domain.map(|d| format!("admin.{}", d))
    };

    if let Some(host) = admin_host {
        // Use container name for Docker network routing (works on Linux servers)
        // The litehouse-server container and caddy-container are on the same Docker network
        let admin_route = Route {
            match_rules: vec![HostMatcher { host: vec![host] }],
            handle: vec![Handler {
                handler: "reverse_proxy".to_string(),
                upstreams: vec![Upstream {
                    dial: "litehouse-server:3030".to_string(),
                }],
            }],
        };

        routes.push(admin_route);
    }

    // Add app routes
    for app in apps {
        // Apps without a deployed image yet have nothing to route to.
        if app.image.is_none() {
            info!("Skipping app '{}' - no deployed image found", app.name);
            continue;
        }

        // Get exposed port cached on the app record. Apps without an exposed
        // port have nothing to route to and are skipped.
        let port = match app.exposed_port.clone() {
            Some(p) => p,
            None => {
                info!(
                    "Skipping app '{}' - no exposed port configured",
                    app.name
                );
                continue;
            }
        };

        let host = if local_dev {
            // For local development, use .localhost domains
            format!("{}.localhost", app.name)
        } else {
            // For production, use the domain pattern
            match domain {
                Some(domain_str) => format!("{}.{}", app.name, domain_str),
                None => {
                    anyhow::bail!(
                        "server domain not configured — run lh install --domain <domain>"
                    )
                }
            }
        };

        // Use container name for Docker network routing (works on Linux servers)
        // Apps run in containers named {app-name}-container on the same Docker network
        let upstream = format!("{}-container:{}", app.name, port);

        let route = Route {
            match_rules: vec![HostMatcher { host: vec![host] }],
            handle: vec![Handler {
                handler: "reverse_proxy".to_string(),
                upstreams: vec![Upstream { dial: upstream }],
            }],
        };

        routes.push(route);
    }

    let mut servers = HashMap::new();
    // Caddy always listens on :80/:443 inside the container.
    // The container port mapping handles exposing on different host ports
    // (e.g., 9090/9443 for local dev, 80/443 for production).
    let listen_ports = vec![":80".to_string(), ":443".to_string()];

    // Disable automatic HTTPS for local dev (no valid certs for .localhost)
    let automatic_https = if local_dev {
        Some(AutomaticHttps { disable: true })
    } else {
        None
    };

    servers.insert(
        "app_proxy".to_string(),
        Server {
            listen: listen_ports,
            routes,
            automatic_https,
        },
    );

    Ok(CaddyConfig {
        apps: CaddyApps {
            http: HttpApp { servers },
        },
    })
}

/// Update Caddy configuration via API
async fn update_caddy_config(_docker: &Docker, config: CaddyConfig) -> Result<()> {
    // Use native HTTP client instead of docker exec to avoid dependency on curl in container
    let client = reqwest::Client::new();
    // The litehouse server runs on the host, so use localhost to reach Caddy's admin API.
    // Use CADDY_API_URL env var to override if needed.
    let caddy_api_url = std::env::var("CADDY_API_URL")
        .unwrap_or_else(|_| "http://localhost:2019/load".to_string());

    info!("Sending configuration to Caddy API at {}", caddy_api_url);

    let response = client
        .post(caddy_api_url)
        .header("Content-Type", "application/json")
        .json(&config)
        .send()
        .await?;

    let status = response.status();

    if status.is_success() {
        info!(
            "Caddy configuration updated successfully (status: {})",
            status
        );
        Ok(())
    } else {
        let error_body = response
            .text()
            .await
            .unwrap_or_else(|_| "Unable to read response body".to_string());
        error!("Caddy API returned error status {}: {}", status, error_body);
        Err(anyhow::anyhow!(
            "Caddy configuration update failed with status {}: {}",
            status,
            error_body
        ))
    }
}

/// Synchronize Caddy configuration with database apps
#[instrument(skip(docker, db_pool))]
pub async fn sync_configuration(docker: &Docker, db_pool: &Pool<Sqlite>) -> Result<()> {
    info!("Synchronizing Caddy configuration with database apps");

    // Detect if we're running in local development mode
    let local_dev = std::env::var("LITEHOUSE_LOCAL_DEV").is_ok() || cfg!(debug_assertions);

    // Load server config to get domain
    let server_config = ServerConfig::load().unwrap_or_default();
    let domain = server_config.domain.as_deref();

    match db_app::get_all(db_pool).await {
        Ok(apps) => {
            info!("Found {} apps", apps.len());

            let config = build_caddy_config(apps, local_dev, domain).await?;
            update_caddy_config(docker, config).await?;

            info!("Caddy configuration synchronized successfully");
        }
        Err(e) => {
            error!("Failed to get apps from database: {}", e);
            return Err(e.into());
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deployed_app(name: &str, exposed_port: Option<&str>) -> App {
        let mut app = App::new(name).expect("valid app name");
        app.image = Some(format!("{}:latest", name));
        app.exposed_port = exposed_port.map(|p| p.to_string());
        app
    }

    #[tokio::test]
    async fn routes_from_exposed_port_with_configured_domain() {
        let apps = vec![deployed_app("myapp", Some("8080"))];

        let config = build_caddy_config(apps, false, Some("s.danbruder.com"))
            .await
            .expect("config should build");

        let json = serde_json::to_string(&config).expect("serialize config");

        assert!(
            json.contains("myapp.s.danbruder.com"),
            "expected host myapp.s.danbruder.com in config: {json}"
        );
        assert!(
            json.contains("myapp-container:8080"),
            "expected upstream myapp-container:8080 in config: {json}"
        );
    }

    #[tokio::test]
    async fn errors_when_domain_missing_and_not_local_dev() {
        let apps = vec![deployed_app("myapp", Some("8080"))];

        let result = build_caddy_config(apps, false, None).await;

        match result {
            Ok(_) => panic!("expected error when domain is not configured"),
            Err(e) => assert!(
                e.to_string().contains("server domain not configured"),
                "error message should mention missing domain configuration: {e}"
            ),
        }
    }

    #[tokio::test]
    async fn local_dev_routes_use_localhost_without_domain() {
        let apps = vec![deployed_app("myapp", Some("8080"))];

        let config = build_caddy_config(apps, true, None)
            .await
            .expect("local dev should not require a domain");

        let json = serde_json::to_string(&config).expect("serialize config");
        assert!(json.contains("myapp.localhost"), "config: {json}");
    }

    #[tokio::test]
    async fn skips_apps_without_exposed_port() {
        let apps = vec![deployed_app("myapp", None)];

        let config = build_caddy_config(apps, true, None)
            .await
            .expect("config should build");

        let json = serde_json::to_string(&config).expect("serialize config");
        assert!(
            !json.contains("myapp-container"),
            "app without exposed_port should not get a route: {json}"
        );
    }
}
