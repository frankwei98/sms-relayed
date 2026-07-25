use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use futures_util::future::BoxFuture;
use log::{error, info, warn};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::config::AppConfig;
use crate::dbus::{
    InboundEvent, InboundSms, InboundSmsProperties, InboundSubscription, SystemInboundSource,
};
use crate::messaging::{Messaging, ReceiveMessage};
use crate::modem::ModemService;
use crate::persistence::Store;

const MAX_INBOUND_TASKS: usize = 16;
const INITIAL_RECONNECT_DELAY: Duration = Duration::from_secs(5);
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(60);
const BODY_POLL_INTERVAL: Duration = Duration::from_millis(100);
const MAX_BODY_POLLS: usize = 600;
const INITIAL_PERSISTENCE_RETRY_DELAY: Duration = Duration::from_millis(100);
const MAX_PERSISTENCE_RETRY_DELAY: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct ReceivedSms {
    pub phone_number: String,
    pub body: String,
    pub timestamp: String,
    pub modem_sms_path: String,
}

#[derive(Debug, Clone)]
pub(crate) struct InboundSettings {
    configured_modem_path: String,
    ignored_storage: Vec<StorageType>,
    profile_keys: Vec<String>,
}

impl InboundSettings {
    pub(crate) fn from_app_config(config: &AppConfig) -> Self {
        Self {
            configured_modem_path: config.app.modem_path.clone(),
            ignored_storage: config
                .sms
                .ignore_storage
                .iter()
                .map(|storage| StorageType::from_config(storage))
                .collect(),
            profile_keys: config
                .enabled_profiles()
                .unwrap_or_default()
                .iter()
                .map(|profile| profile.key())
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StorageType {
    Unknown = 0,
    Sm = 1,
    Me = 2,
    Mt = 3,
    Sr = 4,
    Bm = 5,
    Ta = 6,
    All = 100,
    NoMatch = 999,
}

impl StorageType {
    fn from_config(value: &str) -> Self {
        match value {
            "unknown" => Self::Unknown,
            "sm" => Self::Sm,
            "me" => Self::Me,
            "mt" => Self::Mt,
            "sr" => Self::Sr,
            "bm" => Self::Bm,
            "ta" => Self::Ta,
            "all" => Self::All,
            _ => {
                warn!(
                    "unknown storage type: {}; storage will not be filtered by this entry",
                    value
                );
                Self::NoMatch
            }
        }
    }

    fn should_ignore(self, storage: u32) -> bool {
        match self {
            Self::All | Self::NoMatch => false,
            _ => self as u32 == storage,
        }
    }
}

pub(crate) struct InboundWorker {
    store: Store,
    messaging: Messaging,
    modem_service: ModemService,
    settings: InboundSettings,
    source: Arc<dyn InboundSourceAdapter>,
    inbound_limit: Arc<Semaphore>,
}

impl InboundWorker {
    pub(crate) fn new(
        store: Store,
        messaging: Messaging,
        modem_service: ModemService,
        settings: InboundSettings,
    ) -> Self {
        Self {
            store,
            messaging,
            modem_service,
            settings,
            source: Arc::new(SystemSourceAdapter),
            inbound_limit: Arc::new(Semaphore::new(MAX_INBOUND_TASKS)),
        }
    }

    #[cfg(test)]
    fn with_source(mut self, source: Arc<dyn InboundSourceAdapter>) -> Self {
        self.source = source;
        self
    }

    pub(crate) async fn run(&self) -> Result<()> {
        println!("短信转发模式正在启动，正在连接系统 D-Bus。");
        info!("正在运行. 按下 Ctrl-C 停止.");

        let mut children = JoinSet::new();
        let mut current_path = None;
        let mut delay = INITIAL_RECONNECT_DELAY;

        loop {
            if current_path.is_none() {
                current_path = match resolve_monitor_path(
                    &self.settings.configured_modem_path,
                    &self.modem_service,
                    &self.store,
                )
                .await
                {
                    Ok(path) => {
                        self.modem_service.set_verified_path(path.clone());
                        path
                    }
                    Err(error) => {
                        error!("modem resolution failed: {}", error);
                        self.modem_service.set_verified_path(None);
                        None
                    }
                };
            }
            let Some(path) = current_path.clone() else {
                warn!("no verified modem identity available; retrying resolution");
                tokio::time::sleep(delay).await;
                delay = next_reconnect_delay(delay);
                continue;
            };

            match self.run_subscription(&path, &mut children).await {
                Ok(()) => {
                    delay = INITIAL_RECONNECT_DELAY;
                }
                Err(error) => {
                    error!("D-Bus monitor lost: {}", error);
                    crate::monitoring::capture_failure("dbus", "dbus.monitor_lost");
                    let resolved = match resolve_monitor_path(
                        &self.settings.configured_modem_path,
                        &self.modem_service,
                        &self.store,
                    )
                    .await
                    {
                        Ok(resolved) => resolved,
                        Err(error) => {
                            error!("modem resolution failed: {}", error);
                            self.modem_service.set_verified_path(None);
                            None
                        }
                    };
                    if let Some(new_path) = resolved {
                        if new_path != path {
                            info!("modem path changed from {} to {}", path, new_path);
                        }
                        self.modem_service.set_verified_path(Some(new_path.clone()));
                        current_path = Some(new_path);
                    } else {
                        warn!("modem re-resolution failed; will retry");
                        self.modem_service.set_verified_path(None);
                        current_path = None;
                    }
                }
            }

            info!("reconnecting in {}s...", delay.as_secs_f64());
            tokio::time::sleep(delay).await;
            delay = next_reconnect_delay(delay);
        }
    }

    async fn run_subscription(&self, actual_path: &str, children: &mut JoinSet<()>) -> Result<()> {
        let mut subscription = self.source.subscribe(actual_path).await?;

        info!("SMS monitor ready on {}", actual_path);

        loop {
            let sms = tokio::select! {
                joined = children.join_next(), if !children.is_empty() => {
                    report_child_result(joined);
                    continue;
                }
                sms = subscription.next_sms() => sms?,
            };
            info!("SmsPath:\n{}", sms.path());

            let permit = self
                .inbound_limit
                .clone()
                .acquire_owned()
                .await
                .map_err(|_| anyhow::anyhow!("inbound task limiter closed"))?;
            let storage_filters = self.settings.ignored_storage.clone();
            let profile_keys = self.settings.profile_keys.clone();
            let messaging = self.messaging.clone();
            children.spawn(async move {
                let _permit = permit;
                if process_incoming_sms(sms, &storage_filters, messaging, profile_keys)
                    .await
                    .is_err()
                {
                    report_child_failure();
                }
            });
        }
    }
}

fn report_child_result(joined: Option<Result<(), tokio::task::JoinError>>) {
    if matches!(joined, Some(Err(_))) {
        report_child_failure();
    }
}

fn report_child_failure() {
    error!("incoming SMS processing task failed");
    crate::monitoring::capture_failure("dbus", "dbus.inbound_processing_failed");
}

fn next_reconnect_delay(delay: Duration) -> Duration {
    (delay * 2).min(MAX_RECONNECT_DELAY)
}

fn next_persistence_retry_delay(delay: Duration) -> Duration {
    (delay * 2).min(MAX_PERSISTENCE_RETRY_DELAY)
}

async fn process_incoming_sms(
    sms: Box<dyn InboundSmsAdapter>,
    storage_filters: &[StorageType],
    messaging: Messaging,
    profile_keys: Vec<String>,
) -> Result<()> {
    let mut retries = 0;
    loop {
        let properties = sms.properties().await?;

        if should_ignore_storage(properties.storage, storage_filters) {
            warn!("已过滤不转发");
            return Ok(());
        }

        if !properties.body.is_empty() {
            let received = ReceivedSms {
                phone_number: properties.phone_number,
                body: properties.body,
                timestamp: properties.timestamp,
                modem_sms_path: sms.path().to_string(),
            };
            let mut delay = INITIAL_PERSISTENCE_RETRY_DELAY;
            loop {
                match messaging
                    .receive(ReceiveMessage {
                        sms: received.clone(),
                        profile_keys: profile_keys.clone(),
                    })
                    .await
                {
                    Ok(_) => break,
                    Err(error) => {
                        error!("persist incoming SMS failed; retrying: {}", error);
                        crate::monitoring::capture_failure("dbus", "dbus.inbound_persist_failed");
                        tokio::time::sleep(delay).await;
                        delay = next_persistence_retry_delay(delay);
                    }
                }
            }
            return Ok(());
        }

        retries += 1;
        if retries % 50 == 0 {
            warn!("短信内容为空，已重试{}次", retries);
        }
        if retries > MAX_BODY_POLLS {
            warn!("短信内容为空，重试次数过多，放弃");
            crate::monitoring::capture_failure("dbus", "dbus.sms_body_unavailable");
            return Ok(());
        }
        tokio::time::sleep(BODY_POLL_INTERVAL).await;
    }
}

fn should_ignore_storage(storage: u32, filters: &[StorageType]) -> bool {
    filters
        .iter()
        .any(|filter| !matches!(filter, StorageType::All) && filter.should_ignore(storage))
}

/// Resolve the actual modem path for monitoring.
/// First tries the configured path directly. If it fails and a fingerprint is
/// stored, scans all modems and matches by fingerprint exactly once.
pub(crate) async fn resolve_monitor_path(
    configured_path: &str,
    modem_service: &ModemService,
    store: &Store,
) -> Result<Option<String>> {
    let stored_fingerprint = store.modem_fingerprint().await?;
    let identity = modem_service.extract_identity(configured_path).await;
    if let Some(identity) = identity {
        let current_fingerprint = ModemService::compute_fingerprint(&identity);
        match stored_fingerprint.as_deref() {
            Some(enrolled_fingerprint) if enrolled_fingerprint == current_fingerprint => {
                store.backfill_dedupe_keys().await?;
                return Ok(Some(configured_path.to_string()));
            }
            Some(enrolled_fingerprint) => {
                warn!("configured modem identity changed; refusing path reuse");
                return Ok(modem_service
                    .scan_and_match_fingerprint(enrolled_fingerprint)
                    .await);
            }
            None => {
                store.set_modem_fingerprint(current_fingerprint).await?;
                // Backfill dedupe keys for legacy modem-inbound messages now
                // that the fingerprint is available for stable hashing.
                store.backfill_dedupe_keys().await?;
                return Ok(Some(configured_path.to_string()));
            }
        }
    }

    let Some(stored_fingerprint) = stored_fingerprint.as_deref() else {
        return Ok(None);
    };
    Ok(modem_service
        .scan_and_match_fingerprint(stored_fingerprint)
        .await)
}

trait InboundSourceAdapter: Send + Sync {
    fn subscribe<'a>(
        &'a self,
        modem_path: &'a str,
    ) -> BoxFuture<'a, Result<Box<dyn InboundSubscriptionAdapter>>>;
}

trait InboundSubscriptionAdapter: Send {
    fn next_sms<'a>(&'a mut self) -> BoxFuture<'a, Result<Box<dyn InboundSmsAdapter>>>;
}

trait InboundSmsAdapter: Send + Sync {
    fn path(&self) -> &str;

