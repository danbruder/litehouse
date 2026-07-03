use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::fs;
use std::path::PathBuf;
use tracing::instrument;

thread_local! {
    static TEST_DATA_DIR: std::cell::RefCell<Option<PathBuf>> = std::cell::RefCell::new(None);
    static TEST_CONFIG_DIR: std::cell::RefCell<Option<PathBuf>> = std::cell::RefCell::new(None);
}

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum ConfigError {
    #[error("I/O error: {0}")]
    IoError(String),
    #[error("Port error: {0}")]
    PortError(String),
    #[error("TOML parsing error: {0}")]
    TomlError(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub caddy_http_port: Option<u16>,
    pub caddy_https_port: Option<u16>,
    pub domain: Option<String>,
    #[serde(default)]
    pub admin_token_hash: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClientConfig {
    pub base_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_token: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: 3030,
            caddy_http_port: Some(9090),
            caddy_https_port: Some(9091), // Use default 443 in production
            domain: None,
            admin_token_hash: None,
        }
    }
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            base_url: "http://localhost:3030/api".to_string(),
            api_token: None,
        }
    }
}

/// Set test directories
/// Can be called multiple times to reset directories between tests
pub fn set_test_dirs(data_dir: PathBuf, config_dir: PathBuf) -> Result<(), PathBuf> {
    TEST_DATA_DIR.with(|dir| *dir.borrow_mut() = Some(data_dir));
    TEST_CONFIG_DIR.with(|dir| *dir.borrow_mut() = Some(config_dir));
    Ok(())
}

/// Get the config directory
#[instrument]
pub fn get_config_dir() -> Result<PathBuf, ConfigError> {
    let test_dir = TEST_CONFIG_DIR.with(|dir| dir.borrow().clone());
    if let Some(dir) = test_dir {
        return Ok(dir);
    }
    let config_dir = get_base_dir().join("config");

    if !config_dir.exists() {
        fs::create_dir_all(&config_dir)
            .map_err(|_| ConfigError::IoError("Failed to create config directory".to_string()))?;
    }

    Ok(config_dir)
}

/// Get the data directory
pub fn get_data_dir() -> Result<PathBuf, ConfigError> {
    let test_dir = TEST_DATA_DIR.with(|dir| dir.borrow().clone());
    if let Some(dir) = test_dir {
        return Ok(dir);
    }

    let data_dir = get_base_dir().join("data");

    if !data_dir.exists() {
        fs::create_dir_all(&data_dir)
            .map_err(|_| ConfigError::IoError("Failed to create data directory".to_string()))?;
    }

    Ok(data_dir)
}

/// Get a unique port for a new app
#[instrument]
pub async fn get_next_available_port(
    db_pool: &sqlx::Pool<sqlx::Sqlite>,
) -> Result<i64, ConfigError> {
    let start_port = 8000;

    // Get all currently used ports
    let rows = sqlx::query("SELECT port FROM app WHERE port IS NOT NULL")
        .fetch_all(db_pool)
        .await
        .context("Failed to query app ports from database")
        .map_err(|e| ConfigError::PortError(e.to_string()))?;

    let mut used_ports = Vec::with_capacity(rows.len());
    for row in &rows {
        used_ports.push(row.get::<i64, _>("port"));
    }

    // Find first available port
    let mut port = start_port as i64;
    while used_ports.contains(&port) {
        port += 1;
    }

    Ok(port)
}

/// Get the app directory
#[instrument]
pub fn get_app_dir(app_name: &str) -> Result<PathBuf, ConfigError> {
    let apps_dir = get_data_dir()?.join("apps");

    if !apps_dir.exists() {
        fs::create_dir_all(&apps_dir)
            .map_err(|_| ConfigError::IoError("Failed to create apps directory".to_string()))?;
    }

    let app_dir = apps_dir.join(app_name);

    if !app_dir.exists() {
        fs::create_dir_all(&app_dir)
            .map_err(|_| ConfigError::IoError("Failed to create app directory".to_string()))?;
    }

    Ok(app_dir)
}




/// Get the app binary path
#[instrument]
pub fn get_app_binary_path(app_name: &str) -> Result<PathBuf, ConfigError> {
    let app_path = get_app_dir(app_name)?.join("app");

    Ok(app_path)
}

