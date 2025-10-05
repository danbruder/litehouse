use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::models::{now, UtcDateTime};

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

impl Remote {
    pub fn new(app_id: &str, name: &str, remote: &str, branch: &str, directory: &str) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            app_id: app_id.to_string(),
            name: name.to_string(),
            remote: remote.to_string(),
            branch: branch.to_string(),
            directory: directory.to_string(),
            created_at: now(),
            updated_at: now(),
            
        }
    }
}