    fn properties(&self) -> BoxFuture<'_, Result<InboundSmsProperties>>;
}

struct SystemSourceAdapter;

impl InboundSourceAdapter for SystemSourceAdapter {
    fn subscribe<'a>(
        &'a self,
        modem_path: &'a str,
    ) -> BoxFuture<'a, Result<Box<dyn InboundSubscriptionAdapter>>> {
        Box::pin(async move {
            let subscription = SystemInboundSource::new().subscribe(modem_path).await?;
            Ok(Box::new(SystemSubscriptionAdapter(subscription))
                as Box<dyn InboundSubscriptionAdapter>)
        })
    }
}

struct SystemSubscriptionAdapter(InboundSubscription);

impl InboundSubscriptionAdapter for SystemSubscriptionAdapter {
    fn next_sms<'a>(&'a mut self) -> BoxFuture<'a, Result<Box<dyn InboundSmsAdapter>>> {
        Box::pin(async move {
            let InboundEvent::Added(sms) = self.0.next().await?;
            Ok(Box::new(SystemSmsAdapter(sms)) as Box<dyn InboundSmsAdapter>)
        })
    }
}

struct SystemSmsAdapter(InboundSms);

impl InboundSmsAdapter for SystemSmsAdapter {
    fn path(&self) -> &str {
        self.0.path()
    }

