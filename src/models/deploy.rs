use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Deploy {
    pub id: String,
    pub app_id: String,
    pub image: String,
    pub git_sha: Option<String>,
    pub status: String, // "in_progress" | "succeeded" | "failed"
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl Deploy {
    /// Create a new deploy record in the "in_progress" state.
    pub fn new(app_id: &str, image: &str, git_sha: Option<&str>) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            app_id: app_id.to_string(),
            image: image.to_string(),
            git_sha: git_sha.map(|s| s.to_string()),
            status: "in_progress".to_string(),
            error: None,
            created_at: now.clone(),
            updated_at: now,
        }
    }
}
