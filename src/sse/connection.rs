use super::{hub::SSEHub, message::SSEMessage, subscription::SubscriptionFilter};
use axum::response::sse::Event;
use futures_util::stream::Stream;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use tracing::warn;

pub fn start_sse_stream(
    hub: Arc<SSEHub>,
    filter: SubscriptionFilter,
    _user_id: i64,
) -> impl Stream<Item = Result<Event, Infallible>> {

    let receiver = hub.subscribe();

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
                                match msg.to_sse_event() {
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
                            let msg = SSEMessage::SystemNotification {
                                level: "warning".to_string(),
                                message: format!("Connection lagged, {} messages skipped", skipped),
                            };
                            if let Ok(event) = msg.to_sse_event() {
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
                    if let Ok(event) = SSEMessage::Heartbeat.to_sse_event() {
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
    use tokio_stream::StreamExt;

    #[tokio::test]
    async fn test_stream_receives_messages() {
        let hub = Arc::new(SSEHub::new());
        let filter = SubscriptionFilter::new(1);

        let mut stream = Box::pin(start_sse_stream(hub.clone(), filter, 1));

        // Publish a message
        hub.publish(SSEMessage::SystemNotification {
            level: "info".to_string(),
            message: "test".to_string(),
        });

        // Should receive the message
        let event = tokio::time::timeout(Duration::from_secs(1), stream.next())
            .await
            .expect("Timeout waiting for event")
            .expect("Stream ended");

        let event = event.expect("Event error");
        assert!(event.data().contains("SystemNotification"));
    }

    #[tokio::test]
    async fn test_stream_filters_by_app_name() {
        let hub = Arc::new(SSEHub::new());
        let filter = SubscriptionFilter::new(1).with_app_names(vec!["myapp".to_string()]);

        let mut stream = Box::pin(start_sse_stream(hub.clone(), filter, 1));

        // Publish a message for different app
        hub.publish(SSEMessage::BuildLogs {
            app_name: "otherapp".to_string(),
            build_id: "123".to_string(),
            event_type: "message".to_string(),
            data: "line".to_string(),
        });

        // Publish a message for matching app
        hub.publish(SSEMessage::BuildLogs {
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
        let data = event.data();
        assert!(data.contains("myapp"));
        assert!(data.contains("matching line"));
    }

    #[tokio::test]
    async fn test_heartbeat_sent() {
        let hub = Arc::new(SSEHub::new());
        let filter = SubscriptionFilter::new(1);

        let mut stream = Box::pin(start_sse_stream(hub.clone(), filter, 1));

        // Wait for heartbeat (should come within 15 seconds, but we'll wait up to 20)
        let event = tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                if let Some(Ok(event)) = stream.next().await {
                    if event.data().contains("Heartbeat") {
                        return event;
                    }
                }
            }
        })
        .await
        .expect("Timeout waiting for heartbeat");

        assert!(event.data().contains("Heartbeat"));
    }
}
