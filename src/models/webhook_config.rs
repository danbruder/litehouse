use crate::models::{utc_datetime, UtcDateTime};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookConfig {
    pub id: String,
    pub app_id: String,
    pub enabled: bool,
    pub secret: String,
    pub auto_deploy: bool,
    pub github_webhook_id: Option<i64>,
    pub status: WebhookStatus,
    pub error_message: Option<String>,
    pub created_at: UtcDateTime,
    pub updated_at: UtcDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WebhookStatus {
    Pending,
    Active,
    Failed,
}

impl WebhookStatus {
    pub fn as_str(&self) -> &str {
        match self {
            WebhookStatus::Pending => "pending",
            WebhookStatus::Active => "active",
            WebhookStatus::Failed => "failed",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "pending" => WebhookStatus::Pending,
            "active" => WebhookStatus::Active,
            "failed" => WebhookStatus::Failed,
            _ => WebhookStatus::Pending,
        }
    }
}

impl WebhookConfig {
    pub fn new(app_id: &str) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            app_id: app_id.to_string(),
            enabled: true,
            secret: Self::generate_secret(),
            auto_deploy: true,
            github_webhook_id: None,
            status: WebhookStatus::Pending,
            error_message: None,
            created_at: utc_datetime::now(),
            updated_at: utc_datetime::now(),
        }
    }

    fn generate_secret() -> String {
        use rand::RngCore;
        let mut rng = rand::thread_rng();
        let mut bytes = vec![0u8; 32];
        rng.fill_bytes(&mut bytes);
        hex::encode(bytes)
    }
}
