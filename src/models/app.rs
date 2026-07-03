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
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error(
        "Invalid app name: {0}. App names must be lowercase alphanumeric with optional hyphens or underscores."
    )]
    InvalidName(String),
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
