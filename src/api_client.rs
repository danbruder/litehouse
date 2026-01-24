use crate::config::ClientConfig;
use crate::github::RepoInfo;
use crate::models::{AuthResponse, AuthenticatedUser, LoginRequest, RegisterRequest, RefreshTokenRequest, TokenPair};
use anyhow::{anyhow, Result};
use bytes::Bytes;
use futures_util::stream::BoxStream;
use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};

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

    /// Get the current access token from config (reloads from disk)
    fn get_access_token(&self) -> Result<Option<String>> {
        let config = ClientConfig::load()?;
        Ok(config.access_token)
    }

    /// Get the current refresh token from config (reloads from disk)
    fn get_refresh_token(&self) -> Result<Option<String>> {
        let config = ClientConfig::load()?;
        Ok(config.refresh_token)
    }

    /// Update tokens in config file
    fn update_tokens(&self, access_token: Option<String>, refresh_token: Option<String>) -> Result<()> {
        let mut config = ClientConfig::load()?;
        config.access_token = access_token;
        config.refresh_token = refresh_token;
        config.save()?;
        Ok(())
    }

    /// Clear tokens from config file
    fn clear_tokens(&self) -> Result<()> {
        self.update_tokens(None, None)
    }

    /// Get Authorization header value if token exists
    fn get_auth_header(&self) -> Result<Option<String>> {
        if let Some(token) = self.get_access_token()? {
            Ok(Some(format!("Bearer {}", token)))
        } else {
            Ok(None)
        }
    }

    // ===== Auth Methods =====

    pub async fn login(&self, email: &str, password: &str) -> Result<AuthResponse> {
        let response = self
            .client
            .post(&format!("{}/auth/login", self.config.base_url))
            .json(&LoginRequest {
                email: email.to_string(),
                password: password.to_string(),
            })
            .send()
            .await?;

        if response.status().is_success() {
            let auth_response: AuthResponse = response.json().await?;
            Ok(auth_response)
        } else {
            let error = response.text().await?;
            Err(anyhow!("Login failed: {}", error))
        }
    }

    pub async fn register(
        &self,
        email: &str,
        password: &str,
        full_name: Option<&str>,
        organization_name: Option<&str>,
    ) -> Result<AuthResponse> {
        let response = self
            .client
            .post(&format!("{}/auth/register", self.config.base_url))
            .json(&RegisterRequest {
                email: email.to_string(),
                password: password.to_string(),
                full_name: full_name.map(|s| s.to_string()),
                organization_name: organization_name.map(|s| s.to_string()),
            })
            .send()
            .await?;

        if response.status().is_success() {
            let auth_response: AuthResponse = response.json().await?;
            Ok(auth_response)
        } else {
            let error = response.text().await?;
            Err(anyhow!("Registration failed: {}", error))
        }
    }

    pub async fn refresh_token(&self) -> Result<TokenPair> {
        let refresh_token = self
            .get_refresh_token()?
            .ok_or_else(|| anyhow!("No refresh token available"))?;

        let response = self
            .client
            .post(&format!("{}/auth/refresh", self.config.base_url))
            .json(&RefreshTokenRequest {
                refresh_token: refresh_token.clone(),
            })
            .send()
            .await?;

        if response.status().is_success() {
            let tokens: TokenPair = response.json().await?;
            // Update tokens in config
            self.update_tokens(Some(tokens.access_token.clone()), Some(tokens.refresh_token.clone()))?;
            Ok(tokens)
        } else {
            let error = response.text().await?;
            // Clear tokens on refresh failure
            self.clear_tokens()?;
            Err(anyhow!("Token refresh failed: {}", error))
        }
    }

    pub async fn logout(&self) -> Result<()> {
        let refresh_token = self.get_refresh_token()?;
        
        if let Some(token) = refresh_token {
            let _ = self
                .client
                .post(&format!("{}/auth/logout", self.config.base_url))
                .json(&RefreshTokenRequest { refresh_token: token })
                .send()
                .await;
        }

        // Always clear local tokens
        self.clear_tokens()?;
        Ok(())
    }

    pub async fn get_current_user(&self) -> Result<AuthenticatedUser> {
        let auth_header = self.get_auth_header()?;
        let mut request = self
            .client
            .get(&format!("{}/auth/me", self.config.base_url));

        if let Some(header) = auth_header {
            request = request.header("Authorization", header);
        }

        let response = request.send().await?;

        if response.status().is_success() {
            let user: AuthenticatedUser = response.json().await?;
            Ok(user)
        } else {
            let error = response.text().await?;
            Err(anyhow!("Failed to get current user: {}", error))
        }
    }

    /// Helper to execute a request with automatic auth header and token refresh
    /// This handles 401 errors by attempting to refresh the token and retrying
    async fn execute_request<F, T>(&self, build_request: F) -> Result<T>
    where
        F: Fn(&Client, Option<String>) -> reqwest::RequestBuilder,
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
        F: Fn(&Client, Option<String>) -> reqwest::RequestBuilder,
    {
        self.execute_request_with_response(build_request, |r| {
            Box::pin(async move {
                r.text().await.map_err(|e| anyhow!("Failed to read response: {}", e))
            })
        }).await
    }

    /// Generic helper to execute a request with automatic auth and refresh
    async fn execute_request_with_response<F, G, T>(&self, build_request: F, process_response: G) -> Result<T>
    where
        F: Fn(&Client, Option<String>) -> reqwest::RequestBuilder,
        G: FnOnce(reqwest::Response) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<T>> + Send>>,
    {
        let auth_header = self.get_auth_header()?;
        let request = build_request(&self.client, auth_header.clone());
        let mut response = request.send().await?;

        // If we get 401 and have a refresh token, try to refresh
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            if let Some(_refresh_token) = self.get_refresh_token()? {
                // Try to refresh token
                if let Ok(new_tokens) = self.refresh_token().await {
                    // Retry the request with new token
                    let new_auth_header = format!("Bearer {}", new_tokens.access_token);
                    let retry_request = build_request(&self.client, Some(new_auth_header));
                    response = retry_request.send().await?;
                } else {
                    // Refresh failed, clear tokens and return error
                    self.clear_tokens()?;
                    let error = response.text().await.unwrap_or_else(|_| "Unauthorized".to_string());
                    return Err(anyhow!("Authentication failed. Please run 'lh auth login' to authenticate. Error: {}", error));
                }
            } else {
                // No refresh token, return error
                let error = response.text().await.unwrap_or_else(|_| "Unauthorized".to_string());
                return Err(anyhow!("Authentication required. Please run 'lh auth login' to authenticate. Error: {}", error));
            }
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

    pub async fn deploy_app(&self, app_name: &str, binary_path: &str) -> Result<()> {
        // Create multipart form
        let form = reqwest::multipart::Form::new()
            .file("binary", binary_path)
            .await
            .map_err(|e| anyhow!("Failed to create multipart form: {}", e))?;

        let url = format!("{}/apps/{}/deploy", self.config.base_url, app_name);
        let auth_header = self.get_auth_header()?;
        
        let mut request = self.client.post(&url).multipart(form);
        if let Some(header) = auth_header {
            request = request.header("Authorization", header);
        }

        let response = request.send().await?;

        // Handle 401 with refresh for multipart requests
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            if let Some(_refresh_token) = self.get_refresh_token()? {
                if let Ok(new_tokens) = self.refresh_token().await {
                    // Recreate form and retry
                    let form = reqwest::multipart::Form::new()
                        .file("binary", binary_path)
                        .await
                        .map_err(|e| anyhow!("Failed to create multipart form: {}", e))?;
                    let new_auth_header = format!("Bearer {}", new_tokens.access_token);
                    let retry_response = self.client
                        .post(&url)
                        .header("Authorization", new_auth_header)
                        .multipart(form)
                        .send()
                        .await?;
                    
                    if retry_response.status().is_success() {
                        println!("App '{}' deployed successfully", app_name);
                        return Ok(());
                    } else {
                        let error = retry_response.text().await.unwrap_or_else(|_| "Deploy failed".to_string());
                        return Err(anyhow!("Failed to deploy app: {}", error));
                    }
                } else {
                    self.clear_tokens()?;
                    return Err(anyhow!("Authentication failed. Please run 'lh auth login' to authenticate."));
                }
            } else {
                return Err(anyhow!("Authentication required. Please run 'lh auth login' to authenticate."));
            }
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
        
        let mut response = request.send().await?;

        // Handle 401 with refresh
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            if let Some(_refresh_token) = self.get_refresh_token()? {
                if let Ok(new_tokens) = self.refresh_token().await {
                    let new_auth_header = format!("Bearer {}", new_tokens.access_token);
                    let retry_request = self.client.get(&url)
                        .header("Authorization", new_auth_header);
                    response = retry_request.send().await?;
                } else {
                    self.clear_tokens()?;
                    return Err(anyhow!("Authentication failed. Please run 'lh auth login' to authenticate."));
                }
            } else {
                return Err(anyhow!("Authentication required. Please run 'lh auth login' to authenticate."));
            }
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

    pub async fn remote_add(&self, app_name: &str, remote: &str) -> Result<()> {
        let url = format!("{}/apps/{}/remote", self.config.base_url, app_name);
        let payload = serde_json::json!({ "remote": remote });
        
        self.execute_request_text(|client, auth_header| {
            let mut req = client.post(&url).json(&payload);
            if let Some(header) = auth_header {
                req = req.header("Authorization", header);
            }
            req
        }).await?;

        println!("Remote configured for app '{}'", app_name);
        Ok(())
    }

    pub async fn remote_remove(&self, app_name: &str) -> Result<()> {
        let url = format!("{}/apps/{}/remote", self.config.base_url, app_name);
        
        self.execute_request_text(|client, auth_header| {
            let mut req = client.delete(&url);
            if let Some(header) = auth_header {
                req = req.header("Authorization", header);
            }
            req
        }).await?;

        println!("Remote removed for app '{}'", app_name);
        Ok(())
    }

    pub async fn build(&self, app_name: &str) -> Result<()> {
        let url = format!("{}/apps/{}/build", self.config.base_url, app_name);
        
        self.execute_request_text(|client, auth_header| {
            let mut req = client.post(&url);
            if let Some(header) = auth_header {
                req = req.header("Authorization", header);
            }
            req
        }).await?;

        println!("App '{}' built successfully", app_name);
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
        println!("Litestream will now back up all databases to S3 bucket: {}", bucket);
        Ok(())
    }

    pub async fn get_s3_config(&self) -> Result<()> {
        let url = format!("{}/config/s3", self.config.base_url);
        
        let auth_header = self.get_auth_header()?;
        let mut request = self.client.get(&url);
        if let Some(header) = &auth_header {
            request = request.header("Authorization", header);
        }
        
        let mut response = request.send().await?;

        // Handle 401 with refresh
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            if let Some(_refresh_token) = self.get_refresh_token()? {
                if let Ok(new_tokens) = self.refresh_token().await {
                    let new_auth_header = format!("Bearer {}", new_tokens.access_token);
                    response = self.client.get(&url)
                        .header("Authorization", new_auth_header)
                        .send()
                        .await?;
                } else {
                    self.clear_tokens()?;
                    return Err(anyhow!("Authentication failed. Please run 'lh auth login' to authenticate."));
                }
            } else {
                return Err(anyhow!("Authentication required. Please run 'lh auth login' to authenticate."));
            }
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

    // ===== GitHub Methods =====

    pub async fn github_connect_start(&self) -> Result<DeviceFlowStartResponse> {
        let url = format!("{}/github/connect/start", self.config.base_url);
        self.execute_request(|client, auth_header| {
            let mut req = client.post(&url);
            if let Some(header) = auth_header {
                req = req.header("Authorization", header);
            }
            req
        }).await
    }

    pub async fn github_connect_poll(
        &self,
        device_code: &str,
        interval: u64,
        expires_in: u64,
    ) -> Result<GitHubConnectResponse> {
        let url = format!("{}/github/connect/poll", self.config.base_url);
        let payload = serde_json::json!({
            "device_code": device_code,
            "interval": interval,
            "expires_in": expires_in
        });
        
        let auth_header = self.get_auth_header()?;
        let mut request = self.client.post(&url).json(&payload);
        if let Some(header) = &auth_header {
            request = request.header("Authorization", header);
        }
        
        let mut response = request.send().await?;

        // Handle 401 with refresh
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            if let Some(_refresh_token) = self.get_refresh_token()? {
                if let Ok(new_tokens) = self.refresh_token().await {
                    let new_auth_header = format!("Bearer {}", new_tokens.access_token);
                    response = self.client.post(&url)
                        .header("Authorization", new_auth_header)
                        .json(&payload)
                        .send()
                        .await?;
                } else {
                    self.clear_tokens()?;
                    return Err(anyhow!("Authentication failed. Please run 'lh auth login' to authenticate."));
                }
            } else {
                return Err(anyhow!("Authentication required. Please run 'lh auth login' to authenticate."));
            }
        }

        if response.status().is_success() {
            let data: GitHubConnectResponse = response.json().await?;
            Ok(data)
        } else {
            let status = response.status();
            let error = response.text().await?;
            if status.as_u16() == 408 {
                return Err(anyhow!("Authorization timed out"));
            }
            if status.as_u16() == 403 {
                return Err(anyhow!("Authorization was denied"));
            }
            Err(anyhow!("Failed to complete GitHub connection: {}", error))
        }
    }

    pub async fn github_disconnect(&self) -> Result<()> {
        let url = format!("{}/github/connection", self.config.base_url);
        
        self.execute_request_text(|client, auth_header| {
            let mut req = client.delete(&url);
            if let Some(header) = auth_header {
                req = req.header("Authorization", header);
            }
            req
        }).await?;

        Ok(())
    }

    pub async fn github_status(&self) -> Result<GitHubStatusResponse> {
        let url = format!("{}/github/status", self.config.base_url);
        self.execute_request(|client, auth_header| {
            let mut req = client.get(&url);
            if let Some(header) = auth_header {
                req = req.header("Authorization", header);
            }
            req
        }).await
    }

    pub async fn github_list_repos(&self, limit: u32) -> Result<Vec<RepoInfo>> {
        let url = format!("{}/github/repos?limit={}", self.config.base_url, limit);
        self.execute_request(|client, auth_header| {
            let mut req = client.get(&url);
            if let Some(header) = auth_header {
                req = req.header("Authorization", header);
            }
            req
        }).await
    }

    pub async fn github_search_repos(&self, query: &str) -> Result<Vec<RepoInfo>> {
        let url = format!(
            "{}/github/repos/search?q={}",
            self.config.base_url,
            urlencoding::encode(query)
        );
        self.execute_request(|client, auth_header| {
            let mut req = client.get(&url);
            if let Some(header) = auth_header {
                req = req.header("Authorization", header);
            }
            req
        }).await
    }

    pub async fn create_app_from_github(&self, app_name: &str, github_repo: &str) -> Result<()> {
        let url = format!("{}/apps", self.config.base_url);
        let payload = serde_json::json!({
            "name": app_name,
            "from_github": github_repo
        });
        
        self.execute_request_text(|client, auth_header| {
            let mut req = client.post(&url).json(&payload);
            if let Some(header) = auth_header {
                req = req.header("Authorization", header);
            }
            req
        }).await?;

        println!("App '{}' created from GitHub repo '{}'", app_name, github_repo);
        Ok(())
    }
}

// GitHub response types
#[derive(Debug, Deserialize)]
pub struct DeviceFlowStartResponse {
    pub user_code: String,
    pub verification_uri: String,
    pub device_code: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Debug, Deserialize)]
