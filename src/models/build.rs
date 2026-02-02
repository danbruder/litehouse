use crate::models::{now, UtcDateTime};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BuildStatus {
    Idle,
    Building,
    Success,
    Failed,
}

impl std::fmt::Display for BuildStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuildStatus::Idle => write!(f, "idle"),
            BuildStatus::Building => write!(f, "building"),
            BuildStatus::Success => write!(f, "success"),
            BuildStatus::Failed => write!(f, "failed"),
        }
    }
}

impl std::str::FromStr for BuildStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "idle" => Ok(BuildStatus::Idle),
            "building" => Ok(BuildStatus::Building),
            "success" => Ok(BuildStatus::Success),
            "failed" => Ok(BuildStatus::Failed),
            _ => Err(format!("Unknown build status: {}", s)),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Build {
    pub id: String,
    pub app_id: String,
    pub image_id: Option<String>,
    pub image_tag: Option<String>,
    pub git_commit: Option<String>,
    pub log_path: Option<String>,
    pub exposed_port: Option<String>,
    pub status: BuildStatus,
    pub created_at: UtcDateTime,
    pub updated_at: UtcDateTime,
}

impl Build {
    /// Create a new build record in "building" state
    pub fn new_building(app_id: String, log_path: String) -> Build {
        let now = now();
        Build {
            id: Uuid::new_v4().to_string(),
            app_id,
            image_id: None,
            image_tag: None,
            git_commit: None,
            log_path: Some(log_path),
            exposed_port: None,
            status: BuildStatus::Building,
            created_at: now.clone(),
            updated_at: now,
        }
    }

    /// Mark build as successful with image details
    pub fn mark_success(&mut self, image_id: String, image_tag: String, git_commit: String) {
        self.image_id = Some(image_id);
        self.image_tag = Some(image_tag);
        self.git_commit = Some(git_commit);
        self.status = BuildStatus::Success;
        self.updated_at = now();
    }

    /// Mark build as failed
    pub fn mark_failed(&mut self) {
        self.status = BuildStatus::Failed;
        self.updated_at = now();
    }

    /// Set the exposed port from the Docker image
    pub fn set_exposed_port(&mut self, port: String) {
        self.exposed_port = Some(port);
        self.updated_at = now();
    }

    /// Create a successful build record (without building)
    pub fn new_success(
        app_id: String,
        image_tag: String,
        git_commit: String,
    ) -> Self {
        let now_val = now();
        Self {
            id: Uuid::new_v4().to_string(),
            app_id,
            image_id: None,
            image_tag: Some(image_tag),
            git_commit: Some(git_commit),
            log_path: None,
            exposed_port: None,
            status: BuildStatus::Success,
            created_at: now_val.clone(),
            updated_at: now_val,
        }
    }
}