    fn properties(&self) -> BoxFuture<'_, Result<InboundSmsProperties>> {
        Box::pin(self.0.properties())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Mutex;

    use tokio::sync::Notify;

    use super::*;
    use crate::dbus::SystemSmsSender;
    use crate::delivery::DeliveryWakeup;
    use crate::events::EventBus;
    use crate::message::{MessageDirection, MessageSource, MessageStatus};
    use crate::modem::{MmcliOutput, MmcliRunner, ModemError};

    const MODEM_PATH: &str = "/org/freedesktop/ModemManager1/Modem/0";
    const OTHER_MODEM_PATH: &str = "/org/freedesktop/ModemManager1/Modem/1";
    const SMS_PATH: &str = "/org/freedesktop/ModemManager1/SMS/1";

    enum PropertyAction {
        Return(Result<InboundSmsProperties>),
        Wait {
            started: Arc<AtomicUsize>,
            dropped: Arc<AtomicUsize>,
        },
    }

    struct ScriptedSms {
        path: String,
        actions: Mutex<VecDeque<PropertyAction>>,
        calls: Arc<AtomicUsize>,
    }

    impl ScriptedSms {
        fn new(path: &str, actions: Vec<PropertyAction>) -> Self {
            Self {
                path: path.to_string(),
                actions: Mutex::new(actions.into()),
                calls: Arc::new(AtomicUsize::new(0)),
            }
        }
    }

