use serde::{Deserialize, Serialize};

use crate::models::UtcDateTime;

#[derive(Debug, sqlx::FromRow, Clone, Serialize, Deserialize, PartialEq)]
pub struct Remote {
    pub id: String,
    pub app_id: String,
    pub name: String,
    pub remote: String,
    pub branch: String,
    pub directory: String,

    pub created_at: UtcDateTime,
    pub updated_at: UtcDateTime,
}
