use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use crate::models::{now, UtcDateTime};

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct GitHubConnection {
    pub id: String,
    pub user_id: String,
    pub github_user_id: i64,
    pub github_username: String,
    pub github_email: Option<String>,
    #[serde(skip_serializing)]
    pub access_token: String,
    pub scopes: String,
    pub created_at: UtcDateTime,
    pub updated_at: UtcDateTime,
}

impl GitHubConnection {
    pub fn new(
        user_id: &str,
        github_user_id: i64,
        github_username: &str,
        github_email: Option<String>,
        access_token: &str,
        scopes: &str,
    ) -> Self {
        let now = now();

        Self {
            id: Uuid::new_v4().to_string(),
            user_id: user_id.to_string(),
            github_user_id,
            github_username: github_username.to_string(),
            github_email,
            access_token: access_token.to_string(),
            scopes: scopes.to_string(),
            created_at: now.clone(),
            updated_at: now,
        }
    }

    pub fn update_token(&mut self, access_token: &str, scopes: &str) {
        self.access_token = access_token.to_string();
        self.scopes = scopes.to_string();
        self.updated_at = now();
    }
}
