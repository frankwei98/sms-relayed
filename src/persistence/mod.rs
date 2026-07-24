//! Async persistence interface backed by the private synchronous SQLite module.

use std::path::Path;

use anyhow::Result;

use crate::message::{ConversationSummary, Message, MessageFilter, MessageSource, MessageStatus};
use crate::storage::{ForwardAttemptSample, InboundInsertResult, MessageStore, NewMessage};

const MODEM_FINGERPRINT_META_KEY: &str = "modem_fingerprint";

#[derive(Debug, Clone)]
pub struct InboundMessage {
    pub phone_number: String,
    pub body: String,
    pub timestamp: String,
    pub modem_sms_path: String,
}

#[derive(Clone)]
pub struct Store {
    sqlite: MessageStore,
}

impl From<MessageStore> for Store {
    fn from(sqlite: MessageStore) -> Self {
        Self { sqlite }
    }
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        Ok(Self {
            sqlite: MessageStore::open(path)?,
        })
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        Ok(Self {
            sqlite: MessageStore::open_in_memory()?,
        })
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
    ) -> Result<InboundInsertResult> {
        self.run(move |sqlite| {
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
        .await
    }

    pub async fn create_outbound(
        &self,
        phone_number: String,
        body: String,
        source: MessageSource,
    ) -> Result<Message> {
        self.run(move |sqlite| {
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
        .await
    }

    pub async fn finish_outbound(
        &self,
        id: i64,
        status: MessageStatus,
        error: Option<String>,
    ) -> Result<Message> {
        self.run(move |sqlite| sqlite.update_status(id, status, error))
            .await
    }

    pub async fn list_messages(&self, filter: MessageFilter) -> Result<Vec<Message>> {
        self.run(move |sqlite| sqlite.list_messages(&filter)).await
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

    pub fn stream_messages(
        &self,
        filter: MessageFilter,
    ) -> tokio::sync::mpsc::Receiver<Result<Message>> {
        let (sender, receiver) = tokio::sync::mpsc::channel(8);
        let sqlite = self.sqlite.clone();
        tokio::task::spawn_blocking(move || {
            let result = sqlite.for_each_export_message(&filter, |message| {
                Ok(sender.blocking_send(Ok(message)).is_ok())
            });
            if let Err(error) = result {
                let _ = sender.blocking_send(Err(error));
            }
        });
        receiver
    }
}
