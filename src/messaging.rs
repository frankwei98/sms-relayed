use std::sync::Arc;

use crate::dbus::{ReceivedSms, SmsSender};
use crate::delivery::DeliveryWakeup;
use crate::events::{AppEvent, EventBus};
use crate::message::{ConversationSummary, Message, MessageFilter, MessageSource, MessageStatus};
use crate::persistence::{InboundMessage, Store};
use crate::storage::InboundInsertResult;

pub struct SendMessage {
    pub modem_path: String,
    pub phone_number: String,
    pub body: String,
    pub source: MessageSource,
}

#[derive(Clone)]
pub struct ReceiveMessage {
    pub sms: ReceivedSms,
    pub profile_keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceiveOutcome {
    Inserted(Message),
    Duplicate,
}

#[derive(Clone)]
pub struct Messaging {
    store: Store,
    events: EventBus,
    delivery_wakeup: DeliveryWakeup,
    sms_sender: Arc<dyn SmsSender>,
}

impl Messaging {
    pub fn new(
        store: Store,
        events: EventBus,
        delivery_wakeup: DeliveryWakeup,
        sms_sender: Arc<dyn SmsSender>,
    ) -> Self {
        Self {
            store,
            events,
            delivery_wakeup,
            sms_sender,
        }
    }

    pub async fn send(&self, request: SendMessage) -> anyhow::Result<Message> {
        let SendMessage {
            modem_path,
            phone_number,
            body,
            source,
        } = request;
        let message = self
            .store
            .create_outbound(phone_number.clone(), body.clone(), source)
            .await?;
        self.events.send(AppEvent::MessageCreated(message.clone()));

        let (status, error) = match self
            .sms_sender
            .send(&modem_path, &phone_number, &body)
            .await
        {
            Ok(_) => (MessageStatus::Sent, None),
            Err(error) => (MessageStatus::Failed, Some(error.to_string())),
        };
        let updated = self
            .store
            .finish_outbound(message.id, status, error)
            .await?;
        self.events.send(AppEvent::MessageUpdated(updated.clone()));
        Ok(updated)
    }

    pub async fn receive(&self, request: ReceiveMessage) -> anyhow::Result<ReceiveOutcome> {
        let ReceiveMessage {
            sms,
            mut profile_keys,
        } = request;
        profile_keys.sort();
        profile_keys.dedup();
        let has_profiles = !profile_keys.is_empty();
        let insert_result = self
            .store
            .receive_inbound(
                InboundMessage {
                    phone_number: sms.phone_number,
                    body: sms.body,
                    timestamp: sms.timestamp,
                    modem_sms_path: sms.modem_sms_path,
                },
                profile_keys,
            )
            .await?;

        match insert_result {
            InboundInsertResult::Inserted(message) => {
                if has_profiles {
                    self.delivery_wakeup.notify();
                }
                self.events.send(AppEvent::MessageCreated(message.clone()));
                Ok(ReceiveOutcome::Inserted(message))
            }
            InboundInsertResult::Duplicate(_) => Ok(ReceiveOutcome::Duplicate),
        }
    }

    pub async fn list(&self, filter: MessageFilter) -> anyhow::Result<Vec<Message>> {
        self.store.list_messages(filter).await
    }

    pub async fn conversations(&self) -> anyhow::Result<Vec<ConversationSummary>> {
        self.store.list_conversations().await
    }

    pub async fn set_read(&self, id: i64, read: bool) -> anyhow::Result<Message> {
        let message = if read {
            self.store.mark_read(id).await?
        } else {
            self.store.mark_unread(id).await?
        };
        self.events
            .send(AppEvent::MessageReadStateChanged(message.clone()));
        Ok(message)
    }

    pub async fn mark_conversation_read(&self, phone_number: String) -> anyhow::Result<i64> {
        let changed = self.store.mark_conversation_read(phone_number).await?;
        self.events.send(AppEvent::ConversationRead);
        Ok(changed)
    }

