use crate::config;
use crate::db::app as db_app;
use crate::db::system_config as db_system_config;
use crate::models::S3Config;
use anyhow::Result;
use bollard::container::Config;
use bollard::models::{HostConfig, RestartPolicy, RestartPolicyNameEnum};
use bollard::Docker;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sqlx::Pool;
use sqlx::Sqlite;
use std::fs;
use tracing::{error, info, instrument, warn};

#[derive(Debug, thiserror::Error)]
pub enum LitestreamError {
    #[error("Config error: {0}")]
    ConfigError(String),
    #[error("Container error: {0}")]
    ContainerError(String),
}

// Litestream YAML Configuration Structures
#[derive(Serialize, Deserialize)]
struct LitestreamConfig {
    dbs: Vec<DatabaseConfig>,
}

#[derive(Serialize, Deserialize)]
struct DatabaseConfig {
    path: String,
    replicas: Vec<ReplicaConfig>,
}

#[derive(Serialize, Deserialize)]
struct ReplicaConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
}

/// Start the Litestream backup container
/// This function should be called with a database pool to fetch S3 config
#[instrument]
pub async fn start_with_pool(docker: &Docker, db_pool: &Pool<Sqlite>) -> Result<()> {
    info!("Ensuring Litestream backup container is running");

    let container_name = "litestream-container";
    let image_name = "litestream/litestream";

    // Get S3 config from database
    let s3_config = db_system_config::get_s3_config(db_pool)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to fetch S3 config: {}", e))?;

    // Step 1: Ensure image exists
    ensure_image_exists(docker, image_name).await?;

    // Step 2: Handle container state
    let container_state = get_container_state(docker, container_name).await?;

    match container_state {
        ContainerState::NotExists => {
            info!("Container doesn't exist, creating new one");
            create_and_start_container(docker, container_name, image_name, s3_config.as_ref())
                .await?;
        }
        ContainerState::Running { id: _ } => {
            info!("Container is already running");
        }
        ContainerState::Stopped { id } => {
            info!("Container is stopped, starting it");
            start_existing_container(docker, &id).await?;
        }
        ContainerState::Paused { id } => {
            info!("Container is paused, unpausing");
            docker.unpause_container(&id).await?;
        }
        ContainerState::Error { id } | ContainerState::Exited { id } => {
            info!("Container is in error/exited state, removing and recreating");
            remove_container(docker, &id).await?;
            create_and_start_container(docker, container_name, image_name, s3_config.as_ref())
                .await?;
        }
        ContainerState::Restarting { id } => {
            info!("Container is restarting, waiting for it to stabilize");
            wait_for_container_stable(docker, &id).await?;
        }
    }

    info!("Litestream backup container is now running successfully");
    Ok(())
}

/// Legacy start function without database pool (for backwards compatibility)
/// This will start Litestream without S3 configuration
#[instrument]
pub async fn start(docker: &Docker) -> Result<()> {
    info!("Starting Litestream backup container without S3 config");

    let container_name = "litestream-container";
    let image_name = "litestream/litestream";

    // Step 1: Ensure image exists
    ensure_image_exists(docker, image_name).await?;

    // Step 2: Handle container state
    let container_state = get_container_state(docker, container_name).await?;

    match container_state {
        ContainerState::NotExists => {
            info!("Container doesn't exist, creating new one");
            create_and_start_container(docker, container_name, image_name, None).await?;
        }
        ContainerState::Running { id: _ } => {
            info!("Container is already running");
        }
        ContainerState::Stopped { id } => {
            info!("Container is stopped, starting it");
            start_existing_container(docker, &id).await?;
        }
        ContainerState::Paused { id } => {
            info!("Container is paused, unpausing");
            docker.unpause_container(&id).await?;
        }
        ContainerState::Error { id } | ContainerState::Exited { id } => {
            info!("Container is in error/exited state, removing and recreating");
            remove_container(docker, &id).await?;
            create_and_start_container(docker, container_name, image_name, None).await?;
        }
        ContainerState::Restarting { id } => {
            info!("Container is restarting, waiting for it to stabilize");
            wait_for_container_stable(docker, &id).await?;
        }
    }

    info!("Litestream backup container is now running successfully");
    Ok(())
}

