use super::message::SSEMessage;
use tokio::sync::broadcast;
use tracing::debug;

const CHANNEL_CAPACITY: usize = 1000;

#[derive(Clone)]
pub struct SSEHub {
    sender: broadcast::Sender<SSEMessage>,
}

impl SSEHub {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(CHANNEL_CAPACITY);
        Self { sender }
    }

    pub fn publish(&self, message: SSEMessage) {
        match self.sender.send(message.clone()) {
            Ok(receiver_count) => {
                debug!(
                    "Published SSE message type={} to {} receivers",
                    message.message_type(),
                    receiver_count
                );
            }
            Err(_) => {
                // No receivers, this is fine
                debug!(
                    "Published SSE message type={} but no receivers",
                    message.message_type()
                );
            }
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SSEMessage> {
        self.sender.subscribe()
    }

    pub fn receiver_count(&self) -> usize {
        self.sender.receiver_count()
    }
}

impl Default for SSEHub {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_publish_and_subscribe() {
        let hub = SSEHub::new();
        let mut rx = hub.subscribe();

        let msg = SSEMessage::Heartbeat;
        hub.publish(msg.clone());

        let received = rx.recv().await.unwrap();
        assert!(matches!(received, SSEMessage::Heartbeat));
    }

    #[tokio::test]
    async fn test_multiple_subscribers() {
        let hub = SSEHub::new();
        let mut rx1 = hub.subscribe();
        let mut rx2 = hub.subscribe();

        assert_eq!(hub.receiver_count(), 2);

        let msg = SSEMessage::SystemNotification {
            level: "info".to_string(),
            message: "test".to_string(),
        };

        hub.publish(msg.clone());

        let received1 = rx1.recv().await.unwrap();
        let received2 = rx2.recv().await.unwrap();

        assert!(matches!(
            received1,
            SSEMessage::SystemNotification { .. }
        ));
        assert!(matches!(
            received2,
            SSEMessage::SystemNotification { .. }
        ));
    }
}
