use crate::config::ClientConfig;
use anyhow::{anyhow, Result};
use bytes::Bytes;
use futures_util::stream::BoxStream;
use futures_util::StreamExt;
use reqwest::Client;
use serde::Deserialize;

pub enum LogStream {
    Lines(BoxStream<'static, anyhow::Result<String>>),
    Full(String),
}

#[derive(Debug, Deserialize)]
pub struct AppInfo {
    pub id: String,
    pub name: String,
    pub state: String,
    pub host: Option<String>,
    pub port: u16,
    pub process_id: Option<u32>,
    pub binary_path: Option<String>,
    pub binary_hash: Option<String>,
}

pub struct ApiClient {
    config: ClientConfig,
    client: Client,
}

impl ApiClient {
    pub fn new(config: ClientConfig) -> Self {
        Self {
            client: Client::new(),
            config,
        }
    }

    pub async fn create_app(&self, app_name: &str) -> Result<()> {
        let response = self
            .client
            .post(&format!("{}/apps", self.config.base_url))
            .json(&serde_json::json!({ "name": app_name }))
            .send()
            .await?;

        if response.status().is_success() {
            println!("App '{}' created successfully", app_name);
            Ok(())
        } else {
            let error = response.text().await?;
            Err(anyhow!("Failed to create app: {}", error))
        }
    }

    pub async fn start_app(&self, app_name: &str) -> Result<()> {
        let response = self
            .client
            .post(&format!("{}/apps/{}/start", self.config.base_url, app_name))
            .send()
            .await?;

        if response.status().is_success() {
            println!("App '{}' started successfully", app_name);
            Ok(())
        } else {
            let err = response.text().await?;
            Err(anyhow!("Failed to start app: {}", err))
        }
    }

    pub async fn stop_app(&self, app_name: &str) -> Result<()> {
        let response = self
            .client
            .post(&format!("{}/apps/{}/stop", self.config.base_url, app_name))
            .send()
            .await?;

        if response.status().is_success() {
            println!("App '{}' stopped successfully", app_name);
            Ok(())
        } else {
            let error = response.text().await?;
            Err(anyhow!(error))
        }
    }

    pub async fn restart_app(&self, app_name: &str) -> Result<()> {
        let response = self
            .client
            .post(&format!(
                "{}/apps/{}/restart",
                self.config.base_url, app_name
            ))
            .send()
            .await?;

        if response.status().is_success() {
            println!("App '{}' restarted successfully", app_name);
            Ok(())
        } else {
            let error = response.text().await?;
            Err(anyhow!("Failed to restart app: {}", error))
        }
    }

    pub async fn delete_app(&self, app_name: &str) -> Result<()> {
        let response = self
            .client
            .delete(&format!("{}/apps/{}", self.config.base_url, app_name))
            .send()
            .await?;

        if response.status().is_success() {
            println!("App '{}' deleted successfully", app_name);
            Ok(())
        } else {
            let error = response.text().await?;
            Err(anyhow!("Failed to delete app: {}", error))
        }
    }

    pub async fn deploy_app(&self, app_name: &str, binary_path: &str) -> Result<()> {
        // Create multipart form
        let form = reqwest::multipart::Form::new()
            .file("binary", binary_path)
            .await
            .map_err(|e| anyhow!("Failed to create multipart form: {}", e))?;

        let response = self
            .client
            .post(&format!(
                "{}/apps/{}/deploy",
                self.config.base_url, app_name
            ))
            .multipart(form)
            .send()
            .await?;

        if response.status().is_success() {
            println!("App '{}' deployed successfully", app_name);
            Ok(())
        } else {
            let error = response.text().await?;
            Err(anyhow!("Failed to deploy app: {}", error))
        }
    }

    pub async fn set_env(
        &self,
        app_name: &str,
        key: &str,
        value: &str,
        delete: bool,
    ) -> Result<()> {
        let response = self
            .client
            .post(&format!("{}/apps/{}/env", self.config.base_url, app_name))
            .json(&serde_json::json!({ "key": key, "value": value, "delete": delete }))
            .send()
            .await?;

        if response.status().is_success() {
            println!("Environment variable set for app '{}'", app_name);
            Ok(())
        } else {
            let error = response.text().await?;
            Err(anyhow!("Failed to set environment variable: {}", error))
        }
    }

    pub async fn get_status(&self, app_name: Option<&str>) -> Result<()> {
        let url = match app_name {
            Some(name) => format!("{}/apps/{}", self.config.base_url, name),
            None => format!("{}/apps", self.config.base_url),
        };

        let response = self.client.get(&url).send().await?;

        if response.status().is_success() {
            let status = response.text().await?;
            println!("Status: {}", status);
            Ok(())
        } else {
            let error = response.text().await?;
            Err(anyhow!("Failed to get status: {}", error))
        }
    }

    pub async fn get_logs(&self, app_name: &str, lines: usize, follow: bool) -> Result<LogStream> {
        let url = format!(
            "{}/apps/{}/logs?lines={}&follow={}",
            self.config.base_url, app_name, lines, follow
        );
        let response = self.client.get(&url).send().await?;
        if response.status().is_success() {
            if follow {
                let stream = response
                    .bytes_stream()
                    .map(|chunk: Result<Bytes, reqwest::Error>| {
                        chunk
                            .map_err(|e| anyhow::anyhow!(e))
                            .map(|bytes| String::from_utf8_lossy(&bytes).to_string())
                    })
                    .boxed();
                Ok(LogStream::Lines(stream))
            } else {
                Ok(LogStream::Full(response.text().await?))
            }
        } else {
            Err(anyhow::anyhow!(
                "Failed to fetch logs: {}",
                response.status()
            ))
        }
    }

    pub async fn get_app_info(&self, app_name: &str) -> Result<AppInfo> {
        let url = format!("{}/apps/{}", self.config.base_url, app_name);
        let response = self.client.get(&url).send().await?;
        if response.status().is_success() {
            let app_info: AppInfo = response.json().await?;
            Ok(app_info)
        } else {
            let error = response.text().await?;
            Err(anyhow!("Failed to get app info: {}", error))
        }
    }

    pub async fn get_podman_version(&self) -> Result<()> {
        let response = self
            .client
            .get(&format!("{}/podman/version", self.config.base_url))
            .send()
            .await?;
        if response.status().is_success() {
            let version = response.text().await?;
            println!("Podman version: {}", version);
            Ok(())
        } else {
            let error = response.text().await?;
            Err(anyhow!("Failed to get podman version: {}", error))
        }
    }
}