/// Get the app binary path
pub fn get_app_data_dir(app_name: &str) -> Result<PathBuf, ConfigError> {
    let data_dir = get_app_dir(app_name)?.join("data");
    if !data_dir.exists() {
        fs::create_dir_all(&data_dir)
            .context(format!(
                "Failed to create data directory: {}",
                data_dir.display()
            ))
            .map_err(|_| ConfigError::IoError("Failed to create app data directory".to_string()))?;
    }
    Ok(data_dir)
}

/// Get the app database path
#[instrument]
pub fn get_app_database_path(app_name: &str) -> Result<PathBuf, ConfigError> {
    let data_dir = get_app_data_dir(app_name)?;
    Ok(data_dir.join("app.db"))
}

/// Initialize the SQLite database for an app
#[instrument]
pub fn init_app_database(app_name: &str) -> Result<(), ConfigError> {
    let db_path = get_app_database_path(app_name)?;

    // Ensure parent directory exists and is writable
    if let Some(parent) = db_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent).map_err(|e| {
                ConfigError::IoError(format!("Failed to create data directory: {}", e))
            })?;
        }
        // Ensure parent directory is writable
        let metadata = fs::metadata(parent).map_err(|e| {
            ConfigError::IoError(format!("Failed to get directory metadata: {}", e))
        })?;
        let mut perms = metadata.permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Set to 755 (rwxr-xr-x) to allow owner write, others read/execute
            perms.set_mode(0o755);
        }
        fs::set_permissions(parent, perms).map_err(|e| {
            ConfigError::IoError(format!("Failed to set directory permissions: {}", e))
        })?;
    }

    // Create empty database file if it doesn't exist
    if !db_path.exists() {
        let file = fs::File::create(&db_path)
            .map_err(|e| ConfigError::IoError(format!("Failed to create database file: {}", e)))?;
        // Ensure the file is writable
        let metadata = file
            .metadata()
            .map_err(|e| ConfigError::IoError(format!("Failed to get file metadata: {}", e)))?;
        let mut perms = metadata.permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Set to 644 (rw-r--r--) to allow owner read/write, others read
            perms.set_mode(0o644);
        }
        fs::set_permissions(&db_path, perms)
            .map_err(|e| ConfigError::IoError(format!("Failed to set file permissions: {}", e)))?;
        tracing::info!(
            "Created SQLite database for app '{}' at {}",
            app_name,
            db_path.display()
        );
    } else {
        // Ensure existing database file is writable
        let metadata = fs::metadata(&db_path)
            .map_err(|e| ConfigError::IoError(format!("Failed to get file metadata: {}", e)))?;
        let mut perms = metadata.permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Ensure owner has write permission
            let mode = perms.mode();
            if mode & 0o200 == 0 {
                // Owner doesn't have write permission, add it
                perms.set_mode(mode | 0o200);
                fs::set_permissions(&db_path, perms).map_err(|e| {
                    ConfigError::IoError(format!("Failed to set file permissions: {}", e))
                })?;
                tracing::info!(
                    "Fixed write permissions on database file for app '{}' at {}",
                    app_name,
                    db_path.display()
                );
            }
        }
    }

    Ok(())
}

/// Get the app log file path
#[instrument]
pub fn get_app_log_path(app_name: &str) -> Result<PathBuf, ConfigError> {
    Ok(get_app_dir(app_name)?.join(format!("{}.log", app_name)))
}

impl ClientConfig {
    #[tracing::instrument]
    pub fn load() -> Result<Self, ConfigError> {
        let config_path = Self::get_config_path()?;
        tracing::info!("Loading client config from {}", config_path.display());

        if !config_path.exists() {
            let config = Self::default();
            config.save()?;
            return Ok(config);
        }

        let contents = fs::read_to_string(config_path)
            .map_err(|_| ConfigError::IoError("Failed to read config file".to_string()))?;
        let config: Self =
            toml::from_str(&contents).map_err(|e| ConfigError::TomlError(e.to_string()))?;
        Ok(config)
    }

