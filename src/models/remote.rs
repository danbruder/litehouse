use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, sqlx::FromRow, Clone, Serialize, Deserialize, PartialEq)]
pub struct Remote {
    pub id: String,
    pub app_id: String,
    pub name: String,
    pub remote: String,
    pub branch: String,
    pub directory: String,

    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
