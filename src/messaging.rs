use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use crate::dbus::{ReceivedSms, SendAttemptOutcome, SmsSender};
use crate::delivery::DeliveryWakeup;
use crate::events::{AppEvent, EventBus};
use crate::message::{ConversationSummary, Message, MessageFilter, MessageSource, MessageStatus};
use crate::persistence::{
    CreateOutboundOutcome, InboundMessage, InboundOutcome, OutboundClaim, OutboundPhase,
    OutboundTransition, Store,
};

pub struct SendMessage {
    pub modem_path: String,
    pub phone_number: String,
    pub body: String,
    pub source: MessageSource,
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendOutcome {
    Sent(Message),
    Failed(Message),
    Pending(Message),
}

impl SendOutcome {
    pub fn into_message(self) -> Message {
        match self {
            Self::Sent(message) | Self::Failed(message) | Self::Pending(message) => message,
        }
    }
}

fn send_outcome(message: Message) -> SendOutcome {
    match message.status {
        MessageStatus::Sent => SendOutcome::Sent(message),
        MessageStatus::Failed => SendOutcome::Failed(message),
        MessageStatus::Sending => SendOutcome::Pending(message),
        MessageStatus::Received => unreachable!("outbound operation cannot be received"),
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

#[derive(Clone)]
pub struct Messaging {
    store: Store,
    events: EventBus,
    delivery_wakeup: DeliveryWakeup,
    sms_sender: Arc<dyn SmsSender>,
}

impl Messaging {
    #[cfg(not(test))]
    const OUTBOUND_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
    #[cfg(test)]
    const OUTBOUND_HEARTBEAT_INTERVAL: Duration = Duration::from_millis(10);

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
    /// was aborted by runtime shutdown; the supervised outbound worker resumes
    /// such rows after their lease expires.
    pub async fn send(&self, request: SendMessage) -> anyhow::Result<SendOutcome> {
        let owner = uuid::Uuid::new_v4().to_string();
        let messaging = self.clone();
        let finalizer =
            tokio::spawn(async move { messaging.create_and_run_outbound(request, owner).await });
        finalizer
            .await
            .map_err(|error| anyhow::anyhow!("outbound finalizer task failed: {error}"))?
    }

    async fn create_and_run_outbound(
        &self,
        request: SendMessage,
        owner: String,
    ) -> anyhow::Result<SendOutcome> {
        let SendMessage {
            ref phone_number,
            ref body,
            source,
            ..
        } = request;
        let created = self
            .store
            .create_claimed_outbound(
                phone_number.clone(),
                body.clone(),
                source,
                request.idempotency_key.clone(),
                owner.clone(),
            )
            .await?;
        match created {
            CreateOutboundOutcome::Created(message) => {
                self.events.send(AppEvent::MessageCreated(message.clone()));
                self.run_new_outbound(message, request, owner).await
            }
            CreateOutboundOutcome::Existing(message) => Ok(send_outcome(message)),
        }
    }

    async fn run_new_outbound(
        &self,
        message: Message,
        request: SendMessage,
        owner: String,
    ) -> anyhow::Result<SendOutcome> {
        let prepared = match self
            .with_outbound_lease(
                message.id,
                &owner,
                self.sms_sender
                    .prepare(&request.modem_path, &request.phone_number, &request.body),
            )
            .await
        {
            Ok(prepared) => prepared,
            Err(error) => {
                return self
                    .finish_outbound(
                        message.id,
                        owner,
                        MessageStatus::Failed,
                        Some(error.to_string()),
                    )
                    .await;
            }
        };
        match self
            .store
            .set_outbound_prepared(message.id, owner.clone(), prepared.modem_sms_path.clone())
            .await?
        {
            OutboundTransition::Applied(_) => {
                self.begin_and_send(message.id, owner, &prepared.modem_sms_path)
                    .await
            }
            OutboundTransition::OwnershipLost(message) => Ok(send_outcome(message)),
        }
    }

    async fn begin_and_send(
        &self,
        message_id: i64,
        owner: String,
        modem_sms_path: &str,
    ) -> anyhow::Result<SendOutcome> {
        match self
            .store
            .begin_outbound_send(message_id, owner.clone())
            .await?
        {
            OutboundTransition::Applied(_) => {
                let outcome = self
                    .with_outbound_lease(
                        message_id,
                        &owner,
                        self.sms_sender.send_prepared(modem_sms_path),
                    )
                    .await;
                self.handle_send_attempt(message_id, owner, outcome).await
            }
            OutboundTransition::OwnershipLost(message) => Ok(send_outcome(message)),
        }
    }

    async fn handle_send_attempt(
        &self,
        message_id: i64,
        owner: String,
        outcome: SendAttemptOutcome,
    ) -> anyhow::Result<SendOutcome> {
        match outcome {
            SendAttemptOutcome::Accepted => {
                self.finish_outbound(message_id, owner, MessageStatus::Sent, None)
                    .await
            }
            SendAttemptOutcome::Rejected(error) => {
                self.finish_outbound(
                    message_id,
                    owner,
                    MessageStatus::Failed,
                    Some(error.to_string()),
                )
                .await
            }
            SendAttemptOutcome::NotAttempted(error) => {
                self.defer_outbound(
                    message_id,
                    owner,
                    OutboundPhase::Prepared,
                    Some(error.to_string()),
                    Some(Duration::from_secs(5)),
                )
                .await
            }
            SendAttemptOutcome::Unknown(error) => {
                self.defer_outbound(
                    message_id,
                    owner,
                    OutboundPhase::Uncertain,
                    Some(format!(
                        "send outcome unknown; automatic resend suppressed: {error}"
                    )),
                    Some(Duration::from_secs(5)),
                )
                .await
            }
        }
    }

    async fn with_outbound_lease<T>(
        &self,
        message_id: i64,
        owner: &str,
        operation: impl Future<Output = T>,
    ) -> T {
        tokio::pin!(operation);
        let mut heartbeat = tokio::time::interval(Self::OUTBOUND_HEARTBEAT_INTERVAL);
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        heartbeat.tick().await;
        loop {
            tokio::select! {
                result = &mut operation => return result,
                _ = heartbeat.tick() => {
                    match self
                        .store
                        .renew_outbound_lease(message_id, owner.to_string())
                        .await
                    {
                        Ok(true) => {}
                        Ok(false) => {
                            log::warn!(
                                "outbound lease ownership was lost; message_id={message_id}"
                            );
                        }
                        Err(error) => {
                            log::error!(
                                "renewing outbound lease failed; message_id={message_id}: {error}"
                            );
                        }
                    }
                }
            }
        }
    }

    async fn finish_outbound(
        &self,
        message_id: i64,
        owner: String,
        status: MessageStatus,
        error: Option<String>,
    ) -> anyhow::Result<SendOutcome> {
        let mut delay = Duration::from_millis(100);
        for attempt in 0..5 {
            match self
                .store
                .finish_claimed_outbound(message_id, owner.clone(), status, error.clone())
                .await
            {
                Ok(OutboundTransition::Applied(updated)) => {
                    self.events.send(AppEvent::MessageUpdated(updated.clone()));
                    return Ok(send_outcome(updated));
                }
                Ok(OutboundTransition::OwnershipLost(message)) => {
                    return Ok(send_outcome(message));
                }
                Err(update_error) => {
                    log::error!(
                        "finalizing outbound message failed; message_id={message_id}: {update_error}"
                    );
                    if attempt == 4 {
                        return Err(update_error);
                    }
                    tokio::time::sleep(delay).await;
                    delay *= 2;
                }
            }
        }
        unreachable!()
    }

    async fn defer_outbound(
        &self,
        message_id: i64,
        owner: String,
        phase: OutboundPhase,
        error: Option<String>,
        retry_after: Option<Duration>,
    ) -> anyhow::Result<SendOutcome> {
        let transition = self
            .store
            .defer_outbound(message_id, owner, phase, error, retry_after)
            .await?;
        let message = match transition {
            OutboundTransition::Applied(message) | OutboundTransition::OwnershipLost(message) => {
                message
            }
        };
        self.events.send(AppEvent::MessageUpdated(message.clone()));
        Ok(send_outcome(message))
    }

    pub async fn run_outbound_worker(&self) {
        let owner = uuid::Uuid::new_v4().to_string();
        loop {
            match self.store.claim_due_outbound(owner.clone()).await {
                Ok(Some(claim)) => {
                    if let Err(error) = self.recover_claim(claim, owner.clone()).await {
                        log::error!("outbound recovery attempt failed: {error}");
                    }
                }
                Ok(None) => tokio::time::sleep(Duration::from_secs(1)).await,
                Err(error) => {
                    log::error!("claiming outbound recovery work failed: {error}");
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
    }

    #[cfg(test)]
    async fn recover_one_outbound(&self) -> anyhow::Result<bool> {
        let owner = uuid::Uuid::new_v4().to_string();
        let Some(claim) = self.store.claim_due_outbound(owner.clone()).await? else {
            return Ok(false);
        };
        self.recover_claim(claim, owner).await?;
        Ok(true)
    }

    async fn recover_claim(
        &self,
        claim: OutboundClaim,
        owner: String,
    ) -> anyhow::Result<SendOutcome> {
        match claim.phase {
            OutboundPhase::Created => {
                self.finish_outbound(
                    claim.message.id,
                    owner,
                    MessageStatus::Failed,
                    Some("outbound interrupted before modem preparation".to_string()),
                )
                .await
            }
            OutboundPhase::Prepared => {
                let Some(path) = claim.message.modem_sms_path.as_deref() else {
                    return self
                        .finish_outbound(
                            claim.message.id,
                            owner,
                            MessageStatus::Failed,
                            Some("prepared outbound has no modem SMS path".to_string()),
                        )
                        .await;
                };
                self.begin_and_send(claim.message.id, owner, path).await
            }
            OutboundPhase::SendStarted | OutboundPhase::Uncertain => {
                self.reconcile_claim(claim.message, owner).await
            }
            OutboundPhase::Unknown => Ok(SendOutcome::Pending(claim.message)),
        }
    }

    async fn reconcile_claim(
        &self,
        message: Message,
        owner: String,
    ) -> anyhow::Result<SendOutcome> {
        let Some(path) = message.modem_sms_path.as_deref() else {
            return self
                .defer_outbound(
                    message.id,
                    owner,
                    OutboundPhase::Unknown,
                    Some("send outcome unknown and modem SMS path is missing".to_string()),
                    None,
                )
                .await;
        };
        match self
            .with_outbound_lease(message.id, &owner, self.sms_sender.sms_state(path))
            .await
        {
            Ok(crate::dbus::ModemSmsState::Sent) => {
                self.finish_outbound(message.id, owner, MessageStatus::Sent, None)
                    .await
            }
            Ok(crate::dbus::ModemSmsState::Stored) | Ok(crate::dbus::ModemSmsState::Sending) => {
                self.defer_outbound(
                    message.id,
                    owner,
                    OutboundPhase::Uncertain,
                    Some(
                        "send outcome remains unresolved; automatic resend suppressed".to_string(),
                    ),
                    Some(Duration::from_secs(5)),
                )
                .await
            }
            Ok(crate::dbus::ModemSmsState::Unknown) => {
                self.defer_outbound(
                    message.id,
                    owner,
                    OutboundPhase::Unknown,
                    Some("send outcome unknown; automatic resend suppressed".to_string()),
                    None,
                )
                .await
            }
            Err(error) => {
                self.defer_outbound(
                    message.id,
                    owner,
                    OutboundPhase::Uncertain,
                    Some(format!("reading modem SMS state failed: {error}")),
                    Some(Duration::from_secs(5)),
                )
                .await
            }
        }
    }

    pub async fn has_pending_outbound(&self) -> anyhow::Result<bool> {
        self.store.has_pending_outbound().await
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
    use crate::dbus::{ModemSmsState, PreparedSms, SendAttemptOutcome, SmsSender};
    use crate::delivery::DeliveryWakeup;
    use crate::events::{AppEvent, EventBus};
    use crate::message::{Message, MessageFilter, MessageSource, MessageStatus};
    use crate::persistence::{CreateOutboundOutcome, OutboundPhase, OutboundTransition, Store};

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

    async fn seed_claimable_outbound(
        store: &Store,
        phase: OutboundPhase,
        modem_sms_path: Option<&str>,
    ) -> Message {
        let owner = uuid::Uuid::new_v4().to_string();
        let CreateOutboundOutcome::Created(message) = store
            .create_claimed_outbound(
                "+15550000000".to_string(),
                "seeded outbound".to_string(),
                MessageSource::Web,
                None,
                owner.clone(),
            )
            .await
            .unwrap()
        else {
            unreachable!()
        };
        if let Some(path) = modem_sms_path {
            assert!(matches!(
                store
                    .set_outbound_prepared(message.id, owner.clone(), path.to_string())
                    .await
                    .unwrap(),
                OutboundTransition::Applied(_)
            ));
        }
        if matches!(phase, OutboundPhase::SendStarted | OutboundPhase::Uncertain) {
            assert!(matches!(
                store
                    .begin_outbound_send(message.id, owner.clone())
                    .await
                    .unwrap(),
                OutboundTransition::Applied(_)
            ));
        }
        let OutboundTransition::Applied(message) = store
            .defer_outbound(message.id, owner, phase, None, None)
            .await
            .unwrap()
        else {
            unreachable!()
        };
        message
    }

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
        ) -> Pin<Box<dyn Future<Output = SendAttemptOutcome> + Send + 'a>> {
            Box::pin(async { SendAttemptOutcome::Unknown(anyhow::anyhow!("send reply timed out")) })
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
        ) -> Pin<Box<dyn Future<Output = SendAttemptOutcome> + Send + 'a>> {
            let send_calls = self.send_calls.clone();
            Box::pin(async move {
                send_calls.fetch_add(1, Ordering::SeqCst);
                SendAttemptOutcome::Accepted
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
            Box::pin(async {
                Ok(PreparedSms {
                    modem_sms_path: "/org/freedesktop/ModemManager1/SMS/idempotent".to_string(),
                })
            })
        }

        fn send_prepared<'a>(
            &'a self,
            _modem_sms_path: &'a str,
        ) -> Pin<Box<dyn Future<Output = SendAttemptOutcome> + Send + 'a>> {
            let send_calls = self.send_calls.clone();
            Box::pin(async move {
                send_calls.fetch_add(1, Ordering::SeqCst);
                SendAttemptOutcome::Accepted
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
        ) -> Pin<Box<dyn Future<Output = SendAttemptOutcome> + Send + 'a>> {
            let send_started = self.send_started.clone();
            let release_send = self.release_send.clone();
            Box::pin(async move {
                send_started.notify_one();
                release_send.notified().await;
                SendAttemptOutcome::Accepted
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
        ) -> Pin<Box<dyn Future<Output = SendAttemptOutcome> + Send + 'a>> {
            let store = self.store.clone();
            let observed = self.observed_persisted_path.clone();
            let modem_sms_path = modem_sms_path.to_string();
            Box::pin(async move {
                let messages = store
                    .list_messages(MessageFilter::default())
                    .await
                    .expect("path observation should read the store");
                observed.store(
                    messages[0].modem_sms_path.as_deref() == Some(modem_sms_path.as_str()),
                    Ordering::SeqCst,
                );
                SendAttemptOutcome::Accepted
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
        ) -> Pin<Box<dyn Future<Output = SendAttemptOutcome> + Send + 'a>> {
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
                idempotency_key: None,
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
    async fn repeated_idempotency_key_returns_the_original_send() {
        let send_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let messaging = Messaging::new(
            Store::open_in_memory().unwrap(),
            EventBus::new(),
            DeliveryWakeup::new(),
            Arc::new(RecoverySender {
                send_calls: send_calls.clone(),
                state: ModemSmsState::Sent,
            }),
        );
        let send = |body: &str| SendMessage {
            modem_path: "/org/freedesktop/ModemManager1/Modem/0".to_string(),
            phone_number: "+15550000000".to_string(),
            body: body.to_string(),
            source: MessageSource::Web,
            idempotency_key: Some("request-42".to_string()),
        };

        let (first, repeated) = tokio::join!(
            messaging.send(send("send once")),
            messaging.send(send("send once"))
        );
        let first = first.unwrap().into_message();
        let repeated = repeated.unwrap().into_message();

        assert_eq!(first.id, repeated.id);
        assert_eq!(send_calls.load(Ordering::SeqCst), 1);
        let conflict = messaging.send(send("different request")).await.unwrap_err();
        assert!(conflict
            .downcast_ref::<crate::message::IdempotencyConflict>()
            .is_some());

        messaging.delete(vec![first.id]).await.unwrap();
        let replay = messaging.send(send("send once")).await.unwrap_err();
        assert!(replay
            .downcast_ref::<crate::message::IdempotencyReplayUnavailable>()
            .is_some());
        assert_eq!(send_calls.load(Ordering::SeqCst), 1);
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
                idempotency_key: None,
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
            store.clone(),
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
                        idempotency_key: None,
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
    async fn heartbeat_and_detached_finalizer_protect_an_active_send() {
        let store = Store::open_in_memory().unwrap();
        let send_started = Arc::new(tokio::sync::Notify::new());
        let release_send = Arc::new(tokio::sync::Notify::new());
        let messaging = Messaging::new(
            store.clone(),
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
                        idempotency_key: None,
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
        store.expire_outbound_lease(message.id).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        assert!(store
            .claim_due_outbound("competing-owner".to_string())
            .await
            .unwrap()
            .is_none());

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
                        idempotency_key: None,
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
                idempotency_key: None,
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
        seed_claimable_outbound(
            &store,
            OutboundPhase::Uncertain,
            Some("/org/freedesktop/ModemManager1/SMS/recover"),
        )
        .await;
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

        assert!(messaging.recover_one_outbound().await.unwrap());
        let recovered = messaging
            .list(MessageFilter::default())
            .await
            .unwrap()
            .remove(0);
        assert_eq!(recovered.status, MessageStatus::Sent);
        assert_eq!(send_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn ownership_and_terminal_cas_exclude_competing_processes() {
        let store = Store::open_in_memory().unwrap();
        let owner = "active-owner".to_string();
        let CreateOutboundOutcome::Created(message) = store
            .create_claimed_outbound(
                "+15550000000".to_string(),
                "owned send".to_string(),
                MessageSource::Web,
                None,
                owner.clone(),
            )
            .await
            .unwrap()
        else {
            unreachable!()
        };

        assert!(store
            .claim_due_outbound("competing-owner".to_string())
            .await
            .unwrap()
            .is_none());
        store.expire_outbound_lease(message.id).await.unwrap();
        let claimed = store
            .claim_due_outbound("competing-owner".to_string())
            .await
            .unwrap()
            .expect("expired lease should be claimable");
        assert_eq!(claimed.message.id, message.id);
        assert!(matches!(
            store
                .finish_claimed_outbound(
                    message.id,
                    owner,
                    MessageStatus::Failed,
                    Some("late failure".to_string())
                )
                .await
                .unwrap(),
            OutboundTransition::OwnershipLost(ref current)
                if current.status == MessageStatus::Sending
        ));
        assert!(matches!(
            store
                .finish_claimed_outbound(
                    message.id,
                    "competing-owner".to_string(),
                    MessageStatus::Sent,
                    None
                )
                .await
                .unwrap(),
            OutboundTransition::Applied(ref current)
                if current.status == MessageStatus::Sent
        ));
        assert!(matches!(
            store
                .finish_claimed_outbound(
                    message.id,
                    "competing-owner".to_string(),
                    MessageStatus::Failed,
                    Some("late failure".to_string())
                )
                .await
                .unwrap(),
            OutboundTransition::OwnershipLost(ref current)
                if current.status == MessageStatus::Sent
        ));
    }

    #[tokio::test]
    async fn recovery_resumes_the_same_stored_modem_object() {
        let store = Store::open_in_memory().unwrap();
        seed_claimable_outbound(
            &store,
            OutboundPhase::Prepared,
            Some("/org/freedesktop/ModemManager1/SMS/stored"),
        )
        .await;
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

        assert!(messaging.recover_one_outbound().await.unwrap());
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
    async fn unknown_send_result_stays_pending_without_state_based_resend() {
        let store = Store::open_in_memory().unwrap();
        let messaging = Messaging::new(
            store,
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
                idempotency_key: None,
            })
            .await
            .unwrap();

        let SendOutcome::Pending(message) = outcome else {
            panic!("an ambiguous D-Bus result must remain pending");
        };
        assert_eq!(message.status, MessageStatus::Sending);
        assert!(message
            .error
            .as_deref()
            .unwrap()
            .contains("automatic resend suppressed"));
        assert!(messaging.has_pending_outbound().await.unwrap());
    }

    #[tokio::test]
    async fn recovery_waits_for_a_queued_modem_object_without_resending() {
        let store = Store::open_in_memory().unwrap();
        seed_claimable_outbound(
            &store,
            OutboundPhase::Uncertain,
            Some("/org/freedesktop/ModemManager1/SMS/sending"),
        )
        .await;
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

        assert!(messaging.recover_one_outbound().await.unwrap());
        let recovered = messaging
            .list(MessageFilter::default())
            .await
            .unwrap()
            .remove(0);
        assert_eq!(recovered.status, MessageStatus::Sending);
        assert_eq!(send_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn recovery_does_not_resend_a_missing_modem_object() {
        let store = Store::open_in_memory().unwrap();
        seed_claimable_outbound(
            &store,
            OutboundPhase::Uncertain,
            Some("/org/freedesktop/ModemManager1/SMS/missing"),
        )
        .await;
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

        assert!(messaging.recover_one_outbound().await.unwrap());
        let recovered = messaging
            .list(MessageFilter::default())
            .await
            .unwrap()
            .remove(0);
        assert_eq!(recovered.status, MessageStatus::Sending);
        assert!(recovered
            .error
            .unwrap()
            .contains("automatic resend suppressed"));
        assert_eq!(send_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn recovery_fails_an_unprepared_message_without_sending() {
        let store = Store::open_in_memory().unwrap();
        seed_claimable_outbound(&store, OutboundPhase::Created, None).await;
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

        assert!(messaging.recover_one_outbound().await.unwrap());
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
                idempotency_key: None,
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
