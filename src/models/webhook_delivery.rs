use crate::models::{utc_datetime, UtcDateTime};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookDelivery {
    pub id: String,
    pub app_id: Option<String>,
    pub github_delivery_id: Option<String>,
    pub github_event: String,
    pub repository_url: String,
    pub ref_: Option<String>,
    pub commit_sha: Option<String>,
    pub status: WebhookDeliveryStatus,
    pub error_message: Option<String>,
    pub build_id: Option<String>,
    pub payload_snippet: Option<String>,
    pub created_at: UtcDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WebhookDeliveryStatus {
    Matched,
    SignatureInvalid,
    AppNotFound,
    BuildTriggered,
    BuildFailed,
    IgnoredEvent,
}

impl WebhookDeliveryStatus {
    pub fn as_str(&self) -> &str {
        match self {
            WebhookDeliveryStatus::Matched => "matched",
            WebhookDeliveryStatus::SignatureInvalid => "signature_invalid",
            WebhookDeliveryStatus::AppNotFound => "app_not_found",
            WebhookDeliveryStatus::BuildTriggered => "build_triggered",
            WebhookDeliveryStatus::BuildFailed => "build_failed",
            WebhookDeliveryStatus::IgnoredEvent => "ignored_event",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "matched" => WebhookDeliveryStatus::Matched,
            "signature_invalid" => WebhookDeliveryStatus::SignatureInvalid,
            "app_not_found" => WebhookDeliveryStatus::AppNotFound,
            "build_triggered" => WebhookDeliveryStatus::BuildTriggered,
            "build_failed" => WebhookDeliveryStatus::BuildFailed,
            "ignored_event" => WebhookDeliveryStatus::IgnoredEvent,
            _ => WebhookDeliveryStatus::AppNotFound,
        }
    }
}

impl WebhookDelivery {
    pub fn new(
        app_id: Option<String>,
        github_delivery_id: Option<String>,
        github_event: String,
        repository_url: String,
        status: WebhookDeliveryStatus,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            app_id,
            github_delivery_id,
            github_event,
            repository_url,
            ref_: None,
            commit_sha: None,
            status,
            error_message: None,
            build_id: None,
            payload_snippet: None,
            created_at: utc_datetime::now(),
        }
    }
}
