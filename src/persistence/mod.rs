//! Async persistence interface backed by the private synchronous SQLite module.

use std::path::Path;
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
#[cfg(test)]
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;

use crate::message::{ConversationSummary, Message, MessageFilter, MessageSource, MessageStatus};
use crate::storage::{ForwardAttemptSample, MessageStore, NewMessage};

mod delivery;

pub use delivery::{
    ClaimedDelivery, CompleteDelivery, CompletionResult, DeliveryAttempt, DeliveryAttemptOutcome,
    DeliveryClaim, DeliveryDisposition, DeliveryTime,
};

const MODEM_FINGERPRINT_META_KEY: &str = "modem_fingerprint";

fn outbound_phase_to_str(phase: OutboundPhase) -> &'static str {
    match phase {
        OutboundPhase::Created => "created",
        OutboundPhase::Prepared => "prepared",
        OutboundPhase::SendStarted => "send_started",
        OutboundPhase::Uncertain => "uncertain",
        OutboundPhase::Unknown => "unknown",
    }
}

fn parse_outbound_phase(value: &str) -> Result<OutboundPhase> {
    match value {
        "created" => Ok(OutboundPhase::Created),
        "prepared" => Ok(OutboundPhase::Prepared),
        "send_started" => Ok(OutboundPhase::SendStarted),
        "uncertain" => Ok(OutboundPhase::Uncertain),
        "unknown" => Ok(OutboundPhase::Unknown),
        _ => anyhow::bail!("unknown outbound phase: {value}"),
    }
}

