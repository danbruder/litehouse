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
use tracing::{error, info, instrument};

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
            create_and_start_container(docker, container_name, image_name, s3_config.as_ref()).await?;
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
            create_and_start_container(docker, container_name, image_name, s3_config.as_ref()).await?;
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

fn generate_config(apps: &[crate::models::App], s3_config: Option<&S3Config>) -> Result<LitestreamConfig> {
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
    let main_replica_path = "/data/litestream-replicas/main";

    let mut main_replicas = vec![ReplicaConfig {
        path: Some(main_replica_path.to_string()),
        url: None,
    }];

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

    // Add app databases
    for app in apps {
        // Database path inside the container
        let db_path = format!("/data/apps/{}/data/app.db", app.name);
        let replica_path = format!("/data/litestream-replicas/{}", app.name);

        let mut replicas = vec![ReplicaConfig {
            path: Some(replica_path),
            url: None,
        }];

        // Add S3 replica if configured
        if let Some(s3) = s3_config {
            let path_prefix = s3.path_prefix.as_deref().unwrap_or("litehouse");
            let s3_url = build_s3_url(s3, &format!("{}/{}/db", path_prefix, app.name));
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
        format!("s3://{}:{}@{}/{}",
            s3_config.access_key_id,
            s3_config.secret_access_key,
            endpoint,
            path)
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

async fn create_and_start_container(
    docker: &Docker,
    container_name: &str,
    image_name: &str,
    s3_config: Option<&S3Config>,
) -> Result<()> {
    info!("Creating new Litestream container: {}", container_name);

    let data_dir = config::get_data_dir()
        .map_err(|e| anyhow::anyhow!("Failed to get data directory: {}", e))?;

    let config_dir = config::get_config_dir()
        .map_err(|e| anyhow::anyhow!("Failed to get config directory: {}", e))?;

    let config_file_path = data_dir.join("litestream.yml");

    // Create empty config if it doesn't exist
    if !config_file_path.exists() {
        write_config_file(&LitestreamConfig { dbs: vec![] })?;
    }

    // Prepare environment variables for S3 configuration
    let mut env_vars = Vec::new();
    if let Some(s3) = s3_config {
        env_vars.push(format!("LITESTREAM_ACCESS_KEY_ID={}", s3.access_key_id));
        env_vars.push(format!("LITESTREAM_SECRET_ACCESS_KEY={}", s3.secret_access_key));

        // AWS credentials (some apps might check these)
        env_vars.push(format!("AWS_ACCESS_KEY_ID={}", s3.access_key_id));
        env_vars.push(format!("AWS_SECRET_ACCESS_KEY={}", s3.secret_access_key));
        env_vars.push(format!("AWS_REGION={}", s3.region));

        if let Some(endpoint) = &s3.endpoint {
            env_vars.push(format!("AWS_ENDPOINT_URL={}", endpoint));
        }

        info!("Configuring Litestream with S3 backup to bucket: {}", s3.bucket);
    }

    let container_config = Config {
        image: Some(image_name.to_string()),
        hostname: Some(container_name.to_string()),
        cmd: Some(vec![
            "replicate".to_string(),
            "-config".to_string(),
            "/etc/litestream.yml".to_string(),
        ]),
        env: if env_vars.is_empty() { None } else { Some(env_vars) },
        host_config: Some(HostConfig {
            restart_policy: Some(RestartPolicy {
                name: Some(RestartPolicyNameEnum::ALWAYS),
                maximum_retry_count: None,
            }),
            binds: Some(vec![
                format!("{}:/data", data_dir.display()),
                format!("{}:/config", config_dir.display()),
                format!("{}:/etc/litestream.yml", config_file_path.display()),
            ]),
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
