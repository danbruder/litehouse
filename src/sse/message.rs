use crate::message_bus::Message;
use axum::response::sse::Event;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum SSEMessage {
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
    Heartbeat,
}

impl SSEMessage {
    pub fn to_sse_event(&self) -> Result<Event, serde_json::Error> {
        let event_name = match self {
            SSEMessage::GitHubOAuth { .. } => "github_oauth",
            SSEMessage::BuildLogs { .. } => "build_logs",
            SSEMessage::BuildStatus { .. } => "build_status",
            SSEMessage::ContainerLogs { .. } => "container_logs",
            SSEMessage::AppState { .. } => "app_state",
            SSEMessage::SystemNotification { .. } => "system_notification",
            SSEMessage::Heartbeat => "heartbeat",
        };

        let data = serde_json::to_string(self)?;
        Ok(Event::default().event(event_name).data(data))
    }

    pub fn app_name(&self) -> Option<&str> {
        match self {
            SSEMessage::BuildLogs { app_name, .. } => Some(app_name),
            SSEMessage::BuildStatus { app_name, .. } => Some(app_name),
            SSEMessage::ContainerLogs { app_name, .. } => Some(app_name),
            SSEMessage::AppState { app_name, .. } => Some(app_name),
            _ => None,
        }
    }

    pub fn message_type(&self) -> &str {
        match self {
            SSEMessage::GitHubOAuth { .. } => "GitHubOAuth",
            SSEMessage::BuildLogs { .. } => "BuildLogs",
            SSEMessage::BuildStatus { .. } => "BuildStatus",
            SSEMessage::ContainerLogs { .. } => "ContainerLogs",
            SSEMessage::AppState { .. } => "AppState",
            SSEMessage::SystemNotification { .. } => "SystemNotification",
            SSEMessage::Heartbeat => "Heartbeat",
        }
    }
}

impl From<Message> for SSEMessage {
    fn from(msg: Message) -> Self {
        match msg {
            Message::GitHubOAuth { event_type, data } => SSEMessage::GitHubOAuth {
                event_type,
                data,
            },
            Message::BuildLogs {
                app_name,
                build_id,
                event_type,
                data,
            } => SSEMessage::BuildLogs {
                app_name,
                build_id,
                event_type,
                data,
            },
            Message::BuildStatus {
                app_name,
                build_id,
                status,
            } => SSEMessage::BuildStatus {
                app_name,
                build_id,
                status,
            },
            Message::ContainerLogs { app_name, data } => SSEMessage::ContainerLogs {
                app_name,
                data,
            },
            Message::AppState { app_name, state } => SSEMessage::AppState { app_name, state },
            Message::SystemNotification { level, message } => SSEMessage::SystemNotification {
                level,
                message,
            },
            Message::Heartbeat => SSEMessage::Heartbeat,
        }
    }
}
