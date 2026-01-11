use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::{now, UtcDateTime};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvVar {
    pub id: String,
    pub app_id: String,
    pub key: String,
    pub value: String,
    pub created_at: UtcDateTime,
    pub updated_at: UtcDateTime,
}

impl EnvVar {
    pub fn new(app_id: &str, key: &str, value: &str) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            app_id: app_id.to_string(),
            key: key.to_string(),
            value: value.to_string(),
            created_at: now(),
            updated_at: now(),
        }
    }
}