    #[tracing::instrument]
    pub fn save(&self) -> Result<(), ConfigError> {
        let config_path = Self::get_config_path()?;
        let contents =
            toml::to_string_pretty(self).map_err(|e| ConfigError::TomlError(e.to_string()))?;
        tracing::info!("Saving client config to {}", config_path.display());
        fs::write(config_path, contents)
            .map_err(|_| ConfigError::IoError("Failed to write config file".to_string()))?;
        Ok(())
    }

    pub fn get_config_path() -> Result<PathBuf, ConfigError> {
        Ok(get_config_dir()?.join("client-config.toml"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_test_dirs() -> (TempDir, TempDir) {
        let data_dir = tempfile::tempdir().unwrap();
        let config_dir = tempfile::tempdir().unwrap();
        // Ensure directories exist
        std::fs::create_dir_all(data_dir.path()).unwrap();
        std::fs::create_dir_all(config_dir.path()).unwrap();
        set_test_dirs(
            data_dir.path().to_path_buf(),
            config_dir.path().to_path_buf(),
        ).unwrap();
        (data_dir, config_dir)
    }

    #[test]
    fn test_client_config_default() {
        let (_data_dir, _config_dir) = setup_test_dirs();
        let config = ClientConfig::default();
        assert_eq!(config.base_url, "http://localhost:3030/api");
        assert_eq!(config.api_token, None);
    }

    #[test]
    fn test_client_config_save_and_load() {
        let (_data_dir, _config_dir) = setup_test_dirs();
        let mut config = ClientConfig::default();
        config.base_url = "http://test.example.com/api".to_string();
        config.api_token = Some("test-token".to_string());

        config.save().unwrap();

        let loaded = ClientConfig::load().unwrap();
        assert_eq!(loaded.base_url, "http://test.example.com/api");
        assert_eq!(loaded.api_token, Some("test-token".to_string()));
    }

    #[test]
    fn test_client_config_save_without_token() {
        let (_data_dir, _config_dir) = setup_test_dirs();
        let config = ClientConfig::default();
        config.save().unwrap();

        let loaded = ClientConfig::load().unwrap();
        assert_eq!(loaded.api_token, None);
    }

    #[test]
    fn test_client_config_update_token() {
        let (_data_dir, _config_dir) = setup_test_dirs();
        let mut config = ClientConfig::default();
        config.save().unwrap();

        // Update with a token
        config.api_token = Some("new-token".to_string());
        config.save().unwrap();

        let loaded = ClientConfig::load().unwrap();
        assert_eq!(loaded.api_token, Some("new-token".to_string()));

        // Clear token
        config.api_token = None;
        config.save().unwrap();

        let loaded = ClientConfig::load().unwrap();
        assert_eq!(loaded.api_token, None);
    }
}

impl ServerConfig {
    #[tracing::instrument]
    pub fn load() -> Result<Self, ConfigError> {
        let config_path = Self::get_config_path()?;
        tracing::info!("Loading server config from {}", config_path.display());

        if !config_path.exists() {
            tracing::info!("Server config file not found, using defaults");
            return Ok(Self::default());
        }

        let contents = fs::read_to_string(config_path)
            .map_err(|_| ConfigError::IoError("Failed to read config file".to_string()))?;
        let config: Self =
            toml::from_str(&contents).map_err(|e| ConfigError::TomlError(e.to_string()))?;
        Ok(config)
    }

    #[tracing::instrument]
    pub fn save(&self) -> Result<(), ConfigError> {
        let config_path = Self::get_config_path()?;
        let contents =
            toml::to_string_pretty(self).map_err(|e| ConfigError::TomlError(e.to_string()))?;
        tracing::info!("Saving server config to {}", config_path.display());
        fs::write(config_path, contents)
            .map_err(|_| ConfigError::IoError("Failed to write config file".to_string()))?;
        Ok(())
    }

    pub fn get_config_path() -> Result<PathBuf, ConfigError> {
        Ok(get_config_dir()?.join("server-config.toml"))
    }
}

fn get_base_dir() -> PathBuf {
    // if on osx, use the home directory
    if cfg!(target_os = "macos") {
        return "/Users/dan/Desktop/litehouse-data".into();
        //return std::env::current_dir().unwrap();
    }

    if std::env::var("LITEHOUSE_DIR").is_ok() {
        PathBuf::from(std::env::var("LITEHOUSE_DIR").unwrap())
    } else {
        PathBuf::from("/opt/litehouse")
    }
}
