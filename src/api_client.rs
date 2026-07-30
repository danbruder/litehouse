use crate::backup::{BackupReport, RestoreReport};
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
pub struct CreateAppResult {
    pub id: String,
    pub name: String,
    pub state: String,
    pub deploy_token: String,
    pub url: String,
}

#[derive(Debug, Deserialize, serde::Serialize)]
pub struct DeployResult {
    pub status: String,
    pub deploy_id: String,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize, Clone, serde::Serialize)]
pub struct DeployListItem {
    pub id: String,
    pub image: String,
    pub git_sha: Option<String>,
    pub status: String,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
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

#[derive(Debug, Deserialize)]
pub struct BackupStatus {
    pub last_backup_date: Option<String>,
    pub last_backup_report: Option<BackupReport>,
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

    /// Get the current API token from config (reloads from disk)
    fn get_api_token(&self) -> Result<Option<String>> {
        let config = ClientConfig::load()?;
        Ok(config.api_token)
    }

    /// Get Authorization header value if a token exists
    fn get_auth_header(&self) -> Result<Option<String>> {
        if let Some(token) = self.get_api_token()? {
            Ok(Some(format!("Bearer {}", token)))
        } else {
            Ok(None)
        }
    }

    /// Helper to execute a request with the admin token attached
    async fn execute_request<F, T>(&self, build_request: F) -> Result<T>
    where
        F: FnOnce(&Client, Option<String>) -> reqwest::RequestBuilder,
        T: serde::de::DeserializeOwned,
    {
        self.execute_request_with_response(build_request, |r| {
            Box::pin(async move {
                r.json::<T>().await.map_err(|e| anyhow!("Failed to parse JSON: {}", e))
            })
        }).await
    }

    /// Helper to execute a request that returns text
    async fn execute_request_text<F>(&self, build_request: F) -> Result<String>
    where
        F: FnOnce(&Client, Option<String>) -> reqwest::RequestBuilder,
    {
        self.execute_request_with_response(build_request, |r| {
            Box::pin(async move {
                r.text().await.map_err(|e| anyhow!("Failed to read response: {}", e))
            })
        }).await
    }

    /// Generic helper to execute a request with the admin token attached
    async fn execute_request_with_response<F, G, T>(&self, build_request: F, process_response: G) -> Result<T>
    where
        F: FnOnce(&Client, Option<String>) -> reqwest::RequestBuilder,
        G: FnOnce(reqwest::Response) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<T>> + Send>>,
    {
        let auth_header = self.get_auth_header()?;
        let request = build_request(&self.client, auth_header);
        let response = request.send().await?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(anyhow!(
                "Authentication required or invalid token. Run 'lh connect <base-url> --token <token>' to configure the admin token."
            ));
        }

        if response.status().is_success() {
            process_response(response).await
        } else {
            let error = response.text().await.unwrap_or_else(|_| "Request failed".to_string());
            Err(anyhow!("Request failed: {}", error))
        }
    }

    /// Create a new app, optionally linking it to a GitHub `repo`
    /// ("owner/name"). If `rotate_token` is true and the app already
    /// exists, a fresh deploy token is minted and returned instead of
    /// erroring (idempotent-create).
    pub async fn create_app(
        &self,
        app_name: &str,
        repo: Option<&str>,
        rotate_token: bool,
    ) -> Result<CreateAppResult> {
        let url = format!("{}/apps", self.config.base_url);
        let payload = serde_json::json!({ "name": app_name, "repo": repo, "rotate_token": rotate_token });

        self.execute_request(|client, auth_header| {
            let mut req = client.post(&url).json(&payload);
            if let Some(header) = auth_header {
                req = req.header("Authorization", header);
            }
            req
        }).await
    }

    pub async fn start_app(&self, app_name: &str) -> Result<()> {
        let url = format!("{}/apps/{}/start", self.config.base_url, app_name);

        self.execute_request_text(|client, auth_header| {
            let mut req = client.post(&url);
            if let Some(header) = auth_header {
                req = req.header("Authorization", header);
            }
            req
        }).await?;

        Ok(())
    }

    pub async fn stop_app(&self, app_name: &str) -> Result<()> {
        let url = format!("{}/apps/{}/stop", self.config.base_url, app_name);

        self.execute_request_text(|client, auth_header| {
            let mut req = client.post(&url);
            if let Some(header) = auth_header {
                req = req.header("Authorization", header);
            }
            req
        }).await?;

        Ok(())
    }

