//! Async persistence interface backed by the private synchronous SQLite module.

use std::path::Path;
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
#[cfg(test)]
use std::sync::{Arc, Mutex};

use anyhow::Result;

use crate::message::{ConversationSummary, Message, MessageFilter, MessageSource, MessageStatus};
use crate::storage::{ForwardAttemptSample, MessageStore, NewMessage};

const MODEM_FINGERPRINT_META_KEY: &str = "modem_fingerprint";

#[derive(Debug, Clone)]
pub struct InboundMessage {
    pub phone_number: String,
    pub body: String,
    pub timestamp: String,
    pub modem_sms_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InboundOutcome {
    Inserted(Message),
    Duplicate,
}

#[derive(Clone)]
pub struct Store {
    sqlite: MessageStore,
    #[cfg(test)]
    outbound_finalization_failures: Arc<AtomicUsize>,
    #[cfg(test)]
    outbound_creation_pause: Arc<Mutex<Option<OutboundCreationPause>>>,
}

#[cfg(test)]
#[derive(Clone)]
struct OutboundCreationPause {
    committed: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
}

impl From<MessageStore> for Store {
    fn from(sqlite: MessageStore) -> Self {
        Self {
            sqlite,
            #[cfg(test)]
            outbound_finalization_failures: Arc::new(AtomicUsize::new(0)),
            #[cfg(test)]
            outbound_creation_pause: Arc::new(Mutex::new(None)),
        }
    }
}

impl Store {
    pub async fn open(path: &Path) -> Result<Self> {
        let path = path.to_path_buf();
        let sqlite = tokio::task::spawn_blocking(move || MessageStore::open(&path)).await??;
        Ok(sqlite.into())
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        Ok(MessageStore::open_in_memory()?.into())
    }

    /// Transitional seam for the delivery worker. Phase three will move its
    /// queue operations behind this module's async interface as well.
    pub(crate) fn delivery_store(&self) -> MessageStore {
        self.sqlite.clone()
    }

    #[cfg(test)]
    pub(crate) fn sqlite(&self) -> MessageStore {
        self.sqlite.clone()
    }

    async fn run<T, F>(&self, operation: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(MessageStore) -> Result<T> + Send + 'static,
    {
        let sqlite = self.sqlite.clone();
        tokio::task::spawn_blocking(move || operation(sqlite)).await?
    }

    pub async fn receive_inbound(
        &self,
        input: InboundMessage,
        profile_keys: Vec<String>,
    ) -> Result<InboundOutcome> {
        let result = self
            .run(move |sqlite| {
                let fingerprint = sqlite
                    .get_meta(MODEM_FINGERPRINT_META_KEY)?
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| anyhow::anyhow!("modem fingerprint is not enrolled"))?;
                let message = NewMessage::modem_inbound(
                    &input.phone_number,
                    &input.body,
                    &input.timestamp,
                    &input.modem_sms_path,
                    &fingerprint,
                );
                sqlite.insert_inbound_message_with_deliveries(message, &profile_keys)
            })
            .await?;
        Ok(match result {
            crate::storage::InboundInsertResult::Inserted(message) => {
                InboundOutcome::Inserted(message)
            }
            crate::storage::InboundInsertResult::Duplicate(_) => InboundOutcome::Duplicate,
        })
    }

    pub async fn create_outbound(
        &self,
        phone_number: String,
        body: String,
        source: MessageSource,
    ) -> Result<Message> {
        let message = self
            .run(move |sqlite| {
                let now = time::OffsetDateTime::now_utc()
                    .format(&time::format_description::well_known::Rfc3339)?;
                sqlite.insert_message(NewMessage {
                    direction: crate::message::MessageDirection::Outbound,
                    phone_number,
                    body,
                    timestamp: now.clone(),
                    status: MessageStatus::Sending,
                    source,
                    modem_sms_path: None,
                    read_at: Some(now),
                    error: None,
                    inbound_dedupe_key: None,
                })
            })
            .await?;
        #[cfg(test)]
        {
            let pause = self.outbound_creation_pause.lock().unwrap().take();
            if let Some(pause) = pause {
                pause.committed.notify_one();
                pause.release.notified().await;
            }
        }
        Ok(message)
    }

