use super::AppState;
use crate::models::{now, UtcDateTime};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateChange {
    pub id: String,
    pub app_id: String,
    pub state: AppState,
    pub last_state: Option<AppState>,
    pub last_error: String,

    pub created_at: UtcDateTime,
}

impl StateChange {
    pub fn new(app_id: &str, new_state: AppState) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            app_id: app_id.to_string(),
            created_at: now(),
            last_state: None,
            state: new_state,
            last_error: String::new(),
        }
    }

    pub fn with_last_error(mut self, error: &str) -> Self {
        self.last_error = error.to_string();
        self
    }

    pub fn with_last_state(mut self, last_state: AppState) -> Self {
        self.last_state = Some(last_state);
        self
    }
}
