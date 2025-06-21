use anyhow::{anyhow, Result};
use sqlx::{Pool, Sqlite};
use std::{path::PathBuf, process::Command};
use tracing::{info, instrument};

use crate::config;
use crate::models::App;
use crate::providers::{Handle, Provider};

pub struct PodmanProvider {}

#[derive(Debug)]
pub struct PodmanHandle {
    container_id: String,
}

impl Handle for PodmanHandle {
    fn id(&self) -> u32 {
        // Get container PID from podman inspect
        let output = Command::new("podman")
            .args(["inspect", "-f", "{{.State.Pid}}", &self.container_id])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0);
        output
    }
}

impl Provider for PodmanProvider {
    type Handle = PodmanHandle;

    #[instrument(skip(self, pool))]
    async fn setup(&self, pool: &Pool<Sqlite>, app: &App) -> anyhow::Result<App> {
        // Create app directory
        let _app_dir = config::get_app_dir(&app.name)?;

        // Get next available port
        let port = config::get_next_available_port(pool).await?;
        let app = app.with_port(port);

        Ok(app)
    }

    #[instrument(skip(self))]
    async fn teardown(&self, app: &App) -> anyhow::Result<App> {
        // Stop and remove container if it exists
        let _ = Command::new("podman").args(["stop", &app.name]).status();

        let _ = Command::new("podman").args(["rm", &app.name]).status();

        // Remove app directory
        let app_dir = config::get_app_dir(&app.name)?;
        std::fs::remove_dir_all(&app_dir)?;

        Ok(app.clone())
    }

    #[instrument(skip(self, app))]
    async fn start(&self, app: &App) -> anyhow::Result<PodmanHandle> {
        let binary_path = app
            .binary_path
            .as_ref()
            .ok_or_else(|| anyhow!("No binary path"))?;

        let app_dir = config::get_app_dir(&app.name)?;
        let data_dir = config::get_app_data_dir(&app.name)?;

        // Build container command
        let mut cmd = Command::new("podman");
        cmd.args([
            "run",
            "--name",
            &app.name,
            "--rm",     // Remove container when it stops
            "--detach", // Run in background
            "--publish",
            &format!("{}:{}", app.port.unwrap_or(0), app.port.unwrap_or(0)),
            "--volume",
            &format!("{}:/app/data", data_dir.to_string_lossy()),
            "--volume",
            &format!("{}:/app/logs", app_dir.join("logs").to_string_lossy()),
        ]);

        // Add environment variables
        cmd.env("PORT", app.port.map_or("".to_string(), |p| p.to_string()));
        cmd.env("APP_NAME", &app.name);
        cmd.env("DATA_DIR", "/app/data");
        for (key, value) in &app.environment {
            cmd.env(key, value);
        }

        // Use the binary as the container command
        cmd.arg(binary_path);

        // Start container
        let output = cmd.output()?;
        if !output.status.success() {
            return Err(anyhow!(
                "Failed to start container: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        // Get container ID
        let container_id = String::from_utf8(output.stdout)?.trim().to_string();

        info!("Started app '{}' in container {}", app.name, container_id);

        Ok(PodmanHandle { container_id })
    }
}

impl PodmanProvider {
    pub async fn get_status(&self, container_id: &str) -> Result<String> {
        let output = Command::new("podman")
            .args(["inspect", "-f", "{{.State.Status}}", container_id])
            .output()?;

        if !output.status.success() {
            return Err(anyhow!("Container not found"));
        }

        Ok(String::from_utf8(output.stdout)?.trim().to_string())
    }

    pub async fn get_logs(&self, container_id: &str, lines: Option<usize>) -> Result<String> {
        let mut cmd = Command::new("podman");
        cmd.arg("logs");

        if let Some(n) = lines {
            cmd.args(["--tail", &n.to_string()]);
        }

        cmd.arg(container_id);

        let output = cmd.output()?;
        if !output.status.success() {
            return Err(anyhow!("Failed to get logs"));
        }

        Ok(String::from_utf8(output.stdout)?)
    }

    pub async fn restart(&self, container_id: &str) -> Result<()> {
        let status = Command::new("podman")
            .args(["restart", container_id])
            .status()?;

        if !status.success() {
            return Err(anyhow!("Failed to restart container"));
        }

        Ok(())
    }

    pub async fn stop(&self, container_id: &str) -> Result<()> {
        let status = Command::new("podman")
            .args(["stop", container_id])
            .status()?;

        if !status.success() {
            return Err(anyhow!("Failed to stop container"));
        }

        Ok(())
    }
}

pub async fn get_podman_version() -> Result<String> {
    let output = Command::new("podman").args(["version"]).output()?;
    if !output.status.success() {
        let err = String::from_utf8(output.stderr)?;
        return Err(anyhow!("Failed to get podman version: {}", err));
    }
    Ok(String::from_utf8(output.stdout)?)
}

pub async fn build_image(path: &PathBuf, app_name: &str) -> Result<()> {
    let output = Command::new("podman")
        .args(["build", "-t", app_name, path.to_string_lossy().as_ref()])
        .output()?;

    if !output.status.success() {
        return Err(anyhow!("Failed to build image"));
    }
    Ok(())
}