    pub async fn finish_outbound(
        &self,
        id: i64,
        status: MessageStatus,
        error: Option<String>,
    ) -> Result<Message> {
        #[cfg(test)]
        if self
            .outbound_finalization_failures
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                remaining.checked_sub(1)
            })
            .is_ok()
        {
            anyhow::bail!("injected outbound finalization failure");
        }
        self.run(move |sqlite| sqlite.update_status(id, status, error))
            .await
    }

    #[cfg(test)]
    pub(crate) fn fail_next_outbound_finalizations(&self, count: usize) {
        self.outbound_finalization_failures
            .store(count, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(crate) fn pause_next_outbound_creation(
        &self,
    ) -> (Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>) {
        let pause = OutboundCreationPause {
            committed: Arc::new(tokio::sync::Notify::new()),
            release: Arc::new(tokio::sync::Notify::new()),
        };
        *self.outbound_creation_pause.lock().unwrap() = Some(pause.clone());
        (pause.committed, pause.release)
    }

    pub async fn set_outbound_modem_sms_path(
        &self,
        id: i64,
        modem_sms_path: String,
    ) -> Result<Message> {
        self.run(move |sqlite| sqlite.set_outbound_modem_sms_path(id, &modem_sms_path))
            .await
    }

    pub async fn list_messages(&self, filter: MessageFilter) -> Result<Vec<Message>> {
        self.run(move |sqlite| sqlite.list_messages(&filter)).await
    }

    pub async fn sending_outbound(&self) -> Result<Vec<Message>> {
        self.run(|sqlite| sqlite.list_sending_outbound()).await
    }

    pub async fn list_conversations(&self) -> Result<Vec<ConversationSummary>> {
        self.run(|sqlite| sqlite.list_conversations()).await
    }

    pub async fn mark_read(&self, id: i64) -> Result<Message> {
        self.run(move |sqlite| sqlite.mark_read(id)).await
    }

    pub async fn mark_unread(&self, id: i64) -> Result<Message> {
        self.run(move |sqlite| sqlite.mark_unread(id)).await
    }

    pub async fn mark_conversation_read(&self, phone_number: String) -> Result<i64> {
        self.run(move |sqlite| sqlite.mark_conversation_read(&phone_number))
            .await
    }

    pub async fn delete_messages(&self, ids: Vec<i64>) -> Result<()> {
        self.run(move |sqlite| sqlite.delete_messages(&ids)).await
    }

    pub async fn forwarding_profiles(&self) -> Result<Vec<String>> {
        self.run(|sqlite| sqlite.list_forward_attempt_profiles())
            .await
    }

    pub async fn forwarding_attempts(
        &self,
        profile_key: String,
        limit: u32,
    ) -> Result<Vec<ForwardAttemptSample>> {
        self.run(move |sqlite| sqlite.list_forward_attempts(&profile_key, limit))
            .await
    }

    pub async fn health_check(&self) -> Result<()> {
        self.run(|sqlite| sqlite.health_check()).await
    }

    pub async fn modem_fingerprint(&self) -> Result<Option<String>> {
        self.run(|sqlite| sqlite.get_meta(MODEM_FINGERPRINT_META_KEY))
            .await
    }

    pub async fn set_modem_fingerprint(&self, fingerprint: String) -> Result<()> {
        self.run(move |sqlite| sqlite.set_meta(MODEM_FINGERPRINT_META_KEY, &fingerprint))
            .await
    }

    pub async fn backfill_dedupe_keys(&self) -> Result<usize> {
        self.run(|sqlite| sqlite.backfill_dedupe_keys()).await
    }

    pub async fn recover_expired_leases(&self, before: String) -> Result<usize> {
        self.run(move |sqlite| sqlite.recover_expired_leases(&before))
            .await
    }

    pub async fn run_retention(&self, max_age_days: u64, batch_size: u32) -> Result<usize> {
        self.run(move |sqlite| sqlite.run_retention(max_age_days, batch_size))
            .await
    }

    pub fn stream_messages<T, F>(
        &self,
        filter: MessageFilter,
        transform: F,
    ) -> tokio::sync::mpsc::Receiver<Result<T>>
    where
        T: Send + 'static,
        F: FnMut(Message) -> Result<T> + Send + 'static,
    {
        let (sender, receiver) = tokio::sync::mpsc::channel(8);
        let sqlite = self.sqlite.clone();
        tokio::task::spawn_blocking(move || {
            let result = produce_messages(&sqlite, &filter, transform, &sender);
            if let Err(error) = result {
                let _ = sender.blocking_send(Err(error));
            }
        });
        receiver
    }
}

fn produce_messages<T, F>(
    sqlite: &MessageStore,
    filter: &MessageFilter,
    mut transform: F,
    sender: &tokio::sync::mpsc::Sender<Result<T>>,
) -> Result<()>
where
    T: Send + 'static,
    F: FnMut(Message) -> Result<T>,
{
    sqlite.for_each_export_message(filter, |message| {
        let item = transform(message)?;
        Ok(sender.blocking_send(Ok(item)).is_ok())
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use crate::message::MessageFilter;
    use crate::storage::NewMessage;

    use super::{produce_messages, Store};

    #[tokio::test]
    async fn open_initializes_storage_through_the_async_interface() {
        let path = std::env::temp_dir().join(format!(
            "sms-relayed-async-open-{}.sqlite",
            uuid::Uuid::new_v4()
        ));

        let store = Store::open(&path).await.unwrap();

        store.health_check().await.unwrap();
        drop(store);
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn export_producer_stops_when_the_consumer_disconnects() {
        let sqlite = Store::open_in_memory().unwrap().sqlite();
        for index in 0..10 {
            sqlite
                .insert_message(NewMessage::inbound("+1", &format!("message-{index}")))
                .unwrap();
        }
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        drop(receiver);
        let transformed = Arc::new(AtomicUsize::new(0));
        let transformed_in_worker = transformed.clone();
        let worker = tokio::task::spawn_blocking(move || {
            produce_messages(
                &sqlite,
                &MessageFilter::default(),
                move |message| {
                    transformed_in_worker.fetch_add(1, Ordering::SeqCst);
                    Ok(message)
                },
                &sender,
            )
        });

        tokio::time::timeout(std::time::Duration::from_secs(1), worker)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(transformed.load(Ordering::SeqCst), 1);
    }
}
