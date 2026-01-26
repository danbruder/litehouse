use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum Message {
    GitHubOAuth {
        event_type: String,
        data: String,
    },
    BuildLogs {
        app_name: String,
        build_id: String,
        event_type: String,
        data: String,
    },
    BuildStatus {
        app_name: String,
        build_id: String,
        status: String,
    },
    ContainerLogs {
        app_name: String,
        data: String,
    },
    AppState {
        app_name: String,
        state: String,
    },
    SystemNotification {
        level: String,
        message: String,
    },
    WebhookReceived {
        app_name: String,
        event_type: String,
        status: String,
        delivery_id: Option<String>,
    },
    Heartbeat,
}

impl Message {
    pub fn app_name(&self) -> Option<&str> {
        match self {
            Message::BuildLogs { app_name, .. } => Some(app_name),
            Message::BuildStatus { app_name, .. } => Some(app_name),
            Message::ContainerLogs { app_name, .. } => Some(app_name),
            Message::AppState { app_name, .. } => Some(app_name),
            Message::WebhookReceived { app_name, .. } => Some(app_name),
            _ => None,
        }
    }

    pub fn message_type(&self) -> &str {
        match self {
            Message::GitHubOAuth { .. } => "GitHubOAuth",
            Message::BuildLogs { .. } => "BuildLogs",
            Message::BuildStatus { .. } => "BuildStatus",
            Message::ContainerLogs { .. } => "ContainerLogs",
            Message::AppState { .. } => "AppState",
            Message::SystemNotification { .. } => "SystemNotification",
            Message::WebhookReceived { .. } => "WebhookReceived",
            Message::Heartbeat => "Heartbeat",
        }
    }
}