    pub async fn delete_app(&self, app_name: &str) -> Result<()> {
        let url = format!("{}/apps/{}", self.config.base_url, app_name);

        self.execute_request_text(|client, auth_header| {
            let mut req = client.delete(&url);
            if let Some(header) = auth_header {
                req = req.header("Authorization", header);
            }
            req
        }).await?;

        Ok(())
    }

    /// Trigger an admin redeploy of `app_name` to `image` (and optional git
    /// sha) via the admin API. This is the same deploy engine the public
    /// GitHub deploy hook uses.
    pub async fn deploy_app(
        &self,
        app_name: &str,
        image: &str,
        sha: Option<&str>,
    ) -> Result<DeployResult> {
        let url = format!("{}/apps/{}/deploy", self.config.base_url, app_name);
        let payload = serde_json::json!({ "image": image, "sha": sha });

        let auth_header = self.get_auth_header()?;
        let mut request = self.client.post(&url).json(&payload);
        if let Some(header) = &auth_header {
            request = request.header("Authorization", header);
        }

        let response = request.send().await?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(anyhow!(
                "Authentication required or invalid token. Run 'lh connect <base-url> --token <token>' to configure the admin token."
            ));
        }

        // 200 (succeeded) and 502 (failed, but with a structured body) both
        // carry a DeployResult; anything else is an unexpected failure.
        if response.status().is_success() || response.status() == reqwest::StatusCode::BAD_GATEWAY
        {
            response
                .json::<DeployResult>()
                .await
                .map_err(|e| anyhow!("Failed to parse deploy response: {}", e))
        } else {
            let error = response.text().await.unwrap_or_default();
            Err(anyhow!("Failed to deploy app: {}", error))
        }
    }

    /// List recent deploys for an app, newest first.
    pub async fn list_deploys(&self, app_name: &str, limit: u32) -> Result<Vec<DeployListItem>> {
        let url = format!(
            "{}/apps/{}/deploys?limit={}",
            self.config.base_url, app_name, limit
        );

        self.execute_request(|client, auth_header| {
            let mut req = client.get(&url);
            if let Some(header) = auth_header {
                req = req.header("Authorization", header);
            }
            req
        }).await
    }

    pub async fn set_env(
        &self,
        app_name: &str,
        key: &str,
        value: &str,
        delete: bool,
    ) -> Result<()> {
        let url = format!("{}/apps/{}/env", self.config.base_url, app_name);
        let payload = serde_json::json!({ "key": key, "value": value, "delete": delete });

        self.execute_request_text(|client, auth_header| {
            let mut req = client.post(&url).json(&payload);
            if let Some(header) = auth_header {
                req = req.header("Authorization", header);
            }
            req
        }).await?;

        Ok(())
    }

    /// Add a custom top-level domain to an app's Caddy route.
    pub async fn add_domain(&self, app_name: &str, domain: &str) -> Result<()> {
        let url = format!("{}/apps/{}/domains", self.config.base_url, app_name);
        let payload = serde_json::json!({ "domain": domain });

        self.execute_request_text(|client, auth_header| {
            let mut req = client.post(&url).json(&payload);
            if let Some(header) = auth_header {
                req = req.header("Authorization", header);
            }
            req
        }).await?;

        Ok(())
    }

    /// Remove a custom top-level domain from an app's Caddy route.
    pub async fn remove_domain(&self, app_name: &str, domain: &str) -> Result<()> {
        let url = format!("{}/apps/{}/domains", self.config.base_url, app_name);
        let payload = serde_json::json!({ "domain": domain });

        self.execute_request_text(|client, auth_header| {
            let mut req = client.delete(&url).json(&payload);
            if let Some(header) = auth_header {
                req = req.header("Authorization", header);
            }
            req
        }).await?;

        Ok(())
    }

    /// List an app's custom top-level domains.
    pub async fn list_domains(&self, app_name: &str) -> Result<Vec<String>> {
        let url = format!("{}/apps/{}/domains", self.config.base_url, app_name);

        self.execute_request(|client, auth_header| {
            let mut req = client.get(&url);
            if let Some(header) = auth_header {
                req = req.header("Authorization", header);
            }
            req
        }).await
    }

    /// Set (or replace) an app's HTTP health check path.
    pub async fn set_health_check(&self, app_name: &str, path: &str) -> Result<()> {
        let url = format!("{}/apps/{}/health-check", self.config.base_url, app_name);
        let payload = serde_json::json!({ "path": path });

        self.execute_request_text(|client, auth_header| {
            let mut req = client.post(&url).json(&payload);
            if let Some(header) = auth_header {
                req = req.header("Authorization", header);
            }
            req
        }).await?;

        Ok(())
    }

    /// Clear an app's HTTP health check path.
    pub async fn unset_health_check(&self, app_name: &str) -> Result<()> {
        let url = format!("{}/apps/{}/health-check", self.config.base_url, app_name);

        self.execute_request_text(|client, auth_header| {
            let mut req = client.delete(&url);
            if let Some(header) = auth_header {
                req = req.header("Authorization", header);
            }
            req
        }).await?;

        Ok(())
    }

    /// Get an app's configured HTTP health check path, if any.
    pub async fn get_health_check(&self, app_name: &str) -> Result<Option<String>> {
        let url = format!("{}/apps/{}/health-check", self.config.base_url, app_name);

        self.execute_request(|client, auth_header| {
            let mut req = client.get(&url);
            if let Some(header) = auth_header {
                req = req.header("Authorization", header);
            }
            req
        }).await
    }

    /// Fetch the status JSON for one app (`Some(name)`) or all apps
    /// (`None`). Returns the server's raw response body.
    pub async fn get_status_json(&self, app_name: Option<&str>) -> Result<String> {
        let url = match app_name {
            Some(name) => format!("{}/apps/{}", self.config.base_url, name),
            None => format!("{}/apps", self.config.base_url),
        };

        self.execute_request_text(|client, auth_header| {
            let mut req = client.get(&url);
            if let Some(header) = auth_header {
                req = req.header("Authorization", header);
            }
            req
        }).await
    }

    pub async fn get_logs(&self, app_name: &str, lines: usize, follow: bool) -> Result<LogStream> {
        let url = format!(
            "{}/apps/{}/logs?lines={}&follow={}",
            self.config.base_url, app_name, lines, follow
        );

        let auth_header = self.get_auth_header()?;
        let mut request = self.client.get(&url);
        if let Some(header) = &auth_header {
            request = request.header("Authorization", header);
        }

        let response = request.send().await?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(anyhow!(
                "Authentication required or invalid token. Run 'lh connect <base-url> --token <token>' to configure the admin token."
            ));
        }

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
        self.execute_request(|client, auth_header| {
            let mut req = client.get(&url);
            if let Some(header) = auth_header {
                req = req.header("Authorization", header);
            }
            req
        }).await
    }

    pub async fn get_docker_version(&self) -> Result<()> {
        let url = format!("{}/docker/version", self.config.base_url);
        let version = self.execute_request_text(|client, auth_header| {
            let mut req = client.get(&url);
            if let Some(header) = auth_header {
                req = req.header("Authorization", header);
            }
            req
        }).await?;

        println!("Docker version: {}", version);
        Ok(())
    }

    pub async fn set_s3_config(
        &self,
        access_key_id: &str,
        secret_access_key: &str,
        bucket: &str,
        region: &str,
        endpoint: Option<&str>,
        path_prefix: Option<&str>,
    ) -> Result<()> {
        let url = format!("{}/config/s3", self.config.base_url);
        let payload = serde_json::json!({
            "access_key_id": access_key_id,
            "secret_access_key": secret_access_key,
            "bucket": bucket,
            "region": region,
            "endpoint": endpoint,
            "path_prefix": path_prefix,
        });

        self.execute_request_text(|client, auth_header| {
            let mut req = client.post(&url).json(&payload);
            if let Some(header) = auth_header {
                req = req.header("Authorization", header);
            }
            req
        }).await?;

        println!("S3 configuration saved successfully");
        println!("Daily backups will be uploaded to S3 bucket: {}", bucket);
        Ok(())
    }

    pub async fn get_s3_config(&self) -> Result<()> {
        let url = format!("{}/config/s3", self.config.base_url);

        let auth_header = self.get_auth_header()?;
        let mut request = self.client.get(&url);
        if let Some(header) = &auth_header {
            request = request.header("Authorization", header);
        }

        let response = request.send().await?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(anyhow!(
                "Authentication required or invalid token. Run 'lh connect <base-url> --token <token>' to configure the admin token."
            ));
        }

        if response.status().is_success() {
            let config: serde_json::Value = response.json().await?;
            println!("Current S3 Configuration:");
            println!("{}", serde_json::to_string_pretty(&config)?);
            Ok(())
        } else if response.status().as_u16() == 404 {
            println!("No S3 configuration found");
            Ok(())
        } else {
            let error = response.text().await?;
            Err(anyhow!("Failed to get S3 config: {}", error))
        }
    }

    pub async fn delete_s3_config(&self) -> Result<()> {
        let url = format!("{}/config/s3", self.config.base_url);

        self.execute_request_text(|client, auth_header| {
            let mut req = client.delete(&url);
            if let Some(header) = auth_header {
                req = req.header("Authorization", header);
            }
            req
        }).await?;

        println!("S3 configuration deleted successfully");
        Ok(())
    }

    pub async fn set_ghcr_token(&self, token: &str) -> Result<()> {
        let url = format!("{}/config/ghcr", self.config.base_url);
        let payload = serde_json::json!({ "token": token });

        self.execute_request_text(|client, auth_header| {
            let mut req = client.post(&url).json(&payload);
            if let Some(header) = auth_header {
                req = req.header("Authorization", header);
            }
            req
        }).await?;

        println!("GHCR token saved successfully");
        Ok(())
    }

    pub async fn get_ghcr_token(&self) -> Result<()> {
        let url = format!("{}/config/ghcr", self.config.base_url);

        let auth_header = self.get_auth_header()?;
        let mut request = self.client.get(&url);
        if let Some(header) = &auth_header {
            request = request.header("Authorization", header);
        }

        let response = request.send().await?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(anyhow!(
                "Authentication required or invalid token. Run 'lh connect <base-url> --token <token>' to configure the admin token."
            ));
        }

        if response.status().is_success() {
            let config: serde_json::Value = response.json().await?;
            println!("{}", serde_json::to_string_pretty(&config)?);
            Ok(())
        } else {
            let error = response.text().await?;
            Err(anyhow!("Failed to get GHCR token config: {}", error))
        }
    }

    pub async fn delete_ghcr_token(&self) -> Result<()> {
        let url = format!("{}/config/ghcr", self.config.base_url);

        self.execute_request_text(|client, auth_header| {
            let mut req = client.delete(&url);
            if let Some(header) = auth_header {
                req = req.header("Authorization", header);
            }
            req
        }).await?;

        println!("GHCR token deleted successfully");
        Ok(())
    }

    /// `POST /backups/run` — run a full backup now and return its report.
    pub async fn run_backup(&self) -> Result<BackupReport> {
        let url = format!("{}/backups/run", self.config.base_url);

        self.execute_request(|client, auth_header| {
            let mut req = client.post(&url);
            if let Some(header) = auth_header {
                req = req.header("Authorization", header);
            }
            req
        })
        .await
    }

    /// `GET /backups/status` — last recorded backup date + report, without
    /// triggering a new run.
    pub async fn backup_status(&self) -> Result<BackupStatus> {
        let url = format!("{}/backups/status", self.config.base_url);

        self.execute_request(|client, auth_header| {
            let mut req = client.get(&url);
            if let Some(header) = auth_header {
                req = req.header("Authorization", header);
            }
            req
        })
        .await
    }

    /// `POST /restore` — run a full disaster-recovery restore from S3 and
    /// return its report.
    pub async fn restore(&self) -> Result<RestoreReport> {
        let url = format!("{}/restore", self.config.base_url);

        self.execute_request(|client, auth_header| {
            let mut req = client.post(&url);
            if let Some(header) = auth_header {
                req = req.header("Authorization", header);
            }
            req
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;
    use tempfile::TempDir;

    fn setup_test_dirs() -> (TempDir, TempDir) {
        let data_dir = tempfile::tempdir().unwrap();
        let config_dir = tempfile::tempdir().unwrap();
        let _ = config::set_test_dirs(data_dir.path().to_path_buf(), config_dir.path().to_path_buf());
        (data_dir, config_dir)
    }

    #[test]
    fn test_get_auth_header_with_token() {
        let (_data_dir, _config_dir) = setup_test_dirs();
        let mut config = ClientConfig::default();
        config.api_token = Some("test-token".to_string());
        config.save().unwrap();

        let client = ApiClient::new(config);
        let header = client.get_auth_header().unwrap();
        assert_eq!(header, Some("Bearer test-token".to_string()));
    }

    #[test]
    fn test_get_auth_header_without_token() {
        let (_data_dir, _config_dir) = setup_test_dirs();
        let config = ClientConfig::default();
        config.save().unwrap();

        let client = ApiClient::new(config);
        let header = client.get_auth_header().unwrap();
        assert_eq!(header, None);
    }

    #[test]
    fn test_get_api_token() {
        let (_data_dir, _config_dir) = setup_test_dirs();
        let mut config = ClientConfig::default();
        config.api_token = Some("test-token".to_string());
        config.save().unwrap();

        let client = ApiClient::new(config);
        let token = client.get_api_token().unwrap();
        assert_eq!(token, Some("test-token".to_string()));
    }
}