fn transition_from(result: (Message, bool)) -> OutboundTransition {
    if result.1 {
        OutboundTransition::Applied(result.0)
    } else {
        OutboundTransition::OwnershipLost(result.0)
    }
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboundPhase {
    Created,
    Prepared,
    SendStarted,
    Uncertain,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundClaim {
    pub message: Message,
    pub phase: OutboundPhase,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreateOutboundOutcome {
    Created(Message),
    Existing(Message),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutboundTransition {
    Applied(Message),
    OwnershipLost(Message),
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
    const OUTBOUND_LEASE: Duration = Duration::from_secs(90);

    pub async fn open(path: &Path) -> Result<Self> {
        let path = path.to_path_buf();
        let sqlite = tokio::task::spawn_blocking(move || MessageStore::open(&path)).await??;
        Ok(sqlite.into())
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        Ok(MessageStore::open_in_memory()?.into())
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

    pub async fn create_claimed_outbound(
        &self,
        phone_number: String,
        body: String,
        source: MessageSource,
        idempotency_key: Option<String>,
        owner: String,
    ) -> Result<CreateOutboundOutcome> {
        let result = self
            .run(move |sqlite| {
                let now = time::OffsetDateTime::now_utc()
                    .format(&time::format_description::well_known::Rfc3339)?;
                sqlite.create_or_get_outbound(
                    NewMessage {
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
                    },
                    idempotency_key.as_deref(),
                    &owner,
                    Self::OUTBOUND_LEASE,
                )
            })
            .await?;
        #[cfg(test)]
        if result.1 {
            let pause = self.outbound_creation_pause.lock().unwrap().take();
            if let Some(pause) = pause {
                pause.committed.notify_one();
                pause.release.notified().await;
            }
        }
        Ok(if result.1 {
            CreateOutboundOutcome::Created(result.0)
        } else {
            CreateOutboundOutcome::Existing(result.0)
        })
    }

    pub async fn claim_due_outbound(&self, owner: String) -> Result<Option<OutboundClaim>> {
        let result = self
            .run(move |sqlite| sqlite.claim_due_outbound(&owner, Self::OUTBOUND_LEASE))
            .await?;
        result
            .map(|(message, phase)| {
                Ok(OutboundClaim {
                    message,
                    phase: parse_outbound_phase(&phase)?,
                })
            })
            .transpose()
    }

    pub async fn set_outbound_prepared(
        &self,
        id: i64,
        owner: String,
        modem_sms_path: String,
    ) -> Result<OutboundTransition> {
        let result = self
            .run(move |sqlite| {
                sqlite.set_outbound_prepared(id, &owner, &modem_sms_path, Self::OUTBOUND_LEASE)
            })
            .await?;
        Ok(transition_from(result))
    }

    pub async fn begin_outbound_send(&self, id: i64, owner: String) -> Result<OutboundTransition> {
        let result = self
            .run(move |sqlite| sqlite.begin_outbound_send(id, &owner, Self::OUTBOUND_LEASE))
            .await?;
        Ok(transition_from(result))
    }

    pub async fn defer_outbound(
        &self,
        id: i64,
        owner: String,
        phase: OutboundPhase,
        error: Option<String>,
        retry_after: Option<Duration>,
    ) -> Result<OutboundTransition> {
        let phase = outbound_phase_to_str(phase);
        let result = self
            .run(move |sqlite| {
                sqlite.defer_outbound(id, &owner, phase, error.as_deref(), retry_after)
            })
            .await?;
        Ok(transition_from(result))
    }

    pub async fn finish_claimed_outbound(
        &self,
        id: i64,
        owner: String,
        status: MessageStatus,
        error: Option<String>,
    ) -> Result<OutboundTransition> {
        if !matches!(status, MessageStatus::Sent | MessageStatus::Failed) {
            anyhow::bail!("outbound terminal status must be sent or failed");
        }
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
        let result = self
            .run(move |sqlite| sqlite.finish_claimed_outbound(id, &owner, status, error.as_deref()))
            .await?;
        Ok(transition_from(result))
    }

    pub async fn claim_pending_outbound_event(&self, owner: String) -> Result<Option<Message>> {
        self.run(move |sqlite| sqlite.claim_pending_outbound_event(&owner, Self::OUTBOUND_LEASE))
            .await
    }

    pub async fn acknowledge_outbound_event(&self, id: i64, owner: String) -> Result<bool> {
        self.run(move |sqlite| sqlite.acknowledge_outbound_event(id, &owner))
            .await
    }

    pub async fn renew_outbound_lease(&self, id: i64, owner: String) -> Result<bool> {
        self.run(move |sqlite| sqlite.renew_outbound_lease(id, &owner, Self::OUTBOUND_LEASE))
            .await
    }

    pub async fn has_pending_outbound(&self) -> Result<bool> {
        self.run(|sqlite| sqlite.has_pending_outbound()).await
    }

    #[cfg(test)]
    pub(crate) async fn expire_outbound_lease(&self, id: i64) -> Result<()> {
        self.run(move |sqlite| sqlite.expire_outbound_lease(id))
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

    pub async fn recover_expired_delivery_leases(&self) -> Result<usize> {
        self.run(move |sqlite| sqlite.recover_expired_leases())
            .await
    }

    pub async fn run_retention(&self, max_age_days: u64, batch_size: u32) -> Result<usize> {
        self.run(move |sqlite| sqlite.run_retention(max_age_days, batch_size))
            .await
    }

    pub async fn prune_outbound_idempotency(&self, max_age_days: u64) -> Result<usize> {
        self.run(move |sqlite| sqlite.prune_outbound_idempotency(max_age_days))
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
    use std::time::Duration;

    use crate::message::MessageFilter;
    use crate::storage::NewMessage;

    use super::{
        produce_messages, CompleteDelivery, CompletionResult, DeliveryAttempt,
        DeliveryAttemptOutcome, DeliveryDisposition, Store,
    };

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
    async fn delivery_claim_exposes_typed_times_and_an_opaque_capability() {
        let store = Store::open_in_memory().unwrap();
        store
            .sqlite()
            .insert_message_with_deliveries(
                NewMessage::inbound("+1", "typed delivery claim"),
                &["bark.primary".to_string()],
            )
            .unwrap();

        let claim = store
            .claim_deliveries(1, Duration::from_secs(90))
            .await
            .unwrap()
            .pop()
            .unwrap();

        assert_eq!(claim.profile_key, "bark.primary");
        assert_eq!(claim.attempt_count, 0);
        assert!(claim.next_attempt_at.is_none());
        assert!(matches!(
            claim.created_at,
            super::DeliveryTime::Valid(created_at)
                if created_at <= time::OffsetDateTime::now_utc()
        ));
        let _opaque_capability = claim.claim;
    }

    #[tokio::test]
    async fn delivery_deadline_preserves_rfc3339_offset_and_precision() {
        let store = Store::open_in_memory().unwrap();
        let message = store
            .sqlite()
            .insert_message_with_deliveries(
                NewMessage::inbound("+1", "precise delivery deadline"),
                &["bark.primary".to_string()],
            )
            .unwrap();
        store
            .sqlite()
            .set_delivery_retry_deadline(message.id, "2026-07-24T08:00:00.123456789+08:00")
            .unwrap();

        let due = store.next_delivery_due().await.unwrap().unwrap();

        assert_eq!(due.unix_timestamp_nanos(), 1_784_851_200_123_456_789);
    }

    #[tokio::test]
    async fn lost_delivery_claim_still_records_the_real_provider_attempt() {
        let store = Store::open_in_memory().unwrap();
        store
            .sqlite()
            .insert_message_with_deliveries(
                NewMessage::inbound("+1", "lost delivery claim"),
                &["bark.primary".to_string()],
            )
            .unwrap();
        let stale = store
            .claim_deliveries(1, Duration::ZERO)
            .await
            .unwrap()
            .pop()
            .unwrap();
        let current = store
            .claim_deliveries(1, Duration::from_secs(90))
            .await
            .unwrap()
            .pop()
            .unwrap();
        let now = time::OffsetDateTime::now_utc();

        let result = store
            .complete_delivery(CompleteDelivery {
                claim: stale.claim,
                disposition: DeliveryDisposition::Succeeded,
                attempt: Some(DeliveryAttempt {
                    started_at: now,
                    completed_at: now + time::Duration::milliseconds(10),
                    latency: Duration::from_millis(10),
                    dispatch_delay: Duration::ZERO,
                    outcome: DeliveryAttemptOutcome::Success,
                    error_code: None,
                }),
            })
            .await
            .unwrap();

        assert_eq!(result, CompletionResult::OwnershipLost);
        assert_eq!(
            store
                .forwarding_attempts("bark.primary".to_string(), 5)
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            store.sqlite().get_delivery(current.id).unwrap().state,
            crate::storage::DeliveryState::InFlight
        );
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
