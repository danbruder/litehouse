use chrono::{DateTime, Utc};

#[derive(Debug)]
pub struct AppBuild {
    pub app_id: String,
    pub image_id: String,
    pub image_tag: String,
    pub git_commit: String,
    pub created_at: DateTime<Utc>,
}
