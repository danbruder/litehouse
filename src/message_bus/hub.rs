use super::message::Message;
use tokio::sync::broadcast;
use tracing::debug;

const CHANNEL_CAPACITY: usize = 1000;

#[derive(Clone)]
pub struct MessageBus {
    sender: broadcast::Sender<Message>,
}

impl MessageBus {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(CHANNEL_CAPACITY);
        Self { sender }
    }

    pub fn publish(&self, message: Message) {
        match self.sender.send(message.clone()) {
            Ok(receiver_count) => {
                debug!(
                    "Published message type={} to {} receivers",
                    message.message_type(),
                    receiver_count
                );
            }
            Err(_) => {
                // No receivers, this is fine
                debug!(
                    "Published message type={} but no receivers",
                    message.message_type()
                );
            }
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Message> {
        self.sender.subscribe()
    }

    pub fn receiver_count(&self) -> usize {
        self.sender.receiver_count()
    }
}

impl Default for MessageBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_publish_and_subscribe() {
        let bus = MessageBus::new();
        let mut rx = bus.subscribe();

        let msg = Message::Heartbeat;
        bus.publish(msg.clone());

        let received = rx.recv().await.unwrap();
        assert!(matches!(received, Message::Heartbeat));
    }

    #[tokio::test]
    async fn test_multiple_subscribers() {
        let bus = MessageBus::new();
        let mut rx1 = bus.subscribe();
        let mut rx2 = bus.subscribe();

        assert_eq!(bus.receiver_count(), 2);

        let msg = Message::SystemNotification {
            level: "info".to_string(),
            message: "test".to_string(),
        };

        bus.publish(msg.clone());

        let received1 = rx1.recv().await.unwrap();
        let received2 = rx2.recv().await.unwrap();

        assert!(matches!(
            received1,
            Message::SystemNotification { .. }
        ));
        assert!(matches!(
            received2,
            Message::SystemNotification { .. }
        ));
    }
}
