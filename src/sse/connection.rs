use super::message::SSEMessage;
use crate::message_bus::{MessageBus, Message, SubscriptionFilter};
use axum::response::sse::Event;
use futures_util::stream::Stream;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use tracing::warn;

pub fn start_sse_stream(
    message_bus: Arc<MessageBus>,
    filter: SubscriptionFilter,
    _user_id: String,
) -> impl Stream<Item = Result<Event, Infallible>> {

    let receiver = message_bus.subscribe();

    async_stream::stream! {
        let mut rx = receiver;
        let mut interval = tokio::time::interval(Duration::from_secs(15));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                // Receive message from broadcast
                result = rx.recv() => {
                    match result {
                        Ok(msg) => {
                            // Filter message
                            if filter.matches(&msg) {
                                // Convert Message to SSEMessage
                                let sse_msg: SSEMessage = msg.into();
                                match sse_msg.to_sse_event() {
                                    Ok(event) => yield Ok(event),
                                    Err(e) => {
                                        warn!("Failed to serialize SSE message: {}", e);
                                        yield Ok(Event::default()
                                            .event("error")
                                            .data(format!("Serialization error: {}", e)));
                                    }
                                }
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(skipped)) => {
                            warn!("SSE client lagged, skipped {} messages", skipped);
                            let msg = Message::SystemNotification {
                                level: "warning".to_string(),
                                message: format!("Connection lagged, {} messages skipped", skipped),
                            };
                            let sse_msg: SSEMessage = msg.into();
                            if let Ok(event) = sse_msg.to_sse_event() {
                                yield Ok(event);
                            }
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            warn!("SSE broadcast channel closed");
                            yield Ok(Event::default().event("close").data("Stream closed"));
                            break;
                        }
                    }
                }
                // Send heartbeat
                _ = interval.tick() => {
                    let heartbeat: SSEMessage = Message::Heartbeat.into();
                    if let Ok(event) = heartbeat.to_sse_event() {
                        yield Ok(event);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;

    #[tokio::test]
    async fn test_stream_receives_messages() {
        use crate::message_bus::MessageBus;
        let message_bus = Arc::new(MessageBus::new());
        let filter = SubscriptionFilter::new(Some("1".to_string()));

        let mut stream = Box::pin(start_sse_stream(message_bus.clone(), filter, "1".to_string()));

        // Publish a message
        message_bus.publish(Message::SystemNotification {
            level: "info".to_string(),
            message: "test".to_string(),
        });

        // Should receive the message
        let event = tokio::time::timeout(Duration::from_secs(1), stream.next())
            .await
            .expect("Timeout waiting for event")
            .expect("Stream ended");

        let event = event.expect("Event error");
        // Format event as SSE string to check its data
        let event_str = format!("{:?}", event);
        assert!(event_str.contains("SystemNotification"));
    }

    #[tokio::test]
    async fn test_stream_filters_by_app_name() {
        use crate::message_bus::MessageBus;
        let message_bus = Arc::new(MessageBus::new());
        let filter = SubscriptionFilter::new(Some("1".to_string())).with_app_names(vec!["myapp".to_string()]);

        let mut stream = Box::pin(start_sse_stream(message_bus.clone(), filter, "1".to_string()));

        // Publish a message for different app
        message_bus.publish(Message::BuildLogs {
            app_name: "otherapp".to_string(),
            build_id: "123".to_string(),
            event_type: "message".to_string(),
            data: "line".to_string(),
        });

        // Publish a message for matching app
        message_bus.publish(Message::BuildLogs {
            app_name: "myapp".to_string(),
            build_id: "456".to_string(),
            event_type: "message".to_string(),
            data: "matching line".to_string(),
        });

        // Should only receive the matching message
        let event = tokio::time::timeout(Duration::from_secs(1), stream.next())
            .await
            .expect("Timeout waiting for event")
            .expect("Stream ended");

        let event = event.expect("Event error");
        // Format event as SSE string to check its data
        let event_str = format!("{:?}", event);
        assert!(event_str.contains("myapp"));
        assert!(event_str.contains("matching line"));
    }

    #[tokio::test]
    async fn test_heartbeat_sent() {
        use crate::message_bus::MessageBus;
        let message_bus = Arc::new(MessageBus::new());
        let filter = SubscriptionFilter::new(Some("1".to_string()));

        let mut stream = Box::pin(start_sse_stream(message_bus.clone(), filter, "1".to_string()));

        // Wait for heartbeat (should come within 15 seconds, but we'll wait up to 20)
        let event = tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                if let Some(Ok(event)) = stream.next().await {
                    let event_str = format!("{:?}", event);
                    if event_str.contains("Heartbeat") {
                        return event;
                    }
                }
            }
        })
        .await
        .expect("Timeout waiting for heartbeat");

        let event_str = format!("{:?}", event);
        assert!(event_str.contains("Heartbeat"));
    }

    #[tokio::test]
    async fn test_stream_receives_container_logs() {
        use crate::message_bus::MessageBus;
        let message_bus = Arc::new(MessageBus::new());
        let filter = SubscriptionFilter::new(Some("1".to_string())).with_app_names(vec!["myapp".to_string()]);

        let mut stream = Box::pin(start_sse_stream(message_bus.clone(), filter, "1".to_string()));

        // Publish a container logs message
        message_bus.publish(Message::ContainerLogs {
            app_name: "myapp".to_string(),
            data: "container log line".to_string(),
        });

        // Should receive the message
        let event = tokio::time::timeout(Duration::from_secs(1), stream.next())
            .await
            .expect("Timeout waiting for event")
            .expect("Stream ended");

        let event = event.expect("Event error");
        // Format event as SSE string to check its data
        let event_str = format!("{:?}", event);
        assert!(event_str.contains("ContainerLogs"));
        assert!(event_str.contains("myapp"));
        assert!(event_str.contains("container log line"));
    }

    #[tokio::test]
    async fn test_stream_filters_container_logs_by_app_name() {
        use crate::message_bus::MessageBus;
        let message_bus = Arc::new(MessageBus::new());
        let filter = SubscriptionFilter::new(Some("1".to_string())).with_app_names(vec!["myapp".to_string()]);

        let mut stream = Box::pin(start_sse_stream(message_bus.clone(), filter, "1".to_string()));

        // Publish a message for different app
        message_bus.publish(Message::ContainerLogs {
            app_name: "otherapp".to_string(),
            data: "should be filtered".to_string(),
        });

        // Publish a message for matching app
        message_bus.publish(Message::ContainerLogs {
            app_name: "myapp".to_string(),
            data: "should be received".to_string(),
        });

        // Should only receive the matching message
        let event = tokio::time::timeout(Duration::from_secs(1), stream.next())
            .await
            .expect("Timeout waiting for event")
            .expect("Stream ended");

        let event = event.expect("Event error");
        // Format event as SSE string to check its data
        let event_str = format!("{:?}", event);
        assert!(event_str.contains("myapp"));
        assert!(event_str.contains("should be received"));
        assert!(!event_str.contains("should be filtered"));
    }
}