    pub async fn delete(&self, ids: Vec<i64>) -> anyhow::Result<()> {
        self.store.delete_messages(ids.clone()).await?;
        self.events.send(AppEvent::MessageDeleted { ids });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::api::test_sms_sender;
    use crate::delivery::DeliveryWakeup;
    use crate::events::{AppEvent, EventBus};
    use crate::message::{MessageSource, MessageStatus};
    use crate::persistence::Store;

    use super::{Messaging, ReceiveMessage, ReceiveOutcome, SendMessage};

    #[tokio::test]
    async fn outbound_send_persists_one_state_machine_and_emits_both_events() {
        let store = Store::open_in_memory().unwrap();
        let events = EventBus::new();
        let mut received_events = events.subscribe();
        let messaging = Messaging::new(
            store.clone(),
            events,
            DeliveryWakeup::new(),
            test_sms_sender(),
        );

        let message = messaging
            .send(SendMessage {
                modem_path: "/org/freedesktop/ModemManager1/Modem/0".to_string(),
                phone_number: "+15550000000".to_string(),
                body: "shared outbound flow".to_string(),
                source: MessageSource::Web,
            })
            .await
            .unwrap();

        assert_eq!(message.status, MessageStatus::Sent);
        assert_eq!(store.sqlite().count_messages().unwrap(), 1);
        assert!(matches!(
            received_events.recv().await.unwrap(),
            AppEvent::MessageCreated(_)
        ));
        assert!(matches!(
            received_events.recv().await.unwrap(),
            AppEvent::MessageUpdated(_)
        ));
    }

    #[tokio::test]
    async fn duplicate_inbound_is_persisted_and_announced_only_once() {
        let store = Store::open_in_memory().unwrap();
        store
            .set_modem_fingerprint("test-fingerprint".to_string())
            .await
            .unwrap();
        let events = EventBus::new();
        let mut received_events = events.subscribe();
        let wakeup = DeliveryWakeup::new();
        let messaging = Messaging::new(store.clone(), events, wakeup.clone(), test_sms_sender());
        let request = ReceiveMessage {
            sms: crate::dbus::ReceivedSms {
                phone_number: "+15550000000".to_string(),
                body: "one inbound message".to_string(),
                timestamp: "2026-07-24T00:00:00Z".to_string(),
                modem_sms_path: "/org/freedesktop/ModemManager1/SMS/1".to_string(),
            },
            profile_keys: vec!["bark.primary".to_string()],
        };

        assert!(matches!(
            messaging.receive(request.clone()).await.unwrap(),
            ReceiveOutcome::Inserted(_)
        ));
        assert!(matches!(
            messaging.receive(request).await.unwrap(),
            ReceiveOutcome::Duplicate
        ));

        assert_eq!(store.sqlite().count_messages().unwrap(), 1);
        assert_eq!(store.sqlite().count_deliveries().unwrap(), 1);
        assert!(matches!(
            received_events.recv().await.unwrap(),
            AppEvent::MessageCreated(_)
        ));
        assert!(received_events.try_recv().is_err());
        tokio::time::timeout(std::time::Duration::from_millis(50), wakeup.wait())
            .await
            .expect("the inserted delivery should wake the worker");
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), wakeup.wait())
                .await
                .is_err(),
            "the duplicate must not wake the worker"
        );
    }

    #[tokio::test]
    async fn inbound_without_profiles_does_not_wake_the_delivery_worker() {
        let store = Store::open_in_memory().unwrap();
        store
            .set_modem_fingerprint("test-fingerprint".to_string())
            .await
            .unwrap();
        let wakeup = DeliveryWakeup::new();
        let messaging = Messaging::new(
            store.clone(),
            EventBus::new(),
            wakeup.clone(),
            test_sms_sender(),
        );

        messaging
            .receive(ReceiveMessage {
                sms: crate::dbus::ReceivedSms {
                    phone_number: "+15550000000".to_string(),
                    body: "store only".to_string(),
                    timestamp: "2026-07-24T00:01:00Z".to_string(),
                    modem_sms_path: "/org/freedesktop/ModemManager1/SMS/2".to_string(),
                },
                profile_keys: Vec::new(),
            })
            .await
            .unwrap();

        assert_eq!(store.sqlite().count_messages().unwrap(), 1);
        assert_eq!(store.sqlite().count_deliveries().unwrap(), 0);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), wakeup.wait())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn inbound_without_an_enrolled_fingerprint_is_not_persisted() {
        let store = Store::open_in_memory().unwrap();
        let messaging = Messaging::new(
            store.clone(),
            EventBus::new(),
            DeliveryWakeup::new(),
            test_sms_sender(),
        );

        let result = messaging
            .receive(ReceiveMessage {
                sms: crate::dbus::ReceivedSms {
                    phone_number: "+15550000000".to_string(),
                    body: "wait for enrollment".to_string(),
                    timestamp: "2026-07-24T00:02:00Z".to_string(),
                    modem_sms_path: "/org/freedesktop/ModemManager1/SMS/3".to_string(),
                },
                profile_keys: Vec::new(),
            })
            .await;

        assert!(result.is_err());
        assert_eq!(store.sqlite().count_messages().unwrap(), 0);
    }
}
