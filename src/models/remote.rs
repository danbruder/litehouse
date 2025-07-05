use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Remote {
    pub id: String,
    pub app_id: String,
    pub git_remote: Option<String>,
    pub git_branch: Option<String>,
    pub git_directory: Option<String>,

    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