/// Generate and sync the Litestream configuration for all apps
#[instrument]
pub async fn sync_configuration(docker: &Docker, db_pool: &Pool<Sqlite>) -> Result<()> {
    info!("Syncing Litestream configuration");

    // Get all apps from database
    let apps = db_app::get_all(db_pool)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to fetch apps: {}", e))?;

    // Get S3 config from database
    let s3_config = db_system_config::get_s3_config(db_pool)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to fetch S3 config: {}", e))?;

    // Generate configuration
    let config = generate_config(&apps, s3_config.as_ref())?;

    // Write config to file
    write_config_file(&config)?;

    info!("Litestream configuration synced for {} app(s)", apps.len());

    // Restart Litestream container to reload config with new S3 settings
    let container_name = "litestream-container";
    if let ContainerState::Running { id } = get_container_state(docker, container_name).await? {
        info!("Restarting Litestream container to reload configuration");

        // Remove the old container and recreate with updated environment variables
        remove_container(docker, &id).await?;
        let image_name = "litestream/litestream";
        create_and_start_container(docker, container_name, image_name, s3_config.as_ref()).await?;
    }

    Ok(())
}

/// Check if database exists, restore from S3 if missing
#[instrument]
pub async fn restore_if_needed(
    docker: &Docker,
    db_pool: &Pool<Sqlite>,
    app_id: &str,
    volume_name: &str,
) -> Result<()> {
    info!("Checking if database restore is needed for app {}", app_id);

    // Check if database already exists in the volume
    let db_exists = check_db_exists(docker, volume_name).await?;

    if db_exists {
        info!("Database already exists in volume, skipping restore");
        return Ok(());
    }

    info!("Database does not exist, attempting restore from S3");

    // Get S3 config from database
    let s3_config = db_system_config::get_s3_config(db_pool).await?;

    if s3_config.is_none() {
        info!("No S3 config found, skipping restore (fresh database)");
        return Ok(());
    }

    let s3_config = s3_config.unwrap();

    // Attempt to restore the database
    match restore_database(docker, app_id, volume_name, &s3_config).await {
        Ok(_) => {
            info!("Database restored successfully from S3");
            Ok(())
        }
        Err(e) => {
            // If restore fails (e.g., no backup exists), log warning but continue
            // This allows new apps to start without a backup
            warn!("Database restore failed (this is OK for new apps): {}", e);
            Ok(())
        }
    }
}

