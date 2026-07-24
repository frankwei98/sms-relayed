use std::sync::Arc;
use std::time::Duration;

use crate::dbus::{ReceivedSms, SmsSender};
use crate::delivery::DeliveryWakeup;
use crate::events::{AppEvent, EventBus};
use crate::message::{ConversationSummary, Message, MessageFilter, MessageSource, MessageStatus};
use crate::persistence::{InboundMessage, InboundOutcome, Store};

pub struct SendMessage {
    pub modem_path: String,
    pub phone_number: String,
    pub body: String,
    pub source: MessageSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendOutcome {
    Sent(Message),
    Failed(Message),
}

impl SendOutcome {
    pub fn into_message(self) -> Message {
        match self {
            Self::Sent(message) | Self::Failed(message) => message,
        }
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResolvedModemSmsState {
    Stored,
    Sent,
    Unknown,
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

    /// The detached finalizer owns the complete send, including creation of
    /// the local `sending` row. Dropping the caller only drops this join
    /// handle; it does not cancel persistence, the modem operation, or durable
    /// finalization.
    ///
    /// A transport rejection is returned as `SendOutcome::Failed` after that
    /// state is persisted. `Err` means the finalizer task itself panicked or
    /// was aborted by runtime shutdown; startup recovery resumes such rows.
    pub async fn send(&self, request: SendMessage) -> anyhow::Result<SendOutcome> {
        let messaging = self.clone();
        let finalizer =
            tokio::spawn(async move { messaging.create_and_run_outbound(request).await });
        finalizer
            .await
            .map_err(|error| anyhow::anyhow!("outbound finalizer task failed: {error}"))?
    }

    async fn create_and_run_outbound(&self, request: SendMessage) -> anyhow::Result<SendOutcome> {
        let SendMessage {
            ref phone_number,
            ref body,
            source,
            ..
        } = request;
        let message = self
            .store
            .create_outbound(phone_number.clone(), body.clone(), source)
            .await?;
        self.events.send(AppEvent::MessageCreated(message.clone()));
        self.run_outbound(message.id, request).await
    }

    async fn run_outbound(
        &self,
        message_id: i64,
        request: SendMessage,
    ) -> anyhow::Result<SendOutcome> {
        let (status, error) = match self
            .sms_sender
            .prepare(&request.modem_path, &request.phone_number, &request.body)
            .await
        {
            Ok(prepared) => {
                self.persist_modem_path(message_id, prepared.modem_sms_path.clone())
                    .await;
                self.send_prepared_and_reconcile(&prepared.modem_sms_path)
                    .await
            }
            Err(error) => (MessageStatus::Failed, Some(error.to_string())),
        };
        Ok(self.finish_outbound(message_id, status, error).await)
    }

    async fn persist_modem_path(&self, message_id: i64, modem_sms_path: String) {
        let mut delay = Duration::from_millis(100);
        loop {
            match self
                .store
                .set_outbound_modem_sms_path(message_id, modem_sms_path.clone())
                .await
            {
                Ok(_) => return,
                Err(error) => {
                    log::error!(
                        "persisting outbound modem path failed; message_id={message_id}: {error}"
                    );
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(Duration::from_secs(30));
                }
            }
        }
    }

    async fn send_prepared_and_reconcile(
        &self,
        modem_sms_path: &str,
    ) -> (MessageStatus, Option<String>) {
        match self.sms_sender.send_prepared(modem_sms_path).await {
            Ok(()) => (MessageStatus::Sent, None),
            Err(error) => match self.resolve_modem_sms_state(modem_sms_path).await {
                ResolvedModemSmsState::Sent => (MessageStatus::Sent, None),
                ResolvedModemSmsState::Stored => (MessageStatus::Failed, Some(error.to_string())),
                ResolvedModemSmsState::Unknown => (
                    MessageStatus::Failed,
                    Some(format!(
                        "delivery outcome unknown; automatic retry suppressed: {error}"
                    )),
                ),
            },
        }
    }

    async fn resolve_modem_sms_state(&self, modem_sms_path: &str) -> ResolvedModemSmsState {
        let mut delay = Duration::from_millis(100);
        let mut stored_observations = 0;
        loop {
            match self.sms_sender.sms_state(modem_sms_path).await {
                Ok(crate::dbus::ModemSmsState::Sent) => return ResolvedModemSmsState::Sent,
                Ok(crate::dbus::ModemSmsState::Stored) => {
                    stored_observations += 1;
                    if stored_observations >= 3 {
                        return ResolvedModemSmsState::Stored;
                    }
                }
                Ok(crate::dbus::ModemSmsState::Unknown) => {
                    return ResolvedModemSmsState::Unknown;
                }
                Ok(crate::dbus::ModemSmsState::Sending) => stored_observations = 0,
                Err(error) => {
                    stored_observations = 0;
                    log::warn!("reading outbound modem state failed: {error}");
                }
            }
            tokio::time::sleep(delay).await;
            delay = (delay * 2).min(Duration::from_secs(5));
        }
    }

    async fn finish_outbound(
        &self,
        message_id: i64,
        status: MessageStatus,
        error: Option<String>,
    ) -> SendOutcome {
        let mut delay = Duration::from_millis(100);
        loop {
            match self
                .store
                .finish_outbound(message_id, status, error.clone())
                .await
            {
                Ok(updated) => {
                    self.events.send(AppEvent::MessageUpdated(updated.clone()));
                    return match updated.status {
                        MessageStatus::Sent => SendOutcome::Sent(updated),
                        MessageStatus::Failed => SendOutcome::Failed(updated),
                        _ => unreachable!("outbound send must finish as sent or failed"),
                    };
                }
                Err(update_error) => {
                    log::error!(
                        "finalizing outbound message failed; message_id={message_id}: {update_error}"
                    );
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(Duration::from_secs(30));
                }
            }
        }
    }

    /// Take a startup snapshot before the service accepts new sends, then
    /// recover each pre-existing row independently. New rows can therefore
    /// never be mistaken for interrupted work, and one unavailable modem
    /// object cannot hold up recovery of the others.
    pub async fn start_outbound_recovery(&self) -> anyhow::Result<usize> {
        let messages = self.store.sending_outbound().await?;
        let count = messages.len();
        for message in messages {
            let messaging = self.clone();
            tokio::spawn(async move {
                messaging.recover_outbound_message(message).await;
            });
        }
        Ok(count)
    }

    #[cfg(test)]
    async fn recover_outbound(&self) -> anyhow::Result<usize> {
        let messages = self.store.sending_outbound().await?;
        let count = messages.len();
        for message in messages {
            self.recover_outbound_message(message).await;
        }
        Ok(count)
    }

    async fn recover_outbound_message(&self, message: Message) {
        let Some(modem_sms_path) = message.modem_sms_path else {
            self.finish_outbound(
                message.id,
                MessageStatus::Failed,
                Some("outbound interrupted before modem preparation".to_string()),
            )
            .await;
            return;
        };

        let (status, error) = match self.resolve_modem_sms_state(&modem_sms_path).await {
            ResolvedModemSmsState::Sent => (MessageStatus::Sent, None),
            ResolvedModemSmsState::Stored => {
                self.send_prepared_and_reconcile(&modem_sms_path).await
            }
            ResolvedModemSmsState::Unknown => (
                MessageStatus::Failed,
                Some(
                    "delivery outcome unknown during startup recovery; automatic retry suppressed"
                        .to_string(),
                ),
            ),
        };
        self.finish_outbound(message.id, status, error).await;
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
            InboundOutcome::Inserted(message) => {
                if has_profiles {
                    self.delivery_wakeup.notify();
                }
                self.events.send(AppEvent::MessageCreated(message.clone()));
                Ok(ReceiveOutcome::Inserted(message))
            }
            InboundOutcome::Duplicate => Ok(ReceiveOutcome::Duplicate),
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
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use crate::api::test_sms_sender;
    use crate::dbus::{ModemSmsState, PreparedSms, SmsSender};
    use crate::delivery::DeliveryWakeup;
    use crate::events::{AppEvent, EventBus};
    use crate::message::{MessageFilter, MessageSource, MessageStatus};
    use crate::persistence::Store;

    use super::{Messaging, ReceiveMessage, ReceiveOutcome, SendMessage, SendOutcome};

    struct PathCheckingSender {
        store: Store,
        observed_persisted_path: Arc<AtomicBool>,
    }

    struct BlockingSender {
        send_started: Arc<tokio::sync::Notify>,
        release_send: Arc<tokio::sync::Notify>,
    }

    struct RecoverySender {
        send_calls: Arc<std::sync::atomic::AtomicUsize>,
        state: ModemSmsState,
    }

    struct AmbiguousSender;

    impl SmsSender for AmbiguousSender {
        fn prepare<'a>(
            &'a self,
            _modem_path: &'a str,
            _tel_number: &'a str,
            _sms_text: &'a str,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<PreparedSms>> + Send + 'a>> {
            Box::pin(async {
                Ok(PreparedSms {
                    modem_sms_path: "/org/freedesktop/ModemManager1/SMS/ambiguous".to_string(),
                })
            })
        }

        fn send_prepared<'a>(
            &'a self,
            _modem_sms_path: &'a str,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
            Box::pin(async { Err(anyhow::anyhow!("send reply timed out")) })
        }

        fn sms_state<'a>(
            &'a self,
            _modem_sms_path: &'a str,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<ModemSmsState>> + Send + 'a>> {
            Box::pin(async { Ok(ModemSmsState::Sent) })
        }
    }

    struct SendingRecoverySender {
        state_reads: Arc<std::sync::atomic::AtomicUsize>,
        send_calls: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl SmsSender for SendingRecoverySender {
        fn prepare<'a>(
            &'a self,
            _modem_path: &'a str,
            _tel_number: &'a str,
            _sms_text: &'a str,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<PreparedSms>> + Send + 'a>> {
            Box::pin(async { unreachable!("recovery must not prepare") })
        }

        fn send_prepared<'a>(
            &'a self,
            _modem_sms_path: &'a str,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
            let send_calls = self.send_calls.clone();
            Box::pin(async move {
                send_calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        }

        fn sms_state<'a>(
            &'a self,
            _modem_sms_path: &'a str,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<ModemSmsState>> + Send + 'a>> {
            let reads = self.state_reads.clone();
            Box::pin(async move {
                if reads.fetch_add(1, Ordering::SeqCst) == 0 {
                    Ok(ModemSmsState::Sending)
                } else {
                    Ok(ModemSmsState::Sent)
                }
            })
        }
    }

    impl SmsSender for RecoverySender {
        fn prepare<'a>(
            &'a self,
            _modem_path: &'a str,
            _tel_number: &'a str,
            _sms_text: &'a str,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<PreparedSms>> + Send + 'a>> {
            Box::pin(async { unreachable!("recovery must not create a new modem SMS") })
        }

        fn send_prepared<'a>(
            &'a self,
            _modem_sms_path: &'a str,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
            let send_calls = self.send_calls.clone();
            Box::pin(async move {
                send_calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        }

        fn sms_state<'a>(
            &'a self,
            _modem_sms_path: &'a str,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<ModemSmsState>> + Send + 'a>> {
            let state = self.state;
            Box::pin(async move { Ok(state) })
        }
    }

    impl SmsSender for BlockingSender {
        fn prepare<'a>(
            &'a self,
            _modem_path: &'a str,
            _tel_number: &'a str,
            _sms_text: &'a str,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<PreparedSms>> + Send + 'a>> {
            Box::pin(async {
                Ok(PreparedSms {
                    modem_sms_path: "/org/freedesktop/ModemManager1/SMS/blocked".to_string(),
                })
            })
        }

        fn send_prepared<'a>(
            &'a self,
            _modem_sms_path: &'a str,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
            let send_started = self.send_started.clone();
            let release_send = self.release_send.clone();
            Box::pin(async move {
                send_started.notify_one();
                release_send.notified().await;
                Ok(())
            })
        }

        fn sms_state<'a>(
            &'a self,
            _modem_sms_path: &'a str,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<ModemSmsState>> + Send + 'a>> {
            Box::pin(async { Ok(ModemSmsState::Sent) })
        }
    }

    impl SmsSender for PathCheckingSender {
        fn prepare<'a>(
            &'a self,
            _modem_path: &'a str,
            _tel_number: &'a str,
            _sms_text: &'a str,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<PreparedSms>> + Send + 'a>> {
            Box::pin(async {
                Ok(PreparedSms {
                    modem_sms_path: "/org/freedesktop/ModemManager1/SMS/prepared".to_string(),
                })
            })
        }

        fn send_prepared<'a>(
            &'a self,
            modem_sms_path: &'a str,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
            let store = self.store.clone();
            let observed = self.observed_persisted_path.clone();
            let modem_sms_path = modem_sms_path.to_string();
            Box::pin(async move {
                let messages = store.list_messages(MessageFilter::default()).await?;
                observed.store(
                    messages[0].modem_sms_path.as_deref() == Some(modem_sms_path.as_str()),
                    Ordering::SeqCst,
                );
                Ok(())
            })
        }

        fn sms_state<'a>(
            &'a self,
            _modem_sms_path: &'a str,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<ModemSmsState>> + Send + 'a>> {
            Box::pin(async { Ok(ModemSmsState::Sent) })
        }
    }

    struct FailingSmsSender;

    impl SmsSender for FailingSmsSender {
        fn prepare<'a>(
            &'a self,
            _modem_path: &'a str,
            _tel_number: &'a str,
            _sms_text: &'a str,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<PreparedSms>> + Send + 'a>> {
            Box::pin(async { Err(anyhow::anyhow!("system bus unavailable")) })
        }

        fn send_prepared<'a>(
            &'a self,
            _modem_sms_path: &'a str,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
            Box::pin(async { unreachable!("prepare failed") })
        }

        fn sms_state<'a>(
            &'a self,
            _modem_sms_path: &'a str,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<ModemSmsState>> + Send + 'a>> {
            Box::pin(async { Ok(ModemSmsState::Unknown) })
        }
    }

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

        let outcome = messaging
            .send(SendMessage {
                modem_path: "/org/freedesktop/ModemManager1/Modem/0".to_string(),
                phone_number: "+15550000000".to_string(),
                body: "shared outbound flow".to_string(),
                source: MessageSource::Web,
            })
            .await
            .unwrap();

        let SendOutcome::Sent(message) = outcome else {
            panic!("successful transport must produce a sent outcome");
        };
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
    async fn outbound_persists_the_modem_path_before_sending() {
        let store = Store::open_in_memory().unwrap();
        let observed_persisted_path = Arc::new(AtomicBool::new(false));
        let messaging = Messaging::new(
            store.clone(),
            EventBus::new(),
            DeliveryWakeup::new(),
            Arc::new(PathCheckingSender {
                store,
                observed_persisted_path: observed_persisted_path.clone(),
            }),
        );

        let outcome = messaging
            .send(SendMessage {
                modem_path: "/org/freedesktop/ModemManager1/Modem/0".to_string(),
                phone_number: "+15550000000".to_string(),
                body: "durable outbound".to_string(),
                source: MessageSource::Web,
            })
            .await
            .unwrap();

        assert!(matches!(outcome, SendOutcome::Sent(_)));
        assert!(observed_persisted_path.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn sending_outbound_message_cannot_be_deleted() {
        let store = Store::open_in_memory().unwrap();
        let send_started = Arc::new(tokio::sync::Notify::new());
        let release_send = Arc::new(tokio::sync::Notify::new());
        let messaging = Messaging::new(
            store,
            EventBus::new(),
            DeliveryWakeup::new(),
            Arc::new(BlockingSender {
                send_started: send_started.clone(),
                release_send: release_send.clone(),
            }),
        );
        let send_task = tokio::spawn({
            let messaging = messaging.clone();
            async move {
                messaging
                    .send(SendMessage {
                        modem_path: "/org/freedesktop/ModemManager1/Modem/0".to_string(),
                        phone_number: "+15550000000".to_string(),
                        body: "cannot delete while sending".to_string(),
                        source: MessageSource::Web,
                    })
                    .await
            }
        });
        tokio::time::timeout(std::time::Duration::from_secs(1), send_started.notified())
            .await
            .unwrap();
        let message = messaging
            .list(MessageFilter::default())
            .await
            .unwrap()
            .remove(0);

        assert!(messaging.delete(vec![message.id]).await.is_err());

        release_send.notify_one();
        assert!(matches!(
            send_task.await.unwrap().unwrap(),
            SendOutcome::Sent(_)
        ));
    }

    #[tokio::test]
    async fn caller_cancellation_does_not_cancel_outbound_finalization() {
        let store = Store::open_in_memory().unwrap();
        let send_started = Arc::new(tokio::sync::Notify::new());
        let release_send = Arc::new(tokio::sync::Notify::new());
        let messaging = Messaging::new(
            store,
            EventBus::new(),
            DeliveryWakeup::new(),
            Arc::new(BlockingSender {
                send_started: send_started.clone(),
                release_send: release_send.clone(),
            }),
        );
        let send_task = tokio::spawn({
            let messaging = messaging.clone();
            async move {
                messaging
                    .send(SendMessage {
                        modem_path: "/org/freedesktop/ModemManager1/Modem/0".to_string(),
                        phone_number: "+15550000000".to_string(),
                        body: "survives cancellation".to_string(),
                        source: MessageSource::Web,
                    })
                    .await
            }
        });
        tokio::time::timeout(std::time::Duration::from_secs(1), send_started.notified())
            .await
            .unwrap();

        send_task.abort();
        let _ = send_task.await;
        release_send.notify_one();

        let finalized = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                let message = messaging
                    .list(MessageFilter::default())
                    .await
                    .unwrap()
                    .remove(0);
                if message.status == MessageStatus::Sent {
                    break message;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("detached finalizer should finish the message");
        assert_eq!(
            finalized.modem_sms_path.as_deref(),
            Some("/org/freedesktop/ModemManager1/SMS/blocked")
        );
    }

    #[tokio::test]
    async fn cancellation_after_local_commit_does_not_orphan_a_sending_row() {
        let store = Store::open_in_memory().unwrap();
        let (committed, release_creation) = store.pause_next_outbound_creation();
        let messaging = Messaging::new(
            store,
            EventBus::new(),
            DeliveryWakeup::new(),
            test_sms_sender(),
        );
        let caller = tokio::spawn({
            let messaging = messaging.clone();
            async move {
                messaging
                    .send(SendMessage {
                        modem_path: "/org/freedesktop/ModemManager1/Modem/0".to_string(),
                        phone_number: "+15550000000".to_string(),
                        body: "cancel after commit".to_string(),
                        source: MessageSource::Web,
                    })
                    .await
            }
        });
        tokio::time::timeout(std::time::Duration::from_secs(1), committed.notified())
            .await
            .unwrap();

        caller.abort();
        let _ = caller.await;
        release_creation.notify_one();

        let finalized = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                let message = messaging
                    .list(MessageFilter::default())
                    .await
                    .unwrap()
                    .remove(0);
                if message.status == MessageStatus::Sent {
                    break message;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("detached owner should continue after the committed create");
        assert_eq!(finalized.status, MessageStatus::Sent);
    }

    #[tokio::test]
    async fn transient_finalization_failure_is_retried_until_sent() {
        let store = Store::open_in_memory().unwrap();
        store.fail_next_outbound_finalizations(1);
        let messaging = Messaging::new(
            store,
            EventBus::new(),
            DeliveryWakeup::new(),
            test_sms_sender(),
        );

        let outcome = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            messaging.send(SendMessage {
                modem_path: "/org/freedesktop/ModemManager1/Modem/0".to_string(),
                phone_number: "+15550000000".to_string(),
                body: "retry finalization".to_string(),
                source: MessageSource::Web,
            }),
        )
        .await
        .unwrap()
        .unwrap();

        assert!(matches!(outcome, SendOutcome::Sent(_)));
    }

    #[tokio::test]
    async fn recovery_finalizes_a_sent_modem_object_without_sending_again() {
        let store = Store::open_in_memory().unwrap();
        let message = store
            .create_outbound(
                "+15550000000".to_string(),
                "recover me".to_string(),
                MessageSource::Web,
            )
            .await
            .unwrap();
        store
            .set_outbound_modem_sms_path(
                message.id,
                "/org/freedesktop/ModemManager1/SMS/recover".to_string(),
            )
            .await
            .unwrap();
        let send_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let messaging = Messaging::new(
            store,
            EventBus::new(),
            DeliveryWakeup::new(),
            Arc::new(RecoverySender {
                send_calls: send_calls.clone(),
                state: ModemSmsState::Sent,
            }),
        );

        assert_eq!(messaging.recover_outbound().await.unwrap(), 1);
        let recovered = messaging
            .list(MessageFilter::default())
            .await
            .unwrap()
            .remove(0);
        assert_eq!(recovered.status, MessageStatus::Sent);
        assert_eq!(send_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn startup_recovery_snapshot_excludes_new_outbound_rows() {
        let store = Store::open_in_memory().unwrap();
        let interrupted = store
            .create_outbound(
                "+15550000000".to_string(),
                "pre-existing send".to_string(),
                MessageSource::Web,
            )
            .await
            .unwrap();
        store
            .set_outbound_modem_sms_path(
                interrupted.id,
                "/org/freedesktop/ModemManager1/SMS/pre-existing".to_string(),
            )
            .await
            .unwrap();
        let messaging = Messaging::new(
            store.clone(),
            EventBus::new(),
            DeliveryWakeup::new(),
            Arc::new(RecoverySender {
                send_calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                state: ModemSmsState::Sent,
            }),
        );

        assert_eq!(messaging.start_outbound_recovery().await.unwrap(), 1);
        let new_message = store
            .create_outbound(
                "+15550000001".to_string(),
                "new active send".to_string(),
                MessageSource::Web,
            )
            .await
            .unwrap();

        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                let messages = messaging.list(MessageFilter::default()).await.unwrap();
                if messages.iter().any(|message| {
                    message.id == interrupted.id && message.status == MessageStatus::Sent
                }) {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let new_message = messaging
            .list(MessageFilter::default())
            .await
            .unwrap()
            .into_iter()
            .find(|message| message.id == new_message.id)
            .unwrap();
        assert_eq!(new_message.status, MessageStatus::Sending);
        assert!(new_message.modem_sms_path.is_none());
    }

    #[tokio::test]
    async fn recovery_resumes_the_same_stored_modem_object() {
        let store = Store::open_in_memory().unwrap();
        let message = store
            .create_outbound(
                "+15550000000".to_string(),
                "resume prepared object".to_string(),
                MessageSource::Web,
            )
            .await
            .unwrap();
        store
            .set_outbound_modem_sms_path(
                message.id,
                "/org/freedesktop/ModemManager1/SMS/stored".to_string(),
            )
            .await
            .unwrap();
        let send_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let messaging = Messaging::new(
            store,
            EventBus::new(),
            DeliveryWakeup::new(),
            Arc::new(RecoverySender {
                send_calls: send_calls.clone(),
                state: ModemSmsState::Stored,
            }),
        );

        assert_eq!(messaging.recover_outbound().await.unwrap(), 1);
        let recovered = messaging
            .list(MessageFilter::default())
            .await
            .unwrap()
            .remove(0);
        assert_eq!(recovered.status, MessageStatus::Sent);
        assert_eq!(
            recovered.modem_sms_path.as_deref(),
            Some("/org/freedesktop/ModemManager1/SMS/stored")
        );
        assert_eq!(send_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn send_error_is_reconciled_against_the_persisted_modem_object() {
        let messaging = Messaging::new(
            Store::open_in_memory().unwrap(),
            EventBus::new(),
            DeliveryWakeup::new(),
            Arc::new(AmbiguousSender),
        );

        let outcome = messaging
            .send(SendMessage {
                modem_path: "/org/freedesktop/ModemManager1/Modem/0".to_string(),
                phone_number: "+15550000000".to_string(),
                body: "reply may be lost".to_string(),
                source: MessageSource::Web,
            })
            .await
            .unwrap();

        assert!(matches!(outcome, SendOutcome::Sent(_)));
    }

    #[tokio::test]
    async fn recovery_waits_for_a_queued_modem_object_without_resending() {
        let store = Store::open_in_memory().unwrap();
        let message = store
            .create_outbound(
                "+15550000000".to_string(),
                "already queued".to_string(),
                MessageSource::Web,
            )
            .await
            .unwrap();
        store
            .set_outbound_modem_sms_path(
                message.id,
                "/org/freedesktop/ModemManager1/SMS/sending".to_string(),
            )
            .await
            .unwrap();
        let send_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let messaging = Messaging::new(
            store,
            EventBus::new(),
            DeliveryWakeup::new(),
            Arc::new(SendingRecoverySender {
                state_reads: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                send_calls: send_calls.clone(),
            }),
        );

        assert_eq!(messaging.recover_outbound().await.unwrap(), 1);
        let recovered = messaging
            .list(MessageFilter::default())
            .await
            .unwrap()
            .remove(0);
        assert_eq!(recovered.status, MessageStatus::Sent);
        assert_eq!(send_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn recovery_does_not_resend_a_missing_modem_object() {
        let store = Store::open_in_memory().unwrap();
        let message = store
            .create_outbound(
                "+15550000000".to_string(),
                "object was removed".to_string(),
                MessageSource::Web,
            )
            .await
            .unwrap();
        store
            .set_outbound_modem_sms_path(
                message.id,
                "/org/freedesktop/ModemManager1/SMS/missing".to_string(),
            )
            .await
            .unwrap();
        let send_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let messaging = Messaging::new(
            store,
            EventBus::new(),
            DeliveryWakeup::new(),
            Arc::new(RecoverySender {
                send_calls: send_calls.clone(),
                state: ModemSmsState::Unknown,
            }),
        );

        assert_eq!(messaging.recover_outbound().await.unwrap(), 1);
        let recovered = messaging
            .list(MessageFilter::default())
            .await
            .unwrap()
            .remove(0);
        assert_eq!(recovered.status, MessageStatus::Failed);
        assert!(recovered
            .error
            .unwrap()
            .contains("automatic retry suppressed"));
        assert_eq!(send_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn recovery_fails_an_unprepared_message_without_sending() {
        let store = Store::open_in_memory().unwrap();
        store
            .create_outbound(
                "+15550000000".to_string(),
                "never reached modem create".to_string(),
                MessageSource::Web,
            )
            .await
            .unwrap();
        let send_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let messaging = Messaging::new(
            store,
            EventBus::new(),
            DeliveryWakeup::new(),
            Arc::new(RecoverySender {
                send_calls: send_calls.clone(),
                state: ModemSmsState::Unknown,
            }),
        );

        assert_eq!(messaging.recover_outbound().await.unwrap(), 1);
        let recovered = messaging
            .list(MessageFilter::default())
            .await
            .unwrap()
            .remove(0);
        assert_eq!(recovered.status, MessageStatus::Failed);
        assert!(recovered
            .error
            .as_deref()
            .unwrap()
            .contains("before modem preparation"));
        assert_eq!(send_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn outbound_transport_failure_is_an_explicit_failed_outcome() {
        let store = Store::open_in_memory().unwrap();
        let events = EventBus::new();
        let mut received_events = events.subscribe();
        let messaging = Messaging::new(
            store,
            events,
            DeliveryWakeup::new(),
            Arc::new(FailingSmsSender),
        );

        let outcome = messaging
            .send(SendMessage {
                modem_path: "/org/freedesktop/ModemManager1/Modem/0".to_string(),
                phone_number: "+15550000000".to_string(),
                body: "will fail".to_string(),
                source: MessageSource::Cli,
            })
            .await
            .unwrap();

        let SendOutcome::Failed(message) = outcome else {
            panic!("transport failure must be explicit");
        };
        assert_eq!(message.status, MessageStatus::Failed);
        assert_eq!(
            messaging
                .list(MessageFilter::default())
                .await
                .unwrap()
                .len(),
            1
        );
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
