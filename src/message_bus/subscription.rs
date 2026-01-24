use super::message::Message;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct SubscriptionFilter {
    #[serde(default)]
    pub message_types: Option<Vec<String>>,
    #[serde(default)]
    pub app_names: Option<Vec<String>>,
    pub user_id: Option<String>,
}

impl SubscriptionFilter {
    pub fn new(user_id: Option<String>) -> Self {
        Self {
            message_types: None,
            app_names: None,
            user_id,
        }
    }

    pub fn with_message_types(mut self, types: Vec<String>) -> Self {
        self.message_types = Some(types);
        self
    }

    pub fn with_app_names(mut self, names: Vec<String>) -> Self {
        self.app_names = Some(names);
        self
    }

    pub fn matches(&self, message: &Message) -> bool {
        // Always allow heartbeats
        if matches!(message, Message::Heartbeat) {
            return true;
        }

        // Filter by message type if specified
        if let Some(ref types) = self.message_types {
            if !types.contains(&message.message_type().to_string()) {
                return false;
            }
        }

        // Filter by app name if specified
        if let Some(ref app_names) = self.app_names {
            if let Some(msg_app_name) = message.app_name() {
                if !app_names.iter().any(|name| name == msg_app_name) {
                    return false;
                }
            } else {
                // Message doesn't have an app_name, so it doesn't match app filter
                // Exception: GitHubOAuth and SystemNotification don't have app_name but should still be shown
                if !matches!(
                    message,
                    Message::GitHubOAuth { .. } | Message::SystemNotification { .. }
                ) {
                    return false;
                }
            }
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_by_message_type() {
        let filter = SubscriptionFilter::new(None).with_message_types(vec!["BuildLogs".to_string()]);

        let build_log = Message::BuildLogs {
            app_name: "test".to_string(),
            build_id: "123".to_string(),
            event_type: "message".to_string(),
            data: "line".to_string(),
        };

        let build_status = Message::BuildStatus {
            app_name: "test".to_string(),
            build_id: "123".to_string(),
            status: "building".to_string(),
        };

        assert!(filter.matches(&build_log));
        assert!(!filter.matches(&build_status));
        assert!(filter.matches(&Message::Heartbeat)); // Heartbeats always pass
    }

    #[test]
    fn test_filter_by_app_name() {
        let filter = SubscriptionFilter::new(None).with_app_names(vec!["myapp".to_string()]);

        let matching = Message::BuildLogs {
            app_name: "myapp".to_string(),
            build_id: "123".to_string(),
            event_type: "message".to_string(),
            data: "line".to_string(),
        };

        let not_matching = Message::BuildLogs {
            app_name: "otherapp".to_string(),
            build_id: "456".to_string(),
            event_type: "message".to_string(),
            data: "line".to_string(),
        };

        assert!(filter.matches(&matching));
        assert!(!filter.matches(&not_matching));
    }

    #[test]
    fn test_github_oauth_always_passes_app_filter() {
        let filter = SubscriptionFilter::new(None).with_app_names(vec!["myapp".to_string()]);

        let oauth_msg = Message::GitHubOAuth {
            event_type: "success".to_string(),
            data: "{}".to_string(),
        };

        assert!(filter.matches(&oauth_msg));
    }
}
