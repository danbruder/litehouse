use super::message::SSEMessage;
use crate::message_bus::{Message, MessageBus, SubscriptionFilter};
use axum::response::sse::Event;
use futures_util::stream::Stream;
use std::collections::VecDeque;
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
    // Throttle configuration: process messages every 100ms (max 10 messages/second)
    const THROTTLE_INTERVAL_MS: u64 = 1000;

    let receiver = message_bus.subscribe();

    async_stream::stream! {
        let mut rx = receiver;
        let mut heartbeat_interval = tokio::time::interval(Duration::from_secs(15));
        heartbeat_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        // Throttle interval for processing queued messages
        let mut throttle_interval = tokio::time::interval(Duration::from_millis(THROTTLE_INTERVAL_MS));
        throttle_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        // Message queue for throttling (FIFO)
        let mut message_queue: VecDeque<Event> = VecDeque::new();

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
                                    Ok(event) => {
                                        // Add to queue for throttled processing (FIFO)
                                        message_queue.push_back(event);
                                    }
                                    Err(e) => {
                                        warn!("Failed to serialize SSE message: {}", e);
                                        let error_event = Event::default()
                                            .event("error")
                                            .data(format!("Serialization error: {}", e));
                                        message_queue.push_back(error_event);
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
                                // System notifications bypass throttle (they're important)
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
                // Process one message from queue per throttle interval (FIFO)
                _ = throttle_interval.tick() => {
                    if let Some(event) = message_queue.pop_front() {
                        yield Ok(event);
                    }
                }
                // Send heartbeat (not throttled)
                _ = heartbeat_interval.tick() => {
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

        let mut stream = Box::pin(start_sse_stream(
            message_bus.clone(),
            filter,
            "1".to_string(),
        ));

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
        let filter = SubscriptionFilter::new(Some("1".to_string()))
            .with_app_names(vec!["myapp".to_string()]);

        let mut stream = Box::pin(start_sse_stream(
            message_bus.clone(),
            filter,
            "1".to_string(),
        ));

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

        // Should only receive the matching message (skip heartbeats)
        let mut found = false;
        for _ in 0..10 {
            let event = tokio::time::timeout(Duration::from_secs(1), stream.next())
                .await
                .expect("Timeout waiting for event")
                .expect("Stream ended");

            let event = event.expect("Event error");
            // Format event as SSE string to check its data
            let event_str = format!("{:?}", event);
            if event_str.contains("myapp") && event_str.contains("matching line") {
                found = true;
                break;
            }
            // Skip heartbeats and continue
        }
        assert!(found, "Should receive the matching message for myapp");
    }

    #[tokio::test]
    async fn test_heartbeat_sent() {
        use crate::message_bus::MessageBus;
        let message_bus = Arc::new(MessageBus::new());
        let filter = SubscriptionFilter::new(Some("1".to_string()));

        let mut stream = Box::pin(start_sse_stream(
            message_bus.clone(),
            filter,
            "1".to_string(),
        ));

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
        let filter = SubscriptionFilter::new(Some("1".to_string()))
            .with_app_names(vec!["myapp".to_string()]);

        let mut stream = Box::pin(start_sse_stream(
            message_bus.clone(),
            filter,
            "1".to_string(),
        ));

        // Publish a container logs message
        message_bus.publish(Message::ContainerLogs {
            app_name: "myapp".to_string(),
            data: "container log line".to_string(),
        });

        // Should receive the message (may need to skip heartbeats)
        let mut found = false;
        for _ in 0..10 {
            let event = tokio::time::timeout(Duration::from_secs(1), stream.next())
                .await
                .expect("Timeout waiting for event")
                .expect("Stream ended");

            let event = event.expect("Event error");
            // Format event as SSE string to check its data
            let event_str = format!("{:?}", event);
            if event_str.contains("ContainerLogs") {
                assert!(event_str.contains("myapp"));
                assert!(event_str.contains("container log line"));
                found = true;
                break;
            }
            // Skip heartbeats and continue
        }
        assert!(found, "Should receive ContainerLogs message");
    }

    #[tokio::test]
    async fn test_stream_filters_container_logs_by_app_name() {
        use crate::message_bus::MessageBus;
        let message_bus = Arc::new(MessageBus::new());
        let filter = SubscriptionFilter::new(Some("1".to_string()))
            .with_app_names(vec!["myapp".to_string()]);

        let mut stream = Box::pin(start_sse_stream(
            message_bus.clone(),
            filter,
            "1".to_string(),
        ));

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

    #[tokio::test]
    async fn test_stream_throttles_messages() {
        use crate::message_bus::MessageBus;
        let message_bus = Arc::new(MessageBus::new());
        let filter = SubscriptionFilter::new(Some("1".to_string()));

        let mut stream = Box::pin(start_sse_stream(
            message_bus.clone(),
            filter,
            "1".to_string(),
        ));

        // Publish 5 messages rapidly (all at once)
        for i in 0..5 {
            message_bus.publish(Message::ContainerLogs {
                app_name: "myapp".to_string(),
                data: format!("message {}", i),
            });
        }

        // Collect messages - they should come through one at a time due to throttling
        let mut events = Vec::new();

        // Collect first 3 non-heartbeat messages with timeouts
        // With throttling at 100ms, these should take at least 200ms total
        let start = std::time::Instant::now();

        while events.len() < 3 {
            if let Ok(Some(Ok(event))) =
                tokio::time::timeout(Duration::from_millis(500), stream.next()).await
            {
                let event_str = format!("{:?}", event);
                // Skip heartbeats
                if !event_str.contains("Heartbeat") {
                    events.push(event);
                }
            } else {
                panic!(
                    "Expected to receive 3 messages, but only got {}",
                    events.len()
                );
            }
        }

        let elapsed = start.elapsed();

        // Verify we received 3 messages
        assert_eq!(events.len(), 3, "Should receive exactly 3 messages");

        // Verify messages are in order (FIFO) - this is the key test
        for (i, event) in events.iter().enumerate() {
            let event_str = format!("{:?}", event);
            assert!(
                event_str.contains(&format!("message {}", i)),
                "Messages should be in order (FIFO), but message {} not found in event. Events: {:?}",
                i,
                events
            );
        }

        // Verify that messages didn't all come through instantly
        // With throttling at 100ms per message, 3 messages should take at least 200ms
        // (first message might be processed on first tick, then 2 more at 100ms intervals)
        assert!(
            elapsed >= Duration::from_millis(150),
            "Messages should be throttled - 3 messages should take at least ~200ms with 100ms throttle, but took {:?}",
            elapsed
        );
    }

    #[tokio::test]
    async fn test_heartbeat_bypasses_throttle() {
        use crate::message_bus::MessageBus;
        let message_bus = Arc::new(MessageBus::new());
        let filter = SubscriptionFilter::new(Some("1".to_string()));

        let mut stream = Box::pin(start_sse_stream(
            message_bus.clone(),
            filter,
            "1".to_string(),
        ));

        // Publish many messages to fill the throttle queue
        for i in 0..10 {
            message_bus.publish(Message::ContainerLogs {
                app_name: "myapp".to_string(),
                data: format!("message {}", i),
            });
        }

        // Wait for heartbeat (should come within 15 seconds, but we'll wait up to 20)
        // Heartbeat should arrive even if message queue is full
        let heartbeat_event = tokio::time::timeout(Duration::from_secs(20), async {
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

        let event_str = format!("{:?}", heartbeat_event);
        assert!(
            event_str.contains("Heartbeat"),
            "Heartbeat should bypass throttle"
        );
    }
}
