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