    impl InboundSmsAdapter for ScriptedSms {
        fn path(&self) -> &str {
            &self.path
        }

        fn properties(&self) -> BoxFuture<'_, Result<InboundSmsProperties>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let action = self
                .actions
                .lock()
                .unwrap()
                .pop_front()
                .expect("scripted property action");
            Box::pin(async move {
                match action {
                    PropertyAction::Return(result) => result,
                    PropertyAction::Wait { started, dropped } => {
                        struct DropSignal(Arc<AtomicUsize>);
                        impl Drop for DropSignal {
                            fn drop(&mut self) {
                                self.0.fetch_add(1, Ordering::SeqCst);
                            }
                        }

                        let _drop_signal = DropSignal(dropped);
                        started.fetch_add(1, Ordering::SeqCst);
                        std::future::pending().await
                    }
                }
            })
        }
    }

    struct ScriptedSubscription {
        messages: VecDeque<Box<dyn InboundSmsAdapter>>,
        reads: Arc<AtomicUsize>,
        dropped: Arc<AtomicBool>,
    }

    impl Drop for ScriptedSubscription {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::SeqCst);
        }
    }

    impl InboundSubscriptionAdapter for ScriptedSubscription {
        fn next_sms<'a>(&'a mut self) -> BoxFuture<'a, Result<Box<dyn InboundSmsAdapter>>> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            let message = self.messages.pop_front();
            Box::pin(async move {
                match message {
                    Some(message) => Ok(message),
                    None => std::future::pending().await,
                }
            })
        }
    }

    struct ScriptedSource {
        subscription: Mutex<Option<Box<dyn InboundSubscriptionAdapter>>>,
        subscribed: Arc<Notify>,
    }

    impl ScriptedSource {
        fn new(subscription: ScriptedSubscription) -> Self {
            Self {
                subscription: Mutex::new(Some(Box::new(subscription))),
                subscribed: Arc::new(Notify::new()),
            }
        }
    }

    impl InboundSourceAdapter for ScriptedSource {
        fn subscribe<'a>(
            &'a self,
            _modem_path: &'a str,
        ) -> BoxFuture<'a, Result<Box<dyn InboundSubscriptionAdapter>>> {
            let subscription = self.subscription.lock().unwrap().take();
            self.subscribed.notify_one();
            Box::pin(async move {
                subscription.ok_or_else(|| anyhow::anyhow!("scripted subscription exhausted"))
            })
        }
    }

    #[derive(Clone)]
    struct IdentityRunner;

    impl MmcliRunner for IdentityRunner {
        fn run<'a>(
            &'a self,
            args: &'a [&'a str],
            _timeout: Duration,
        ) -> Pin<Box<dyn Future<Output = Result<MmcliOutput, ModemError>> + Send + 'a>> {
            Box::pin(async move {
                let stdout = match args {
                    ["--modem", MODEM_PATH, "--output-json"] => {
                        r#"{"modem":{"generic":{"equipment-identifier":"other"}}}"#
                    }
                    ["-L"] => "/org/freedesktop/ModemManager1/Modem/1 [test] modem\n",
                    ["--modem", OTHER_MODEM_PATH, "--output-json"] => {
                        r#"{"modem":{"generic":{"equipment-identifier":"target"}}}"#
                    }
                    _ => "",
                };
                Ok(MmcliOutput {
                    stdout: stdout.to_string(),
                    stderr: String::new(),
                    status_success: !stdout.is_empty(),
                })
            })
        }
    }

    fn properties(body: &str, storage: u32) -> InboundSmsProperties {
        InboundSmsProperties {
            phone_number: "+15550000000".to_string(),
            body: body.to_string(),
            timestamp: "2026-07-25T00:00:00Z".to_string(),
            storage,
        }
    }

    fn settings(ignored_storage: Vec<StorageType>, profile_keys: Vec<String>) -> InboundSettings {
        InboundSettings {
            configured_modem_path: MODEM_PATH.to_string(),
            ignored_storage,
            profile_keys,
        }
    }

    fn messaging(store: Store) -> Messaging {
        Messaging::new(
            store,
            EventBus::new(),
            DeliveryWakeup::new(),
            Arc::new(SystemSmsSender::new()),
        )
    }

    fn worker(
        store: Store,
        source: Arc<dyn InboundSourceAdapter>,
        settings: InboundSettings,
    ) -> InboundWorker {
        InboundWorker::new(
            store.clone(),
            messaging(store),
            ModemService::new_with_runner(IdentityRunner),
            settings,
        )
        .with_source(source)
    }

    async fn wait_for_count(counter: &AtomicUsize, expected: usize) {
        tokio::time::timeout(Duration::from_secs(1), async {
            while counter.load(Ordering::SeqCst) < expected {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("counter reached expected value");
    }

    #[tokio::test]
    async fn seventeenth_sms_backpressures_subscription_before_spawn() {
        let started = Arc::new(AtomicUsize::new(0));
        let child_drops = Arc::new(AtomicUsize::new(0));
        let reads = Arc::new(AtomicUsize::new(0));
        let subscription_dropped = Arc::new(AtomicBool::new(false));
        let messages = (0..(MAX_INBOUND_TASKS + 2))
            .map(|index| {
                Box::new(ScriptedSms::new(
                    &format!("/org/freedesktop/ModemManager1/SMS/{index}"),
                    vec![PropertyAction::Wait {
                        started: started.clone(),
                        dropped: child_drops.clone(),
                    }],
                )) as Box<dyn InboundSmsAdapter>
            })
            .collect();
        let source = Arc::new(ScriptedSource::new(ScriptedSubscription {
            messages,
            reads: reads.clone(),
            dropped: subscription_dropped.clone(),
        }));
        let store = Store::open_in_memory().unwrap();
        let worker = worker(store, source, settings(Vec::new(), Vec::new()));

        let run = tokio::spawn(async move {
            let mut children = JoinSet::new();
            worker.run_subscription(MODEM_PATH, &mut children).await
        });

        wait_for_count(&started, MAX_INBOUND_TASKS).await;
        wait_for_count(&reads, MAX_INBOUND_TASKS + 1).await;
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(
            reads.load(Ordering::SeqCst),
            MAX_INBOUND_TASKS + 1,
            "the eighteenth signal must remain unread while the seventeenth waits for a permit"
        );

        run.abort();
        let _ = run.await;
        wait_for_count(&child_drops, MAX_INBOUND_TASKS).await;
        assert!(subscription_dropped.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn slow_child_does_not_block_later_sms_when_capacity_is_available() {
        let started = Arc::new(AtomicUsize::new(0));
        let child_drops = Arc::new(AtomicUsize::new(0));
        let source = Arc::new(ScriptedSource::new(ScriptedSubscription {
            messages: vec![
                Box::new(ScriptedSms::new(
                    "/org/freedesktop/ModemManager1/SMS/slow",
                    vec![PropertyAction::Wait {
                        started: started.clone(),
                        dropped: child_drops.clone(),
                    }],
                )) as Box<dyn InboundSmsAdapter>,
                Box::new(ScriptedSms::new(
                    "/org/freedesktop/ModemManager1/SMS/ready",
                    vec![PropertyAction::Return(Ok(properties(
                        "later message",
                        StorageType::Me as u32,
                    )))],
                )) as Box<dyn InboundSmsAdapter>,
            ]
            .into(),
            reads: Arc::new(AtomicUsize::new(0)),
            dropped: Arc::new(AtomicBool::new(false)),
        }));
        let store = Store::open_in_memory().unwrap();
        store
            .set_modem_fingerprint("concurrency-fingerprint".to_string())
            .await
            .unwrap();
        let worker = worker(store.clone(), source, settings(Vec::new(), Vec::new()));

        let run = tokio::spawn(async move {
            let mut children = JoinSet::new();
            worker.run_subscription(MODEM_PATH, &mut children).await
        });

        wait_for_count(&started, 1).await;
        tokio::time::timeout(Duration::from_secs(1), async {
            while store.sqlite().count_messages().unwrap() != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the later SMS completes while the first child is still pending");

        run.abort();
        let _ = run.await;
        wait_for_count(&child_drops, 1).await;
    }

    #[tokio::test]
    async fn dropping_worker_run_cancels_subscription_and_all_children() {
        let started = Arc::new(AtomicUsize::new(0));
        let child_drops = Arc::new(AtomicUsize::new(0));
        let reads = Arc::new(AtomicUsize::new(0));
        let subscription_dropped = Arc::new(AtomicBool::new(false));
        let source = Arc::new(ScriptedSource::new(ScriptedSubscription {
            messages: vec![Box::new(ScriptedSms::new(
                SMS_PATH,
                vec![PropertyAction::Wait {
                    started: started.clone(),
                    dropped: child_drops.clone(),
                }],
            )) as Box<dyn InboundSmsAdapter>]
            .into(),
            reads,
            dropped: subscription_dropped.clone(),
        }));
        let subscribed = source.subscribed.clone();
        let store = Store::open_in_memory().unwrap();
        let worker = worker(store, source, settings(Vec::new(), Vec::new()));

        let run = tokio::spawn(async move { worker.run().await });
        subscribed.notified().await;
        wait_for_count(&started, 1).await;

        run.abort();
        let _ = run.await;
        wait_for_count(&child_drops, 1).await;
        assert!(
            subscription_dropped.load(Ordering::SeqCst),
            "cancelling the runtime-owned worker future must drop its active subscription"
        );
    }

    #[tokio::test]
    async fn body_is_polled_until_available_then_persisted() {
        let store = Store::open_in_memory().unwrap();
        store
            .set_modem_fingerprint("body-poll-fingerprint".to_string())
            .await
            .unwrap();
        let sms = ScriptedSms::new(
            SMS_PATH,
            vec![
                PropertyAction::Return(Ok(properties("", StorageType::Me as u32))),
                PropertyAction::Return(Ok(properties("ready", StorageType::Me as u32))),
            ],
        );
        let calls = sms.calls.clone();

        process_incoming_sms(Box::new(sms), &[], messaging(store.clone()), Vec::new())
            .await
            .unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(store.sqlite().count_messages().unwrap(), 1);
    }

    #[tokio::test]
    async fn ignored_storage_stops_before_empty_body_polling() {
        let store = Store::open_in_memory().unwrap();
        store
            .set_modem_fingerprint("storage-filter-fingerprint".to_string())
            .await
            .unwrap();
        let sms = ScriptedSms::new(
            SMS_PATH,
            vec![PropertyAction::Return(Ok(properties(
                "",
                StorageType::Sm as u32,
            )))],
        );
        let calls = sms.calls.clone();

        process_incoming_sms(
            Box::new(sms),
            &[StorageType::Sm],
            messaging(store.clone()),
            Vec::new(),
        )
        .await
        .unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(store.sqlite().count_messages().unwrap(), 0);
    }

    #[tokio::test]
    async fn path_drift_dedupes_and_insert_creates_deliveries_atomically() {
        let store = Store::open_in_memory().unwrap();
        store
            .set_modem_fingerprint("stable-fingerprint".to_string())
            .await
            .unwrap();
        let profiles = vec!["bark.primary".to_string()];
        for path in [
            "/org/freedesktop/ModemManager1/SMS/1",
            "/org/freedesktop/ModemManager1/SMS/99",
        ] {
            process_incoming_sms(
                Box::new(ScriptedSms::new(
                    path,
                    vec![PropertyAction::Return(Ok(properties(
                        "same message",
                        StorageType::Me as u32,
                    )))],
                )),
                &[],
                messaging(store.clone()),
                profiles.clone(),
            )
            .await
            .unwrap();
        }

        assert_eq!(store.sqlite().count_messages().unwrap(), 1);
        assert_eq!(store.sqlite().count_deliveries().unwrap(), 1);
    }

    #[tokio::test]
    async fn persistence_failure_retries_until_enrollment_succeeds() {
        let store = Store::open_in_memory().unwrap();
        let processing_store = store.clone();
        let processing = tokio::spawn(async move {
            process_incoming_sms(
                Box::new(ScriptedSms::new(
                    SMS_PATH,
                    vec![PropertyAction::Return(Ok(properties(
                        "retry me",
                        StorageType::Me as u32,
                    )))],
                )),
                &[],
                messaging(processing_store),
                Vec::new(),
            )
            .await
        });

        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(
            !processing.is_finished(),
            "a persistence error must not terminate inbound processing"
        );
        store
            .set_modem_fingerprint("late-fingerprint".to_string())
            .await
            .unwrap();

        tokio::time::timeout(Duration::from_secs(1), processing)
            .await
            .expect("bounded retry reaches the successful persistence attempt")
            .unwrap()
            .unwrap();
        assert_eq!(store.sqlite().count_messages().unwrap(), 1);
    }

    #[test]
    fn persistence_retry_delay_is_capped_at_thirty_seconds() {
        let mut delay = INITIAL_PERSISTENCE_RETRY_DELAY;
        for _ in 0..20 {
            delay = next_persistence_retry_delay(delay);
        }
        assert_eq!(delay, MAX_PERSISTENCE_RETRY_DELAY);
        assert_eq!(
            next_persistence_retry_delay(delay),
            MAX_PERSISTENCE_RETRY_DELAY
        );
    }

    #[tokio::test]
    async fn enrolled_fingerprint_rejects_unrelated_modem_at_configured_path() {
        let store = Store::open_in_memory().unwrap();
        let target = ModemService::compute_fingerprint("target");
        store.set_modem_fingerprint(target.clone()).await.unwrap();
        let service = ModemService::new_with_runner(IdentityRunner);

        let resolved = resolve_monitor_path(MODEM_PATH, &service, &store)
            .await
            .unwrap();

        assert_eq!(resolved.as_deref(), Some(OTHER_MODEM_PATH));
        assert_eq!(
            store.modem_fingerprint().await.unwrap().as_deref(),
            Some(target.as_str())
        );
    }

    #[tokio::test]
    async fn matching_enrolled_fingerprint_backfills_before_monitoring() {
        let store = Store::open_in_memory().unwrap();
        store
            .sqlite()
            .insert_message(crate::storage::NewMessage {
                direction: MessageDirection::Inbound,
                phone_number: "+1".to_string(),
                body: "legacy".to_string(),
                timestamp: "2026-01-01T00:00:00Z".to_string(),
                status: MessageStatus::Received,
                source: MessageSource::Modem,
                modem_sms_path: Some("/org/freedesktop/ModemManager1/SMS/1".to_string()),
                read_at: None,
                error: None,
                inbound_dedupe_key: None,
            })
            .unwrap();
        let enrolled = ModemService::compute_fingerprint("other");
        store.set_modem_fingerprint(enrolled.clone()).await.unwrap();
        let service = ModemService::new_with_runner(IdentityRunner);

        let resolved = resolve_monitor_path(MODEM_PATH, &service, &store)
            .await
            .unwrap();

        assert_eq!(resolved.as_deref(), Some(MODEM_PATH));
        let replay = crate::storage::NewMessage::modem_inbound(
            "+1",
            "legacy",
            "2026-01-01T00:00:00Z",
            "/org/freedesktop/ModemManager1/SMS/99",
            &enrolled,
        );
        assert!(matches!(
            store
                .sqlite()
                .insert_inbound_message_with_deliveries(replay, &[])
                .unwrap(),
            crate::storage::InboundInsertResult::Duplicate(_)
        ));
        assert_eq!(store.sqlite().count_messages().unwrap(), 1);
    }
}