/// Check if app.db exists in volume using Alpine test container
async fn check_db_exists(docker: &Docker, volume_name: &str) -> Result<bool> {
    info!("Checking if database exists in volume: {}", volume_name);

    let container_name = format!("litehouse-check-db-{}", volume_name);

    let container_config = Config {
        image: Some("alpine:latest".to_string()),
        cmd: Some(vec![
            "test".to_string(),
            "-f".to_string(),
            "/data/app.db".to_string(),
        ]),
        host_config: Some(HostConfig {
            binds: Some(vec![format!("{}:/data:ro", volume_name)]),
            auto_remove: Some(true),
            ..Default::default()
        }),
        ..Default::default()
    };

    // Create the container
    let create_options = bollard::container::CreateContainerOptions {
        name: container_name.clone(),
        ..Default::default()
    };

    let container_info = docker
        .create_container(Some(create_options), container_config)
        .await?;

    // Start the container
    docker
        .start_container::<String>(&container_info.id, None)
        .await?;

    // Wait for container to finish
    let timeout = tokio::time::Duration::from_secs(10);
    let start_time = tokio::time::Instant::now();

    loop {
        if start_time.elapsed() > timeout {
            return Err(anyhow::anyhow!("Check DB container timed out"));
        }

        let container = docker.inspect_container(&container_info.id, None).await?;

        if let Some(state) = container.state {
            if let Some(running) = state.running {
                if !running {
                    // Container has finished
                    if let Some(exit_code) = state.exit_code {
                        // Exit code 0 means file exists, 1 means it doesn't
                        return Ok(exit_code == 0);
                    }
                }
            }
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }
}

/// Run one-shot Litestream restore: litestream restore -replica <s3_url> /data/app.db
async fn restore_database(
    docker: &Docker,
    app_id: &str,
    volume_name: &str,
    s3_config: &S3Config,
) -> Result<()> {
    info!("Restoring database for app {} from S3", app_id);

    // Build S3 URL for the app database
    let path_prefix = s3_config.path_prefix.as_deref().unwrap_or("litehouse");
    let s3_url = build_s3_url(
        s3_config,
        &format!("{}/apps/{}/app.db", path_prefix, app_id),
    );

    info!("Restoring from S3 URL: {}", s3_url);

    let container_name = format!("litehouse-restore-{}", app_id);

    // Prepare environment variables for S3 configuration
    let mut env_vars = vec![
        format!("LITESTREAM_ACCESS_KEY_ID={}", s3_config.access_key_id),
        format!(
            "LITESTREAM_SECRET_ACCESS_KEY={}",
            s3_config.secret_access_key
        ),
        format!("AWS_ACCESS_KEY_ID={}", s3_config.access_key_id),
        format!("AWS_SECRET_ACCESS_KEY={}", s3_config.secret_access_key),
        format!("AWS_REGION={}", s3_config.region),
    ];

    if let Some(endpoint) = &s3_config.endpoint {
        env_vars.push(format!("AWS_ENDPOINT_URL={}", endpoint));
    }

    let container_config = Config {
        image: Some("litestream/litestream:latest".to_string()),
        cmd: Some(vec![
            "restore".to_string(),
            "-replica".to_string(),
            s3_url,
            "/data/app.db".to_string(),
        ]),
        env: Some(env_vars),
        host_config: Some(HostConfig {
            binds: Some(vec![format!("{}:/data", volume_name)]),
            auto_remove: Some(true),
            ..Default::default()
        }),
        ..Default::default()
    };

    // Ensure Litestream image exists
    ensure_image_exists(docker, "litestream/litestream").await?;

    // Create the container
    let create_options = bollard::container::CreateContainerOptions {
        name: container_name.clone(),
        ..Default::default()
    };

    let container_info = docker
        .create_container(Some(create_options), container_config)
        .await?;

    info!("Created restore container: {}", container_info.id);

    // Start the container
    docker
        .start_container::<String>(&container_info.id, None)
        .await?;

    // Wait for container to finish (with timeout)
    let timeout = tokio::time::Duration::from_secs(300); // 5 minutes
    let start_time = tokio::time::Instant::now();

    loop {
        if start_time.elapsed() > timeout {
            return Err(anyhow::anyhow!(
                "Restore container timed out after 5 minutes"
            ));
        }

        let container = docker.inspect_container(&container_info.id, None).await?;

        if let Some(state) = container.state {
            if let Some(running) = state.running {
                if !running {
                    // Container has finished
                    if let Some(exit_code) = state.exit_code {
                        if exit_code == 0 {
                            info!("Database restored successfully");
                            return Ok(());
                        } else {
                            // Get container logs for error details
                            let logs = docker
                                .logs::<String>(
                                    &container_info.id,
                                    Some(bollard::container::LogsOptions {
                                        stdout: true,
                                        stderr: true,
                                        tail: "all".to_string(),
                                        ..Default::default()
                                    }),
                                )
                                .collect::<Vec<_>>()
                                .await;

                            let log_output: String = logs
                                .into_iter()
                                .filter_map(|r| r.ok())
                                .map(|log| log.to_string())
                                .collect::<Vec<_>>()
                                .join("");

                            return Err(anyhow::anyhow!(
                                "Restore container failed with exit code {}: {}",
                                exit_code,
                                log_output
                            ));
                        }
                    }
                }
            }
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    }
}

fn generate_config(
    apps: &[crate::models::App],
    s3_config: Option<&S3Config>,
) -> Result<LitestreamConfig> {
    let data_dir = config::get_data_dir()
        .map_err(|e| anyhow::anyhow!("Failed to get data directory: {}", e))?;

    let replicas_base_dir = data_dir.join("litestream-replicas");
    if !replicas_base_dir.exists() {
        fs::create_dir_all(&replicas_base_dir)
            .map_err(|e| anyhow::anyhow!("Failed to create replicas directory: {}", e))?;
    }

    let mut dbs = Vec::new();

    // Add main litehouse database (mounted from config directory)
    let main_db_path = "/config/litehouse.db";

    let mut main_replicas = vec![];

    // Add S3 replica for main database if configured
    if let Some(s3) = s3_config {
        let path_prefix = s3.path_prefix.as_deref().unwrap_or("litehouse");
        let s3_url = build_s3_url(s3, &format!("{}/main/db", path_prefix));
        main_replicas.push(ReplicaConfig {
            path: None,
            url: Some(s3_url),
        });
    }

    dbs.push(DatabaseConfig {
        path: main_db_path.to_string(),
        replicas: main_replicas,
    });

    // Add app databases - UPDATED for per-app volumes
    for app in apps {
        // Database path inside the litestream container
        // Volume mounted at /apps/{app_id} read-only
        let db_path = format!("/apps/{}/app.db", app.id);
        let mut replicas = vec![];

        // Add S3 replica if configured
        if let Some(s3) = s3_config {
            let path_prefix = s3.path_prefix.as_deref().unwrap_or("litehouse");
            // Use app.id for consistent paths
            let s3_url = build_s3_url(s3, &format!("{}/apps/{}/app.db", path_prefix, app.id));
            replicas.push(ReplicaConfig {
                path: None,
                url: Some(s3_url),
            });
        }

        dbs.push(DatabaseConfig {
            path: db_path,
            replicas,
        });
    }

    Ok(LitestreamConfig { dbs })
}

/// Build S3 URL for Litestream replica
fn build_s3_url(s3_config: &S3Config, path: &str) -> String {
    if let Some(endpoint) = &s3_config.endpoint {
        // S3-compatible service with custom endpoint
        format!(
            "s3://{}:{}@{}/{}",
            s3_config.access_key_id, s3_config.secret_access_key, endpoint, path
        )
    } else {
        // Standard AWS S3
        format!("s3://{}/{}", s3_config.bucket, path)
    }
}

fn write_config_file(config: &LitestreamConfig) -> Result<()> {
    let data_dir = config::get_data_dir()
        .map_err(|e| anyhow::anyhow!("Failed to get data directory: {}", e))?;

    let config_path = data_dir.join("litestream.yml");
    let yaml = serde_yaml::to_string(config)
        .map_err(|e| anyhow::anyhow!("Failed to serialize config: {}", e))?;

    fs::write(&config_path, yaml)
        .map_err(|e| anyhow::anyhow!("Failed to write config file: {}", e))?;

    info!("Wrote Litestream config to {}", config_path.display());
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

async fn ensure_image_exists(docker: &Docker, image_name: &str) -> Result<()> {
    info!("Ensuring Litestream image exists");

    let image_list = docker.list_images::<String>(None).await?;

    let image_exists = image_list.iter().any(|image| {
        image
            .repo_tags
            .iter()
            .any(|tag| tag.starts_with(image_name))
    });

    if !image_exists {
        info!("Pulling Litestream image");
        let pull_stream = docker.create_image(
            Some(bollard::image::CreateImageOptions {
                from_image: image_name,
                ..Default::default()
            }),
            None,
            None,
        );

        let mut stream = pull_stream;
        while let Some(result) = stream.next().await {
            match result {
                Ok(report) => {
                    if let Some(status) = report.status {
                        info!("Pull status: {}", status);
                    }
                }
                Err(e) => {
                    error!("Error pulling image: {}", e);
                    return Err(e.into());
                }
            }
        }
        info!("Litestream image pulled successfully");
    } else {
        info!("Litestream image already exists");
    }

    Ok(())
}

async fn get_container_state(docker: &Docker, container_name: &str) -> Result<ContainerState> {
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

/// Ensure litehouse Docker volumes exist
pub async fn ensure_litehouse_volumes_exist(docker: &Docker) -> Result<()> {
    info!("Ensuring litehouse volumes exist");

    let volume_list = docker.list_volumes::<String>(None).await?;
    let existing_volumes: std::collections::HashSet<String> = volume_list
        .volumes
        .unwrap_or_default()
        .into_iter()
        .map(|v| v.name)
        .collect();

    let volume_names = ["litehouse_config", "litehouse_data"];

    for volume_name in volume_names.iter() {
        if !existing_volumes.contains(*volume_name) {
            info!("Creating volume: {}", volume_name);
            docker
                .create_volume(bollard::volume::CreateVolumeOptions {
                    name: volume_name.to_string(),
                    ..Default::default()
                })
                .await?;
        } else {
            info!("Volume {} already exists", volume_name);
        }
    }

    Ok(())
}

async fn create_and_start_container(
    docker: &Docker,
    container_name: &str,
    image_name: &str,
    s3_config: Option<&S3Config>,
) -> Result<()> {
    info!("Creating new Litestream container: {}", container_name);

    // Ensure volumes exist
    ensure_litehouse_volumes_exist(docker).await?;

    // Get config file path from host (we'll copy it into the volume)
    let data_dir = config::get_data_dir()
        .map_err(|e| anyhow::anyhow!("Failed to get data directory: {}", e))?;
    let config_file_path = data_dir.join("litestream.yml");

    // Create empty config if it doesn't exist
    if !config_file_path.exists() {
        write_config_file(&LitestreamConfig { dbs: vec![] })?;
    }

    // Prepare environment variables for S3 configuration
    let mut env_vars = Vec::new();
    if let Some(s3) = s3_config {
        env_vars.push(format!("LITESTREAM_ACCESS_KEY_ID={}", s3.access_key_id));
        env_vars.push(format!(
            "LITESTREAM_SECRET_ACCESS_KEY={}",
            s3.secret_access_key
        ));

        // AWS credentials (some apps might check these)
        env_vars.push(format!("AWS_ACCESS_KEY_ID={}", s3.access_key_id));
        env_vars.push(format!("AWS_SECRET_ACCESS_KEY={}", s3.secret_access_key));
        env_vars.push(format!("AWS_REGION={}", s3.region));

        if let Some(endpoint) = &s3.endpoint {
            env_vars.push(format!("AWS_ENDPOINT_URL={}", endpoint));
        }

        info!(
            "Configuring Litestream with S3 backup to bucket: {}",
            s3.bucket
        );
    }

    // Get all app volumes to mount dynamically
    let app_volumes = crate::volume::list_app_volumes(docker).await?;

    let mut binds = vec![
        "litehouse_data:/data".to_string(), // Keep for replicas directory
        "litehouse_config:/config".to_string(),
    ];

    // Mount each app volume at /apps/{app_id}
    for (volume_name, app_id) in app_volumes {
        binds.push(format!("{}:/apps/{}:rw", volume_name, app_id));
    }

    info!(
        "Mounting {} app volumes to Litestream container",
        binds.len() - 3
    );

    let container_config = Config {
        image: Some(image_name.to_string()),
        hostname: Some(container_name.to_string()),
        cmd: Some(vec![
            "replicate".to_string(),
            "-config".to_string(),
            "/data/litestream.yml".to_string(),
        ]),
        env: if env_vars.is_empty() {
            None
        } else {
            Some(env_vars)
        },
        host_config: Some(HostConfig {
            restart_policy: Some(RestartPolicy {
                name: Some(RestartPolicyNameEnum::UNLESS_STOPPED),
                maximum_retry_count: None,
            }),
            binds: Some(binds), // Updated binds with all app volumes
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
    start_existing_container(docker, &container_info.id).await
}

async fn start_existing_container(docker: &Docker, container_id: &str) -> Result<()> {
    info!("Starting container: {}", container_id);
    docker.start_container::<String>(container_id, None).await?;
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    info!("Container started successfully");
    Ok(())
}

async fn remove_container(docker: &Docker, container_id: &str) -> Result<()> {
    info!("Removing container: {}", container_id);
    docker
        .remove_container(
            container_id,
            Some(bollard::container::RemoveContainerOptions {
                force: true,
                ..Default::default()
            }),
        )
        .await?;
    info!("Container removed");
    Ok(())
}

async fn wait_for_container_stable(docker: &Docker, container_id: &str) -> Result<()> {
    info!("Waiting for container to stabilize");
    for _ in 0..10 {
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        let inspect = docker.inspect_container(container_id, None).await?;
        if let Some(state) = inspect.state {
            if state.running == Some(true) {
                info!("Container has stabilized and is running");
                return Ok(());
            }
        }
    }
    Err(anyhow::anyhow!("Container did not stabilize in time"))
}
