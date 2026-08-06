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
use crate::modem::{ModemService, ModemTargets};
use crate::persistence::Store;

const MAX_INBOUND_TASKS: usize = 16;
const INITIAL_RECONNECT_DELAY: Duration = Duration::from_secs(5);
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(60);
const BODY_POLL_INTERVAL: Duration = Duration::from_millis(100);
const MAX_BODY_POLLS: usize = 600;
const INITIAL_PERSISTENCE_RETRY_DELAY: Duration = Duration::from_millis(100);
const MAX_PERSISTENCE_RETRY_DELAY: Duration = Duration::from_secs(30);
const LEGACY_SINGLE_MODEM_FINGERPRINT_SEED: &str = "sms-relayed-single-modem";
#[cfg(not(test))]
const RUNTIME_IDENTITY_REFRESH_INTERVAL: Duration = Duration::from_secs(60);
#[cfg(test)]
const RUNTIME_IDENTITY_REFRESH_INTERVAL: Duration = Duration::from_millis(10);

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
                    Ok(resolved) => publish_resolved_path(&self.modem_service, resolved),
                    Err(error) => {
                        error!("modem resolution failed: {}", error);
                        self.modem_service
                            .set_modem_targets(ModemTargets::default());
                        None
                    }
                };
            }
            let Some(path) = current_path.clone() else {
                warn!("no runtime modem path available; retrying resolution");
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
                    let resolved_path = match resolve_monitor_path(
                        &self.settings.configured_modem_path,
                        &self.modem_service,
                        &self.store,
                    )
                    .await
                    {
                        Ok(resolved) => publish_resolved_path(&self.modem_service, resolved),
                        Err(error) => {
                            error!("modem resolution failed: {}", error);
                            self.modem_service
                                .set_modem_targets(ModemTargets::default());
                            None
                        }
                    };
                    if let Some(new_path) = resolved_path {
                        if new_path != path {
                            info!("modem path changed from {} to {}", path, new_path);
                        }
                        current_path = Some(new_path);
                    } else {
                        warn!("modem re-resolution failed; will retry");
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
        let mut identity_refresh = tokio::time::interval(RUNTIME_IDENTITY_REFRESH_INTERVAL);
        identity_refresh.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        identity_refresh.tick().await;

        info!("SMS monitor ready on {}", actual_path);

        loop {
            let sms = tokio::select! {
                _ = identity_refresh.tick() => {
                    if let Err(error) = self.observe_runtime_identity(actual_path).await {
                        warn!("runtime modem identity refresh failed: {}", error);
                    }
                    continue;
                }
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

    async fn observe_runtime_identity(&self, actual_path: &str) -> Result<()> {
        retry_pending_identity_mismatches(&self.modem_service, &self.store).await?;
        let action_path = self.modem_service.verified_path();
        if let Some(action_path) = action_path.as_deref().filter(|path| *path != actual_path) {
            if let Some(identity) = self.modem_service.extract_identity(action_path).await {
                let fingerprint = ModemService::compute_fingerprint(&identity);
                match self.store.modem_fingerprint().await? {
                    None => {
                        self.store.set_modem_fingerprint(fingerprint).await?;
                        self.store.backfill_dedupe_keys().await?;
                    }
                    Some(enrolled) if enrolled == fingerprint => {}
                    Some(enrolled) => {
                        warn!("configured modem identity changed; revoking verified action target");
                        self.modem_service
                            .set_modem_targets(ModemTargets::runtime_only(actual_path));
                        persist_identity_mismatch(
                            &self.modem_service,
                            &self.store,
                            action_path,
                            &enrolled,
                        )
                        .await?;
                    }
                }
            }
        }

        let Some(identity) = self.modem_service.extract_identity(actual_path).await else {
            return Ok(());
        };
        let fingerprint = ModemService::compute_fingerprint(&identity);
        if action_path.as_deref() == Some(actual_path) {
            match self.store.modem_fingerprint().await? {
                None => {
                    self.store.set_modem_fingerprint(fingerprint).await?;
                    self.store.backfill_dedupe_keys().await?;
                }
                Some(enrolled) if enrolled == fingerprint => {}
                Some(enrolled) => {
                    warn!("configured modem identity changed; revoking verified action target");
                    self.modem_service
                        .set_modem_targets(ModemTargets::runtime_only(actual_path));
                    persist_identity_mismatch(
                        &self.modem_service,
                        &self.store,
                        actual_path,
                        &enrolled,
                    )
                    .await?;
                    self.store
                        .ensure_modem_dedupe_namespace_with(fingerprint.clone())
                        .await?;
                    self.store
                        .set_runtime_modem_fingerprint(fingerprint)
                        .await?;
                }
            }
        } else if self.store.runtime_modem_fingerprint().await?.as_deref()
            != Some(fingerprint.as_str())
        {
            self.store
                .ensure_modem_dedupe_namespace_with(fingerprint.clone())
                .await?;
            self.store
                .set_runtime_modem_fingerprint(fingerprint)
                .await?;
            self.store.backfill_dedupe_keys().await?;
        }
        Ok(())
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

fn publish_resolved_path(modem_service: &ModemService, resolved: ModemTargets) -> Option<String> {
    let path = resolved.runtime_path().map(ToString::to_string);
    modem_service.set_modem_targets(resolved);
    path
}

async fn persist_identity_mismatch(
    modem_service: &ModemService,
    store: &Store,
    modem_path: &str,
    enrolled_fingerprint: &str,
) -> Result<()> {
    modem_service.remember_pending_identity_mismatch(modem_path, enrolled_fingerprint);
    store
        .mark_modem_identity_mismatch(modem_path.to_string(), enrolled_fingerprint.to_string())
        .await?;
    modem_service.finish_pending_identity_mismatch(modem_path, enrolled_fingerprint);
    Ok(())
}

async fn retry_pending_identity_mismatches(
    modem_service: &ModemService,
    store: &Store,
) -> Result<()> {
    for (path, fingerprint) in modem_service.pending_identity_mismatches() {
        persist_identity_mismatch(modem_service, store, &path, &fingerprint).await?;
    }
    Ok(())
}

async fn clear_identity_mismatch(
    modem_service: &ModemService,
    store: &Store,
    modem_path: &str,
) -> Result<()> {
    store
        .clear_modem_identity_mismatch(modem_path.to_string())
        .await?;
    modem_service.clear_pending_identity_mismatch(modem_path);
    Ok(())
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

/// Resolve independent runtime and action modem targets.
///
/// A verified identity can serve both roles. When the configured action path
/// has no readable identity but a different runtime modem is matched, the
/// targets remain separate. A modem selected only by an observed runtime
/// fingerprint or because it is the sole available candidate is runtime-only
/// and must never receive control actions. An action-only result preserves an
/// exact configured target when runtime selection is unavailable or ambiguous;
/// the empty default means neither role could be resolved safely.
pub(crate) async fn resolve_monitor_path(
    configured_path: &str,
    modem_service: &ModemService,
    store: &Store,
) -> Result<ModemTargets> {
    retry_pending_identity_mismatches(modem_service, store).await?;
    let legacy_fingerprint =
        ModemService::compute_fingerprint(LEGACY_SINGLE_MODEM_FINGERPRINT_SEED);
    let stored_fingerprint = match store.modem_fingerprint().await? {
        Some(fingerprint) if fingerprint == legacy_fingerprint => {
            if store.migrate_legacy_modem_fingerprint(fingerprint).await? {
                None
            } else {
                store.modem_fingerprint().await?
            }
        }
        fingerprint => fingerprint,
    };
    let runtime_fingerprint = store.runtime_modem_fingerprint().await?;
    let needs_fallback_dedupe_namespace =
        stored_fingerprint.is_none() && runtime_fingerprint.is_none();
    let identity = modem_service.extract_identity(configured_path).await;
    let quarantined_fingerprint = store
        .modem_identity_mismatch_fingerprint(configured_path.to_string())
        .await?;
    let pending_quarantined_fingerprint = modem_service.pending_identity_mismatch(configured_path);
    let mut configured_identity_mismatch = stored_fingerprint.as_deref().is_some_and(|enrolled| {
        quarantined_fingerprint.as_deref() == Some(enrolled)
            || pending_quarantined_fingerprint.as_deref() == Some(enrolled)
    });
    if let Some(identity) = identity {
        let current_fingerprint = ModemService::compute_fingerprint(&identity);
        match stored_fingerprint.as_deref() {
            Some(enrolled_fingerprint) if enrolled_fingerprint == current_fingerprint => {
                clear_identity_mismatch(modem_service, store, configured_path).await?;
                store.backfill_dedupe_keys().await?;
                return Ok(ModemTargets::verified(configured_path));
            }
            Some(enrolled_fingerprint) => {
                configured_identity_mismatch = true;
                persist_identity_mismatch(
                    modem_service,
                    store,
                    configured_path,
                    enrolled_fingerprint,
                )
                .await?;
                warn!("configured modem identity changed; checking available modems");
            }
            None => {
                clear_identity_mismatch(modem_service, store, configured_path).await?;
                store.set_modem_fingerprint(current_fingerprint).await?;
                // Backfill dedupe keys for legacy modem-inbound messages now
                // that the fingerprint is available for stable hashing.
                store.backfill_dedupe_keys().await?;
                return Ok(ModemTargets::verified(configured_path));
            }
        }
    }

    let paths = modem_service.list_all_modem_paths().await;
    let mut candidates = Vec::with_capacity(paths.len());
    for path in paths {
        let fingerprint = modem_service
            .extract_identity(&path)
            .await
            .map(|identity| ModemService::compute_fingerprint(&identity));
        candidates.push((path, fingerprint));
    }
    let configured_action_path = candidates
        .iter()
        .find(|(path, fingerprint)| {
            !configured_identity_mismatch && path == configured_path && fingerprint.is_none()
        })
        .map(|(path, _)| path.clone());

    if let Some(enrolled_fingerprint) = stored_fingerprint.as_deref() {
        let mut matches = candidates.iter().filter_map(|(path, fingerprint)| {
            (fingerprint.as_deref() == Some(enrolled_fingerprint)).then(|| path.clone())
        });
        let matched = matches.next();
        let ambiguous = matches.next().is_some();
        if ambiguous {
            return Ok(configured_action_path
                .clone()
                .map(ModemTargets::action_only)
                .unwrap_or_default());
        }
        if let Some(path) = matched {
            clear_identity_mismatch(modem_service, store, &path).await?;
            store.backfill_dedupe_keys().await?;
            return Ok(ModemTargets::verified(path));
        }
    }

    if let Some(observed_fingerprint) = runtime_fingerprint.as_deref() {
        let mut matches = candidates.iter().filter_map(|(path, fingerprint)| {
            (fingerprint.as_deref() == Some(observed_fingerprint)).then(|| path.clone())
        });
        let matched = matches.next();
        let ambiguous = matches.next().is_some();
        if ambiguous {
            return Ok(configured_action_path
                .clone()
                .map(ModemTargets::action_only)
                .unwrap_or_default());
        }
        if let Some(path) = matched {
            store
                .ensure_modem_dedupe_namespace_with(observed_fingerprint.to_string())
                .await?;
            store.backfill_dedupe_keys().await?;
            return Ok(match configured_action_path {
                Some(action_path) => ModemTargets::separate(path, action_path),
                None => ModemTargets::runtime_only(path),
            });
        }
    }

    if candidates.len() != 1 {
        return Ok(configured_action_path
            .map(ModemTargets::action_only)
            .unwrap_or_default());
    }

    let (selected_path, selected_fingerprint) =
        candidates.into_iter().next().expect("one modem candidate");
    warn!(
        "selecting the only available modem as the runtime target at {}",
        selected_path
    );
    let resolved = if selected_path == configured_path && !configured_identity_mismatch {
        match selected_fingerprint {
            Some(fingerprint) if stored_fingerprint.is_none() => {
                store.set_modem_fingerprint(fingerprint).await?;
                ModemTargets::verified(selected_path)
            }
            Some(fingerprint) => {
                store
                    .ensure_modem_dedupe_namespace_with(fingerprint.clone())
                    .await?;
                store.set_runtime_modem_fingerprint(fingerprint).await?;
                ModemTargets::runtime_only(selected_path)
            }
            None => {
                if needs_fallback_dedupe_namespace {
                    warn!("modem identity is unavailable; using a local inbound dedupe namespace");
                    store.ensure_modem_dedupe_namespace().await?;
                }
                ModemTargets::verified(selected_path)
            }
        }
    } else {
        match selected_fingerprint {
            Some(fingerprint) => {
                store
                    .ensure_modem_dedupe_namespace_with(fingerprint.clone())
                    .await?;
                store.set_runtime_modem_fingerprint(fingerprint).await?;
            }
            None => {
                if needs_fallback_dedupe_namespace {
                    warn!("modem identity is unavailable; using a local inbound dedupe namespace");
                    store.ensure_modem_dedupe_namespace().await?;
                }
            }
        }
        ModemTargets::runtime_only(selected_path)
    };
    store.backfill_dedupe_keys().await?;
    Ok(resolved)
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
    use crate::modem::{MmcliOutput, MmcliRunner, ModemAction, ModemError};

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
        subscribed_paths: Arc<Mutex<Vec<String>>>,
    }

    impl ScriptedSource {
        fn new(subscription: ScriptedSubscription) -> Self {
            Self {
                subscription: Mutex::new(Some(Box::new(subscription))),
                subscribed: Arc::new(Notify::new()),
                subscribed_paths: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    impl InboundSourceAdapter for ScriptedSource {
        fn subscribe<'a>(
            &'a self,
            modem_path: &'a str,
        ) -> BoxFuture<'a, Result<Box<dyn InboundSubscriptionAdapter>>> {
            self.subscribed_paths
                .lock()
                .unwrap()
                .push(modem_path.to_string());
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

    #[derive(Clone)]
    struct SingleModemRunner {
        candidate_identity: Option<&'static str>,
    }

    impl MmcliRunner for SingleModemRunner {
        fn run<'a>(
            &'a self,
            args: &'a [&'a str],
            _timeout: Duration,
        ) -> Pin<Box<dyn Future<Output = Result<MmcliOutput, ModemError>> + Send + 'a>> {
            Box::pin(async move {
                let stdout = match args {
                    ["-L"] => {
                        format!("{OTHER_MODEM_PATH} [test] modem\n")
                    }
                    ["--modem", OTHER_MODEM_PATH, "--output-json"] => self
                        .candidate_identity
                        .map(|identity| {
                            format!(
                                r#"{{"modem":{{"generic":{{"equipment-identifier":"{identity}"}}}}}}"#
                            )
                        })
                        .unwrap_or_default(),
                    _ => String::new(),
                };
                Ok(MmcliOutput {
                    status_success: !stdout.is_empty(),
                    stdout,
                    stderr: String::new(),
                })
            })
        }
    }

    #[derive(Clone)]
    struct RecoveringIdentityRunner {
        candidate_identity: Arc<Mutex<Option<&'static str>>>,
    }

    impl MmcliRunner for RecoveringIdentityRunner {
        fn run<'a>(
            &'a self,
            args: &'a [&'a str],
            _timeout: Duration,
        ) -> Pin<Box<dyn Future<Output = Result<MmcliOutput, ModemError>> + Send + 'a>> {
            Box::pin(async move {
                let stdout = match args {
                    ["-L"] => format!("{OTHER_MODEM_PATH} [test] modem\n"),
                    ["--modem", OTHER_MODEM_PATH, "--output-json"] => self
                        .candidate_identity
                        .lock()
                        .unwrap()
                        .map(|identity| {
                            format!(
                                r#"{{"modem":{{"generic":{{"equipment-identifier":"{identity}"}}}}}}"#
                            )
                        })
                        .unwrap_or_default(),
                    _ => String::new(),
                };
                Ok(MmcliOutput {
                    status_success: !stdout.is_empty(),
                    stdout,
                    stderr: String::new(),
                })
            })
        }
    }

    #[derive(Clone)]
    struct MultipleModemsRunner;

    impl MmcliRunner for MultipleModemsRunner {
        fn run<'a>(
            &'a self,
            args: &'a [&'a str],
            _timeout: Duration,
        ) -> Pin<Box<dyn Future<Output = Result<MmcliOutput, ModemError>> + Send + 'a>> {
            Box::pin(async move {
                let stdout = match args {
                    ["-L"] => format!(
                        "{OTHER_MODEM_PATH} [test] first modem\n/org/freedesktop/ModemManager1/Modem/2 [test] second modem\n"
                    ),
                    _ => String::new(),
                };
                Ok(MmcliOutput {
                    status_success: !stdout.is_empty(),
                    stdout,
                    stderr: String::new(),
                })
            })
        }
    }

    #[derive(Clone)]
    struct ConfiguredModemWithoutIdentityRunner;

    impl MmcliRunner for ConfiguredModemWithoutIdentityRunner {
        fn run<'a>(
            &'a self,
            args: &'a [&'a str],
            _timeout: Duration,
        ) -> Pin<Box<dyn Future<Output = Result<MmcliOutput, ModemError>> + Send + 'a>> {
            Box::pin(async move {
                let stdout = match args {
                    ["-L"] => format!("{MODEM_PATH} [test] configured modem\n"),
                    _ => String::new(),
                };
                Ok(MmcliOutput {
                    status_success: !stdout.is_empty(),
                    stdout,
                    stderr: String::new(),
                })
            })
        }
    }

    #[derive(Clone)]
    struct ConfiguredIdentityRunner {
        identity: Arc<Mutex<Option<&'static str>>>,
    }

    impl MmcliRunner for ConfiguredIdentityRunner {
        fn run<'a>(
            &'a self,
            args: &'a [&'a str],
            _timeout: Duration,
        ) -> Pin<Box<dyn Future<Output = Result<MmcliOutput, ModemError>> + Send + 'a>> {
            Box::pin(async move {
                let stdout = match args {
                    ["-L"] => format!("{MODEM_PATH} [test] configured modem\n"),
                    ["--modem", MODEM_PATH, "--output-json"] => self
                        .identity
                        .lock()
                        .unwrap()
                        .map(|identity| {
                            format!(
                                r#"{{"modem":{{"generic":{{"equipment-identifier":"{identity}"}}}}}}"#
                            )
                        })
                        .unwrap_or_default(),
                    _ => String::new(),
                };
                Ok(MmcliOutput {
                    status_success: !stdout.is_empty(),
                    stdout,
                    stderr: String::new(),
                })
            })
        }
    }

    #[derive(Clone)]
    struct RuntimeAndConfiguredModemsRunner;

    impl MmcliRunner for RuntimeAndConfiguredModemsRunner {
        fn run<'a>(
            &'a self,
            args: &'a [&'a str],
            _timeout: Duration,
        ) -> Pin<Box<dyn Future<Output = Result<MmcliOutput, ModemError>> + Send + 'a>> {
            Box::pin(async move {
                let stdout = match args {
                    ["-L"] => format!(
                        "{MODEM_PATH} [test] configured modem\n{OTHER_MODEM_PATH} [test] runtime modem\n"
                    ),
                    ["--modem", OTHER_MODEM_PATH, "--output-json"] => {
                        r#"{"modem":{"generic":{"equipment-identifier":"target"}}}"#.to_string()
                    }
                    _ => String::new(),
                };
                Ok(MmcliOutput {
                    status_success: !stdout.is_empty(),
                    stdout,
                    stderr: String::new(),
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

        assert_eq!(resolved.runtime_path(), Some(OTHER_MODEM_PATH));
        assert_eq!(
            store.modem_fingerprint().await.unwrap().as_deref(),
            Some(target.as_str())
        );
    }

    #[tokio::test]
    async fn stale_configured_path_selects_the_only_available_modem() {
        let store = Store::open_in_memory().unwrap();
        let service = ModemService::new_with_runner(SingleModemRunner {
            candidate_identity: Some("target"),
        });

        let resolved = resolve_monitor_path(MODEM_PATH, &service, &store)
            .await
            .unwrap();

        assert_eq!(resolved.runtime_path(), Some(OTHER_MODEM_PATH));
        assert_eq!(
            store.runtime_modem_fingerprint().await.unwrap().as_deref(),
            Some(ModemService::compute_fingerprint("target").as_str())
        );
        assert_eq!(store.modem_fingerprint().await.unwrap(), None);
    }

    #[tokio::test]
    async fn observed_runtime_identity_never_becomes_a_verified_action_target() {
        let store = Store::open_in_memory().unwrap();
        let service = ModemService::new_with_runner(SingleModemRunner {
            candidate_identity: Some("target"),
        });

        let first = resolve_monitor_path(MODEM_PATH, &service, &store)
            .await
            .unwrap();
        publish_resolved_path(&service, first);
        assert_eq!(service.verified_path(), None);

        let second = resolve_monitor_path(MODEM_PATH, &service, &store)
            .await
            .unwrap();
        publish_resolved_path(&service, second);

        assert_eq!(service.verified_path(), None);
        assert_eq!(store.modem_fingerprint().await.unwrap(), None);
    }

    #[tokio::test]
    async fn worker_keeps_the_only_available_modem_out_of_the_verified_action_path() {
        let source = Arc::new(ScriptedSource::new(ScriptedSubscription {
            messages: VecDeque::new(),
            reads: Arc::new(AtomicUsize::new(0)),
            dropped: Arc::new(AtomicBool::new(false)),
        }));
        let subscribed = source.subscribed.clone();
        let subscribed_paths = source.subscribed_paths.clone();
        let store = Store::open_in_memory().unwrap();
        let modem_service = ModemService::new_with_runner(SingleModemRunner {
            candidate_identity: Some("target"),
        });
        let worker = InboundWorker::new(
            store.clone(),
            messaging(store),
            modem_service.clone(),
            settings(Vec::new(), Vec::new()),
        )
        .with_source(source);

        let run = tokio::spawn(async move { worker.run().await });
        tokio::time::timeout(Duration::from_secs(1), subscribed.notified())
            .await
            .expect("worker subscribes after resolving the modem");

        assert_eq!(
            subscribed_paths.lock().unwrap().as_slice(),
            [OTHER_MODEM_PATH]
        );
        assert_eq!(modem_service.verified_path(), None);
        let action_error = modem_service
            .run_action("test-session", ModemAction::Disable)
            .await
            .unwrap_err();
        assert_eq!(action_error.code(), "modem_path_unresolved");

        run.abort();
        let _ = run.await;
    }

    #[tokio::test]
    async fn worker_observes_runtime_identity_when_it_becomes_available() {
        let source = Arc::new(ScriptedSource::new(ScriptedSubscription {
            messages: VecDeque::new(),
            reads: Arc::new(AtomicUsize::new(0)),
            dropped: Arc::new(AtomicBool::new(false)),
        }));
        let subscribed = source.subscribed.clone();
        let candidate_identity = Arc::new(Mutex::new(None));
        let store = Store::open_in_memory().unwrap();
        let modem_service = ModemService::new_with_runner(RecoveringIdentityRunner {
            candidate_identity: candidate_identity.clone(),
        });
        let worker = InboundWorker::new(
            store.clone(),
            messaging(store.clone()),
            modem_service.clone(),
            settings(Vec::new(), Vec::new()),
        )
        .with_source(source);

        let run = tokio::spawn(async move { worker.run().await });
        subscribed.notified().await;
        assert_eq!(store.runtime_modem_fingerprint().await.unwrap(), None);

        *candidate_identity.lock().unwrap() = Some("target");
        let expected = ModemService::compute_fingerprint("target");
        tokio::time::timeout(Duration::from_secs(1), async {
            while store.runtime_modem_fingerprint().await.unwrap().as_deref()
                != Some(expected.as_str())
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the refresh observes the recovered runtime identity");

        assert_eq!(
            store.runtime_modem_fingerprint().await.unwrap().as_deref(),
            Some(expected.as_str())
        );
        assert_eq!(modem_service.verified_path(), None);

        run.abort();
        let _ = run.await;
    }

    #[tokio::test]
    async fn only_available_modem_is_selected_when_identity_is_temporarily_unavailable() {
        let store = Store::open_in_memory().unwrap();
        let enrolled = ModemService::compute_fingerprint("target");
        store.set_modem_fingerprint(enrolled.clone()).await.unwrap();
        let service = ModemService::new_with_runner(SingleModemRunner {
            candidate_identity: None,
        });

        let resolved = resolve_monitor_path(MODEM_PATH, &service, &store)
            .await
            .unwrap();

        assert_eq!(resolved.runtime_path(), Some(OTHER_MODEM_PATH));
        assert_eq!(
            store.modem_fingerprint().await.unwrap().as_deref(),
            Some(enrolled.as_str())
        );
    }

    #[tokio::test]
    async fn temporary_identity_loss_keeps_the_existing_inbound_dedupe_namespace() {
        let store = Store::open_in_memory().unwrap();
        let enrolled = ModemService::compute_fingerprint("target");
        store.set_modem_fingerprint(enrolled).await.unwrap();
        process_incoming_sms(
            Box::new(ScriptedSms::new(
                SMS_PATH,
                vec![PropertyAction::Return(Ok(properties(
                    "same message",
                    StorageType::Me as u32,
                )))],
            )),
            &[],
            messaging(store.clone()),
            Vec::new(),
        )
        .await
        .unwrap();

        let unavailable = ModemService::new_with_runner(SingleModemRunner {
            candidate_identity: None,
        });
        resolve_monitor_path(MODEM_PATH, &unavailable, &store)
            .await
            .unwrap();
        process_incoming_sms(
            Box::new(ScriptedSms::new(
                "/org/freedesktop/ModemManager1/SMS/99",
                vec![PropertyAction::Return(Ok(properties(
                    "same message",
                    StorageType::Me as u32,
                )))],
            )),
            &[],
            messaging(store.clone()),
            Vec::new(),
        )
        .await
        .unwrap();

        assert_eq!(store.sqlite().count_messages().unwrap(), 1);
    }

    #[tokio::test]
    async fn configured_path_remains_a_verified_action_target_when_identity_is_unavailable() {
        let store = Store::open_in_memory().unwrap();
        let service = ModemService::new_with_runner(ConfiguredModemWithoutIdentityRunner);

        let resolved = resolve_monitor_path(MODEM_PATH, &service, &store)
            .await
            .unwrap();
        publish_resolved_path(&service, resolved);

        assert_eq!(service.verified_path().as_deref(), Some(MODEM_PATH));
        assert_eq!(store.modem_fingerprint().await.unwrap(), None);
    }

    #[tokio::test]
    async fn configured_identity_mismatch_stays_quarantined_when_identity_becomes_unavailable() {
        let store = Store::open_in_memory().unwrap();
        store
            .set_modem_fingerprint(ModemService::compute_fingerprint("target"))
            .await
            .unwrap();
        let identity = Arc::new(Mutex::new(Some("other")));
        let service = ModemService::new_with_runner(ConfiguredIdentityRunner {
            identity: identity.clone(),
        });

        let mismatched = resolve_monitor_path(MODEM_PATH, &service, &store)
            .await
            .unwrap();
        assert_eq!(mismatched.runtime_path(), Some(MODEM_PATH));
        assert_eq!(mismatched.action_path(), None);
        assert_eq!(
            store
                .modem_identity_mismatch_fingerprint(MODEM_PATH.to_string())
                .await
                .unwrap()
                .as_deref(),
            Some(ModemService::compute_fingerprint("target").as_str())
        );

        *identity.lock().unwrap() = None;
        let unavailable = resolve_monitor_path(MODEM_PATH, &service, &store)
            .await
            .unwrap();

        assert_eq!(unavailable.runtime_path(), Some(MODEM_PATH));
        assert_eq!(unavailable.action_path(), None);

        *identity.lock().unwrap() = Some("target");
        let recovered = resolve_monitor_path(MODEM_PATH, &service, &store)
            .await
            .unwrap();

        assert_eq!(recovered.action_path(), Some(MODEM_PATH));
        assert_eq!(
            store
                .modem_identity_mismatch_fingerprint(MODEM_PATH.to_string())
                .await
                .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn failed_mismatch_persistence_still_blocks_action_during_identity_loss() {
        let store = Store::open_in_memory().unwrap();
        store
            .set_modem_fingerprint(ModemService::compute_fingerprint("target"))
            .await
            .unwrap();
        store.fail_next_modem_identity_mismatch_marks(1);
        let identity = Arc::new(Mutex::new(Some("other")));
        let modem_service = ModemService::new_with_runner(ConfiguredIdentityRunner {
            identity: identity.clone(),
        });
        modem_service.set_verified_path(Some(MODEM_PATH.to_string()));
        let worker = InboundWorker::new(
            store.clone(),
            messaging(store.clone()),
            modem_service.clone(),
            settings(Vec::new(), Vec::new()),
        );

        assert!(worker.observe_runtime_identity(MODEM_PATH).await.is_err());
        assert_eq!(modem_service.verified_path(), None);
        *identity.lock().unwrap() = None;

        let resolved = resolve_monitor_path(MODEM_PATH, &modem_service, &store)
            .await
            .unwrap();

        assert_eq!(resolved.runtime_path(), Some(MODEM_PATH));
        assert_eq!(resolved.action_path(), None);
        assert_eq!(
            store
                .modem_identity_mismatch_fingerprint(MODEM_PATH.to_string())
                .await
                .unwrap()
                .as_deref(),
            Some(ModemService::compute_fingerprint("target").as_str())
        );
    }

    #[tokio::test]
    async fn runtime_rebind_keeps_the_exact_configured_action_target() {
        let store = Store::open_in_memory().unwrap();
        store
            .set_runtime_modem_fingerprint(ModemService::compute_fingerprint("target"))
            .await
            .unwrap();
        let service = ModemService::new_with_runner(RuntimeAndConfiguredModemsRunner);

        let resolved = resolve_monitor_path(MODEM_PATH, &service, &store)
            .await
            .unwrap();
        publish_resolved_path(&service, resolved);

        assert_eq!(service.runtime_path().as_deref(), Some(OTHER_MODEM_PATH));
        assert_eq!(service.verified_path().as_deref(), Some(MODEM_PATH));
    }

    #[tokio::test]
    async fn runtime_rebind_dedupe_survives_configured_action_identity_enrollment() {
        let store = Store::open_in_memory().unwrap();
        store
            .set_runtime_modem_fingerprint(ModemService::compute_fingerprint("target"))
            .await
            .unwrap();
        let service = ModemService::new_with_runner(RuntimeAndConfiguredModemsRunner);
        resolve_monitor_path(MODEM_PATH, &service, &store)
            .await
            .unwrap();
        process_incoming_sms(
            Box::new(ScriptedSms::new(
                SMS_PATH,
                vec![PropertyAction::Return(Ok(properties(
                    "same message",
                    StorageType::Me as u32,
                )))],
            )),
            &[],
            messaging(store.clone()),
            Vec::new(),
        )
        .await
        .unwrap();

        store
            .set_modem_fingerprint(ModemService::compute_fingerprint("configured"))
            .await
            .unwrap();
        process_incoming_sms(
            Box::new(ScriptedSms::new(
                "/org/freedesktop/ModemManager1/SMS/99",
                vec![PropertyAction::Return(Ok(properties(
                    "same message",
                    StorageType::Me as u32,
                )))],
            )),
            &[],
            messaging(store.clone()),
            Vec::new(),
        )
        .await
        .unwrap();

        assert_eq!(store.sqlite().count_messages().unwrap(), 1);
    }

    #[tokio::test]
    async fn first_only_modem_without_identity_receives_without_enrolling_a_fake_identity() {
        let store = Store::open_in_memory().unwrap();
        let service = ModemService::new_with_runner(SingleModemRunner {
            candidate_identity: None,
        });

        let resolved = resolve_monitor_path(MODEM_PATH, &service, &store)
            .await
            .unwrap();

        assert_eq!(resolved.runtime_path(), Some(OTHER_MODEM_PATH));
        assert_eq!(store.modem_fingerprint().await.unwrap(), None);

        process_incoming_sms(
            Box::new(ScriptedSms::new(
                SMS_PATH,
                vec![PropertyAction::Return(Ok(properties(
                    "identity unavailable",
                    StorageType::Me as u32,
                )))],
            )),
            &[],
            messaging(store.clone()),
            Vec::new(),
        )
        .await
        .unwrap();
        assert_eq!(store.sqlite().count_messages().unwrap(), 1);
    }

    #[tokio::test]
    async fn recovered_runtime_identity_is_observed_without_changing_inbound_dedupe() {
        let store = Store::open_in_memory().unwrap();
        let unavailable = ModemService::new_with_runner(SingleModemRunner {
            candidate_identity: None,
        });
        resolve_monitor_path(MODEM_PATH, &unavailable, &store)
            .await
            .unwrap();
        process_incoming_sms(
            Box::new(ScriptedSms::new(
                SMS_PATH,
                vec![PropertyAction::Return(Ok(properties(
                    "same message",
                    StorageType::Me as u32,
                )))],
            )),
            &[],
            messaging(store.clone()),
            Vec::new(),
        )
        .await
        .unwrap();

        let recovered = ModemService::new_with_runner(SingleModemRunner {
            candidate_identity: Some("target"),
        });
        resolve_monitor_path(MODEM_PATH, &recovered, &store)
            .await
            .unwrap();
        assert_eq!(
            store.runtime_modem_fingerprint().await.unwrap().as_deref(),
            Some(ModemService::compute_fingerprint("target").as_str())
        );
        assert_eq!(store.modem_fingerprint().await.unwrap(), None);

        process_incoming_sms(
            Box::new(ScriptedSms::new(
                "/org/freedesktop/ModemManager1/SMS/99",
                vec![PropertyAction::Return(Ok(properties(
                    "same message",
                    StorageType::Me as u32,
                )))],
            )),
            &[],
            messaging(store.clone()),
            Vec::new(),
        )
        .await
        .unwrap();
        assert_eq!(store.sqlite().count_messages().unwrap(), 1);
    }

    #[tokio::test]
    async fn legacy_single_modem_fingerprint_migrates_without_changing_inbound_dedupe() {
        let store = Store::open_in_memory().unwrap();
        let legacy_fingerprint = ModemService::compute_fingerprint("sms-relayed-single-modem");
        store
            .set_modem_fingerprint(legacy_fingerprint)
            .await
            .unwrap();
        process_incoming_sms(
            Box::new(ScriptedSms::new(
                SMS_PATH,
                vec![PropertyAction::Return(Ok(properties(
                    "same message",
                    StorageType::Me as u32,
                )))],
            )),
            &[],
            messaging(store.clone()),
            Vec::new(),
        )
        .await
        .unwrap();

        let recovered = ModemService::new_with_runner(SingleModemRunner {
            candidate_identity: Some("target"),
        });
        resolve_monitor_path(MODEM_PATH, &recovered, &store)
            .await
            .unwrap();

        assert_eq!(store.modem_fingerprint().await.unwrap(), None);
        assert_eq!(
            store.runtime_modem_fingerprint().await.unwrap().as_deref(),
            Some(ModemService::compute_fingerprint("target").as_str())
        );
        process_incoming_sms(
            Box::new(ScriptedSms::new(
                "/org/freedesktop/ModemManager1/SMS/99",
                vec![PropertyAction::Return(Ok(properties(
                    "same message",
                    StorageType::Me as u32,
                )))],
            )),
            &[],
            messaging(store.clone()),
            Vec::new(),
        )
        .await
        .unwrap();
        assert_eq!(store.sqlite().count_messages().unwrap(), 1);
    }

    #[tokio::test]
    async fn stale_configured_path_does_not_select_from_multiple_unidentified_modems() {
        let store = Store::open_in_memory().unwrap();
        let service = ModemService::new_with_runner(MultipleModemsRunner);

        let resolved = resolve_monitor_path(MODEM_PATH, &service, &store)
            .await
            .unwrap();

        assert_eq!(resolved, ModemTargets::default());
        assert_eq!(store.modem_fingerprint().await.unwrap(), None);
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

        assert_eq!(resolved.runtime_path(), Some(MODEM_PATH));
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
