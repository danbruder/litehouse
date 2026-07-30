use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use crate::models::AppState;
use crate::models::{UtcDateTime, now};

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct App {
    pub id: String,
    pub name: String,
    pub port: Option<i64>,
    pub organization_id: Option<String>,
    pub created_at: UtcDateTime,
    pub updated_at: UtcDateTime,

    pub state: AppState,

    pub repo: Option<String>,
    pub image: Option<String>,
    pub exposed_port: Option<String>,
    pub deploy_token_hash: Option<String>,
    /// JSON array of custom top-level domains routed to this app in
    /// addition to the derived `{name}.{server_domain}` host (e.g.
    /// `["familyquotes.app", "www.familyquotes.app"]`). NULL/empty means
    /// no custom domains.
    pub custom_domains: Option<String>,
    /// HTTP path Caddy actively health-checks on this app's upstream (e.g.
    /// `/healthz`). NULL means no health check configured -- the app keeps
    /// today's plain `reverse_proxy` behavior with no active health check
    /// or passive retry tuning.
    pub health_check_path: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error(
        "Invalid app name: {0}. App names must be lowercase alphanumeric with optional hyphens or underscores."
    )]
    InvalidName(String),
    #[error(
        "Invalid domain: {0}. Domains must be lowercase, contain a '.', and have no scheme, path, or spaces."
    )]
    InvalidDomain(String),
}

impl App {
    pub fn new(name: &str) -> Result<Self, AppError> {
        let now = now();

        if !is_valid_app_name(name) {
            return Err(AppError::InvalidName(name.to_string()));
        }

        Ok(Self {
            id: Uuid::new_v4().to_string(),
            name: name.to_string(),
            port: None,
            organization_id: None,
            created_at: now.clone(),
            updated_at: now,
            state: AppState::Stopped,
            repo: None,
            image: None,
            exposed_port: None,
            deploy_token_hash: None,
            custom_domains: None,
            health_check_path: None,
        })
    }

    pub fn new_with_org(name: &str, organization_id: &str) -> Result<Self, AppError> {
        let now = now();

        if !is_valid_app_name(name) {
            return Err(AppError::InvalidName(name.to_string()));
        }

        Ok(Self {
            id: Uuid::new_v4().to_string(),
            name: name.to_string(),
            port: None,
            organization_id: Some(organization_id.to_string()),
            created_at: now.clone(),
            updated_at: now,
            state: AppState::Stopped,
            repo: None,
            image: None,
            exposed_port: None,
            deploy_token_hash: None,
            custom_domains: None,
            health_check_path: None,
        })
    }

    pub fn is_running(&self) -> bool {
        matches!(self.state, AppState::Running | AppState::Starting)
    }

    pub fn started(mut self) -> Self {
        self.state = AppState::Starting;
        self.updated_at = now();
        self
    }

    pub fn running(mut self) -> Self {
        self.state = AppState::Running;
        self.updated_at = now();
        self
    }

    /// Parse `custom_domains` (a JSON array of hostnames) into a `Vec`.
    /// Returns an empty vec on NULL or a parse error rather than failing —
    /// a malformed column should degrade to "no custom domains" instead of
    /// breaking Caddy sync.
    pub fn custom_domains_list(&self) -> Vec<String> {
        match &self.custom_domains {
            Some(json) => serde_json::from_str(json).unwrap_or_default(),
            None => Vec::new(),
        }
    }
}

/// Validate a custom domain hostname: must contain a '.', be lowercase,
/// and have no scheme, path, or whitespace.
pub fn is_valid_domain(domain: &str) -> bool {
    if domain.is_empty() || !domain.contains('.') {
        return false;
    }
    if domain != domain.to_lowercase() {
        return false;
    }
    if domain.contains("://") || domain.contains('/') || domain.chars().any(char::is_whitespace) {
        return false;
    }
    // Reasonable hostname character set: alphanumerics, '.', '-'.
    domain
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

/// Validate a health check path: must start with '/', contain no
/// whitespace, and have no scheme/host.
pub fn is_valid_health_check_path(path: &str) -> bool {
    if !path.starts_with('/') {
        return false;
    }
    if path.contains("://") || path.chars().any(char::is_whitespace) {
        return false;
    }
    true
}

/// Validate app name
fn is_valid_app_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 64 {
        return false;
    }

    let valid_chars = name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');

    valid_chars
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_valid_health_check_path_accepts_plain_path() {
        assert!(is_valid_health_check_path("/healthz"));
    }

    #[test]
    fn is_valid_health_check_path_rejects_empty() {
        assert!(!is_valid_health_check_path(""));
    }

    #[test]
    fn is_valid_health_check_path_rejects_scheme() {
        assert!(!is_valid_health_check_path("https://example.com/healthz"));
    }

    #[test]
    fn is_valid_health_check_path_rejects_whitespace() {
        assert!(!is_valid_health_check_path("/health check"));
    }

    #[test]
    fn is_valid_health_check_path_rejects_missing_leading_slash() {
        assert!(!is_valid_health_check_path("healthz"));
    }
}
