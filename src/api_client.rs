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

    pub async fn create_app(&self, app_name: &str) -> Result<()> {
        let url = format!("{}/apps", self.config.base_url);
        let payload = serde_json::json!({ "name": app_name });

        self.execute_request_text(|client, auth_header| {
            let mut req = client.post(&url).json(&payload);
            if let Some(header) = auth_header {
                req = req.header("Authorization", header);
            }
            req
        }).await?;

        println!("App '{}' created successfully", app_name);
        Ok(())
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

        println!("App '{}' started successfully", app_name);
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

        println!("App '{}' stopped successfully", app_name);
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

        println!("App '{}' deleted successfully", app_name);
        Ok(())
    }

    pub async fn deploy_app(
        &self,
        app_name: &str,
        tarball_path: &str,
        image_tag: Option<&str>,
        git_commit: Option<&str>,
        no_start: bool,
    ) -> Result<()> {
        // Build multipart form with image tarball and metadata
        let mut form = reqwest::multipart::Form::new()
            .file("image", tarball_path)
            .await
            .map_err(|e| anyhow!("Failed to read image tarball: {}", e))?;

        if let Some(tag) = image_tag {
            form = form.text("image_tag", tag.to_string());
        }
        if let Some(commit) = git_commit {
            form = form.text("git_commit", commit.to_string());
        }
        if no_start {
            form = form.text("no_start", "true".to_string());
        }

        let url = format!("{}/apps/{}/deploy", self.config.base_url, app_name);
        let auth_header = self.get_auth_header()?;

        let mut request = self.client.post(&url).multipart(form);
        if let Some(header) = auth_header {
            request = request.header("Authorization", header);
        }

        let response = request.send().await?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(anyhow!(
                "Authentication required or invalid token. Run 'lh connect <base-url> --token <token>' to configure the admin token."
            ));
        }

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
        let url = format!("{}/apps/{}/env", self.config.base_url, app_name);
        let payload = serde_json::json!({ "key": key, "value": value, "delete": delete });

        self.execute_request_text(|client, auth_header| {
            let mut req = client.post(&url).json(&payload);
            if let Some(header) = auth_header {
                req = req.header("Authorization", header);
            }
            req
        }).await?;

        println!("Environment variable set for app '{}'", app_name);
        Ok(())
    }

    pub async fn get_status(&self, app_name: Option<&str>) -> Result<()> {
        let url = match app_name {
            Some(name) => format!("{}/apps/{}", self.config.base_url, name),
            None => format!("{}/apps", self.config.base_url),
        };

        let status = self.execute_request_text(|client, auth_header| {
            let mut req = client.get(&url);
            if let Some(header) = auth_header {
                req = req.header("Authorization", header);
            }
            req
        }).await?;

        println!("Status: {}", status);
        Ok(())
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
