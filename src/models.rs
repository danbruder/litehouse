use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct App {
    pub id: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,

    pub git_directory: Option<String>,
    pub git_remote: Option<String>,
    pub git_branch: Option<String>,
    pub state: AppState,

    pub image_id: Option<String>,
    pub image_tag: Option<String>,
    pub git_commit: Option<String>,
    pub last_built_at: Option<DateTime<Utc>>,
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Invalid app name: {0}. App names must be lowercase alphanumeric with optional hyphens or underscores.")]
    InvalidName(String),
}

#[derive(Debug)]
pub struct AppBuild {
    pub image_id: String,
    pub image_tag: String,
    pub git_commit: String,
}

impl App {
    pub fn new(name: &str) -> Result<Self, AppError> {
        let now = Utc::now();

        if !is_valid_app_name(name) {
            return Err(AppError::InvalidName(name.to_string()));
        }

        Ok(Self {
            id: Uuid::new_v4().to_string(),
            name: name.to_string(),
            created_at: now,
            updated_at: now,
            git_directory: None,
            git_remote: None,
            git_branch: None,
            state: AppState::Created,
            image_id: None,
            image_tag: None,
            git_commit: None,
            last_built_at: None,
        })
    }

    pub fn is_running(&self) -> bool {
        matches!(self.state, AppState::Running | AppState::Starting)
    }

    pub fn is_built(&self) -> bool {
        self.image_id.is_some()
    }

    pub fn started(mut self) -> (Self, AppStateChange) {
        let change = AppStateChange::new(&self.id, AppState::Starting).with_last_state(self.state);
        self.state = AppState::Starting;
        self.updated_at = Utc::now();

        (self, change)
    }

    pub fn running(mut self) -> (Self, AppStateChange) {
        let change = AppStateChange::new(&self.id, AppState::Running).with_last_state(self.state);
        self.state = AppState::Running;
        self.updated_at = Utc::now();

        (self, change)
    }

    pub fn built(mut self, build: AppBuild) -> Self {
        self.image_id = Some(build.image_id);
        self.image_tag = Some(build.image_tag);
        self.git_commit = Some(build.git_commit);
        self.last_built_at = Some(Utc::now());
        self.updated_at = Utc::now();

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AppState {
    Created,
    Deployed,
    Starting,
    Running,
    Stopping,
    Stopped,
    Failed,
    Restarting,
    Crashed,
}

impl std::fmt::Display for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppState::Created => write!(f, "created"),
            AppState::Deployed => write!(f, "deployed"),
            AppState::Starting => write!(f, "starting"),
            AppState::Running => write!(f, "running"),
            AppState::Stopping => write!(f, "stopping"),
            AppState::Stopped => write!(f, "stopped"),
            AppState::Failed => write!(f, "failed"),
            AppState::Restarting => write!(f, "restarting"),
            AppState::Crashed => write!(f, "crashed"),
        }
    }
}

pub fn parse_app_state(state_str: &str) -> AppState {
    let state = match state_str {
        "created" => AppState::Created,
        "deployed" => AppState::Deployed,
        "starting" => AppState::Starting,
        "running" => AppState::Running,
        "stopping" => AppState::Stopping,
        "stopped" => AppState::Stopped,
        "failed" => AppState::Failed,
        "restarting" => AppState::Restarting,
        "crashed" => AppState::Crashed,
        _ => AppState::Created,
    };

    state
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppStateChange {
    pub id: String,
    pub app_id: String,
    pub created_at: DateTime<Utc>,
    pub state: AppState,
    pub last_state: Option<AppState>,
    pub last_error: String,
}

impl AppStateChange {
    pub fn new(app_id: &str, new_state: AppState) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            app_id: app_id.to_string(),
            created_at: Utc::now(),
            last_state: None,
            state: new_state,
            last_error: String::new(),
        }
    }

    pub fn with_last_error(mut self, error: &str) -> Self {
        self.last_error = error.to_string();
        self
    }

    pub fn with_last_state(mut self, last_state: AppState) -> Self {
        self.last_state = Some(last_state);
        self
    }
}