pub struct GitHubConnectResponse {
    pub username: String,
    pub email: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct GitHubStatusResponse {
    pub connected: bool,
    pub username: Option<String>,
    pub email: Option<String>,
    pub scopes: Option<String>,
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
        config.access_token = Some("test-token".to_string());
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
    fn test_update_tokens() {
        let (_data_dir, _config_dir) = setup_test_dirs();
        let config = ClientConfig::default();
        config.save().unwrap();
        
        let client = ApiClient::new(config);
        client.update_tokens(
            Some("access-123".to_string()),
            Some("refresh-456".to_string())
        ).unwrap();
        
        let loaded = ClientConfig::load().unwrap();
        assert_eq!(loaded.access_token, Some("access-123".to_string()));
        assert_eq!(loaded.refresh_token, Some("refresh-456".to_string()));
    }

    #[test]
    fn test_clear_tokens() {
        let (_data_dir, _config_dir) = setup_test_dirs();
        let mut config = ClientConfig::default();
        config.access_token = Some("access-123".to_string());
        config.refresh_token = Some("refresh-456".to_string());
        config.save().unwrap();
        
        let client = ApiClient::new(config);
        client.clear_tokens().unwrap();
        
        let loaded = ClientConfig::load().unwrap();
        assert_eq!(loaded.access_token, None);
        assert_eq!(loaded.refresh_token, None);
    }

    #[test]
    fn test_get_access_token() {
        let (_data_dir, _config_dir) = setup_test_dirs();
        let mut config = ClientConfig::default();
        config.access_token = Some("test-access".to_string());
        config.save().unwrap();
        
        let client = ApiClient::new(config);
        let token = client.get_access_token().unwrap();
        assert_eq!(token, Some("test-access".to_string()));
    }

    #[test]
    fn test_get_refresh_token() {
        let (_data_dir, _config_dir) = setup_test_dirs();
        let mut config = ClientConfig::default();
        config.refresh_token = Some("test-refresh".to_string());
        config.save().unwrap();
        
        let client = ApiClient::new(config);
        let token = client.get_refresh_token().unwrap();
        assert_eq!(token, Some("test-refresh".to_string()));
    }
}
