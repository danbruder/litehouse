use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, sqlx::FromRow, Serialize, Deserialize)]
pub struct BackupRecord {
    pub id: String,
    pub app_name: String,
    pub s3_key: String,
    pub size_bytes: i64,
    pub status: String,
    pub created_at: String,
}

impl BackupRecord {
    pub fn new(app_name: &str, s3_key: &str, size_bytes: i64) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            app_name: app_name.to_string(),
            s3_key: s3_key.to_string(),
            size_bytes,
            status: "succeeded".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }
}
