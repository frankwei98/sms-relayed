use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use log::{error, info, warn};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use zbus::zvariant::{OwnedValue, Value};

use crate::config::AppConfig;
use crate::modem::ModemService;
use crate::persistence::Store;

mod connection;
mod inbound;
mod outbound;

#[allow(unused_imports)]
pub(crate) use inbound::{
    InboundEvent, InboundSms, InboundSmsProperties, InboundSubscription, SystemInboundSource,
};

// Preserve the existing `crate::dbus` facade, including raw helpers with no in-crate caller.
#[allow(unused_imports)]
pub use outbound::{
    create_sms, get_sms_snapshot, get_sms_state, send_prepared_sms, ModemSmsState, PreparedSms,
    SendAttemptOutcome, SmsSender, SmsSnapshot, SystemSmsSender,
};

#[derive(Debug, Clone)]
pub struct ReceivedSms {
    pub phone_number: String,
    pub body: String,
    pub timestamp: String,
    pub modem_sms_path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum StorageType {
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
    pub fn from_config(s: &str) -> Self {
        match s {
            "unknown" => StorageType::Unknown,
            "sm" => StorageType::Sm,
            "me" => StorageType::Me,
            "mt" => StorageType::Mt,
            "sr" => StorageType::Sr,
            "bm" => StorageType::Bm,
            "ta" => StorageType::Ta,
            "all" => StorageType::All,
            _ => {
                warn!(
                    "unknown storage type: {}; storage will not be filtered by this entry",
                    s
                );
                StorageType::NoMatch
            }
        }
    }

    fn should_ignore(&self, storage: u32) -> bool {
        match self {
            StorageType::All | StorageType::NoMatch => false,
            _ => *self as u32 == storage,
        }
    }
}

const MM_SMS_INTERFACE: &str = "org.freedesktop.ModemManager1.Sms";
const MM_MESSAGING_INTERFACE: &str = "org.freedesktop.ModemManager1.Modem.Messaging";
const DBUS_PROPERTIES_INTERFACE: &str = "org.freedesktop.DBus.Properties";
const DBUS_INTERFACE: &str = "org.freedesktop.DBus";
const OBJECT_MANAGER_INTERFACE: &str = "org.freedesktop.DBus.ObjectManager";
const MM_DESTINATION: &str = "org.freedesktop.ModemManager1";

const MAX_INBOUND_TASKS: usize = 16;

fn extract_string(props: &HashMap<String, OwnedValue>, key: &str) -> String {
    props
        .get(key)
        .and_then(|v| {
            let val: Value = (**v).clone();
            if let Value::Str(s) = val {
                Some(s.to_string())
            } else {
                None
            }
        })
        .unwrap_or_default()
}

fn extract_u32(props: &HashMap<String, OwnedValue>, key: &str) -> u32 {
    props
        .get(key)
        .and_then(|v| {
            let val: Value = (**v).clone();
            if let Value::U32(n) = val {
                Some(n)
            } else {
                None
            }
        })
        .unwrap_or(100)
}

/// Resolve the actual modem path for monitoring.
/// First tries the configured path directly. If it fails and a fingerprint is
/// stored, scans all modems and matches by fingerprint exactly once.
async fn get_stored_fingerprint(store: &Store) -> Result<Option<String>> {
    store.modem_fingerprint().await
}

async fn set_stored_fingerprint(store: &Store, fingerprint: String) -> Result<()> {
    store.set_modem_fingerprint(fingerprint).await
}

async fn backfill_dedupe_keys(store: &Store) -> Result<()> {
    store.backfill_dedupe_keys().await?;
    Ok(())
}

pub(crate) async fn resolve_monitor_path(
    configured_path: &str,
    modem_service: &ModemService,
    store: &Store,
) -> Result<Option<String>> {
    // Try configured path first
    let stored_fp = get_stored_fingerprint(store).await?;
    let identity = modem_service.extract_identity(configured_path).await;
    if let Some(identity) = identity {
        let current_fp = ModemService::compute_fingerprint(&identity);
        match stored_fp.as_deref() {
            Some(enrolled_fp) if enrolled_fp == current_fp => {
                backfill_dedupe_keys(store).await?;
                return Ok(Some(configured_path.to_string()));
            }
            Some(enrolled_fp) => {
                warn!("configured modem identity changed; refusing path reuse");
                return Ok(modem_service.scan_and_match_fingerprint(enrolled_fp).await);
            }
            None => {
                set_stored_fingerprint(store, current_fp).await?;
                // Backfill dedupe keys for legacy modem-inbound messages now
                // that the fingerprint is available for stable hashing.
                backfill_dedupe_keys(store).await?;
                return Ok(Some(configured_path.to_string()));
            }
        }
    }

    // Configured path failed; try fingerprint match
    let Some(stored_fp) = stored_fp.as_deref() else {
        return Ok(None);
    };
    Ok(modem_service.scan_and_match_fingerprint(stored_fp).await)
}

async fn run_subscription<F, Fut>(
    actual_path: &str,
    config: &AppConfig,
    on_received: F,
    inbound_limit: Arc<Semaphore>,
) -> Result<()>
where
    F: Fn(ReceivedSms) -> Fut + Send + Clone + 'static,
    Fut: std::future::Future<Output = Result<()>> + Send + 'static,
{
    let source = SystemInboundSource::new();
    let mut subscription = source.subscribe(actual_path).await?;

    info!("SMS monitor ready on {}", actual_path);

    let ignored_storage: Vec<StorageType> = config
        .sms
        .ignore_storage
        .iter()
        .map(|s| StorageType::from_config(s))
        .collect();

    loop {
        let InboundEvent::Added(sms) = subscription.next().await?;
        info!("SmsPath:\n{}", sms.path());
        let permit = inbound_limit
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| anyhow::anyhow!("inbound task limiter closed"))?;
        let task_storage_filters = ignored_storage.clone();
        let task_handler = on_received.clone();
        spawn_inbound_task(permit, async move {
            handle_incoming_sms(sms, &task_storage_filters, task_handler).await
        });
    }
}

async fn handle_incoming_sms<F, Fut>(
    sms: InboundSms,
    storage_filters: &[StorageType],
    on_received: F,
) -> Result<()>
where
    F: Fn(ReceivedSms) -> Fut + Send,
    Fut: std::future::Future<Output = Result<()>> + Send,
{
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
            let mut delay = Duration::from_millis(100);
            loop {
                match on_received(received.clone()).await {
                    Ok(()) => break,
                    Err(e) => {
                        error!("persist incoming SMS failed; retrying: {}", e);
                        crate::monitoring::capture_failure("dbus", "dbus.inbound_persist_failed");
                        tokio::time::sleep(delay).await;
                        delay = (delay * 2).min(Duration::from_secs(30));
                    }
                }
            }
            return Ok(());
        } else {
            retries += 1;
            if retries % 50 == 0 {
                warn!("短信内容为空，已重试{}次", retries);
            }
            if retries > 600 {
                warn!("短信内容为空，重试次数过多，放弃");
                crate::monitoring::capture_failure("dbus", "dbus.sms_body_unavailable");
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
}

fn spawn_inbound_task(
    permit: OwnedSemaphorePermit,
    task: impl std::future::Future<Output = Result<()>> + Send + 'static,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let _permit = permit;
        if task.await.is_err() {
            error!("incoming SMS processing task failed");
            crate::monitoring::capture_failure("dbus", "dbus.inbound_processing_failed");
        }
    })
}

pub async fn monitor_dbus_with_handler<F, Fut>(
    configured_modem_path: &str,
    config: &AppConfig,
    on_received: F,
    modem_service: &ModemService,
    store: &Store,
) -> Result<()>
where
    F: Fn(ReceivedSms) -> Fut + Send + Clone + 'static,
    Fut: std::future::Future<Output = Result<()>> + Send + 'static,
{
    println!("短信转发模式正在启动，正在连接系统 D-Bus。");
    info!("正在运行. 按下 Ctrl-C 停止.");

    let inbound_limit = Arc::new(Semaphore::new(MAX_INBOUND_TASKS));
    let mut current_path = None;
    let mut delay = Duration::from_secs(5);
    let max_delay = Duration::from_secs(60);

    loop {
        if current_path.is_none() {
            current_path =
                match resolve_monitor_path(configured_modem_path, modem_service, store).await {
                    Ok(path) => {
                        modem_service.set_verified_path(path.clone());
                        path
                    }
                    Err(error) => {
                        error!("modem resolution failed: {}", error);
                        modem_service.set_verified_path(None);
                        None
                    }
                };
        }
        let Some(path) = current_path.clone() else {
            warn!("no verified modem identity available; retrying resolution");
            tokio::time::sleep(delay).await;
            delay = (delay * 2).min(max_delay);
            continue;
        };
        match run_subscription(&path, config, on_received.clone(), inbound_limit.clone()).await {
            Ok(()) => {
                delay = Duration::from_secs(5);
            }
            Err(e) => {
                error!("D-Bus monitor lost: {}", e);
                crate::monitoring::capture_failure("dbus", "dbus.monitor_lost");
                let resolved =
                    match resolve_monitor_path(configured_modem_path, modem_service, store).await {
                        Ok(resolved) => resolved,
                        Err(error) => {
                            error!("modem resolution failed: {}", error);
                            modem_service.set_verified_path(None);
                            None
                        }
                    };
                if let Some(new_path) = resolved {
                    if new_path != path {
                        info!("modem path changed from {} to {}", path, new_path);
                    }
                    modem_service.set_verified_path(Some(new_path.clone()));
                    current_path = Some(new_path);
                } else {
                    warn!("modem re-resolution failed; will retry");
                    modem_service.set_verified_path(None);
                    current_path = None;
                }
            }
        }

        info!("reconnecting in {}s...", delay.as_secs_f64());
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(max_delay);
    }
}

fn should_ignore_storage(storage: u32, filters: &[StorageType]) -> bool {
    filters
        .iter()
        .any(|filter| !matches!(filter, StorageType::All) && filter.should_ignore(storage))
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::time::Duration as StdDuration;

    use tokio::sync::Notify;

    use super::*;

    use crate::modem::{MmcliOutput, MmcliRunner, ModemError};

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
                    ["--modem", "/org/freedesktop/ModemManager1/Modem/0", "--output-json"] => {
                        r#"{"modem":{"generic":{"equipment-identifier":"other"}}}"#
                    }
                    ["-L"] => "/org/freedesktop/ModemManager1/Modem/1 [test] modem\n",
                    ["--modem", "/org/freedesktop/ModemManager1/Modem/1", "--output-json"] => {
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

    #[tokio::test]
    async fn slow_inbound_work_does_not_block_later_inbound_work() {
        let first_started = Arc::new(Notify::new());
        let second_completed = Arc::new(Notify::new());
        let limiter = Arc::new(Semaphore::new(2));

        let first_started_task = first_started.clone();
        let first =
            spawn_inbound_task(limiter.clone().acquire_owned().await.unwrap(), async move {
                first_started_task.notify_one();
                std::future::pending::<Result<()>>().await
            });
        first_started.notified().await;

        let second_completed_task = second_completed.clone();
        let second =
            spawn_inbound_task(limiter.clone().acquire_owned().await.unwrap(), async move {
                second_completed_task.notify_one();
                Ok(())
            });
        tokio::time::timeout(StdDuration::from_millis(100), second_completed.notified())
            .await
            .expect("a later inbound SMS must not wait for an earlier slow task");
        second.await.unwrap();
        first.abort();
    }

    #[tokio::test]
    async fn inbound_limit_backpressures_and_detached_tasks_release_permits() {
        let limiter = Arc::new(Semaphore::new(MAX_INBOUND_TASKS));
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let started_task = started.clone();
        let release_task = release.clone();
        let detached =
            spawn_inbound_task(limiter.clone().acquire_owned().await.unwrap(), async move {
                started_task.notify_one();
                release_task.notified().await;
                Ok(())
            });
        started.notified().await;
        drop(detached);

        let mut held = Vec::new();
        for _ in 1..MAX_INBOUND_TASKS {
            held.push(limiter.clone().acquire_owned().await.unwrap());
        }
        assert!(
            tokio::time::timeout(
                StdDuration::from_millis(20),
                limiter.clone().acquire_owned()
            )
            .await
            .is_err(),
            "the seventeenth inbound task must wait"
        );

        release.notify_one();
        let released = tokio::time::timeout(StdDuration::from_millis(100), limiter.acquire_owned())
            .await
            .expect("a finished detached task must release its permit")
            .unwrap();
        drop(released);
        drop(held);
    }

    #[tokio::test]
    async fn enrolled_fingerprint_rejects_unrelated_modem_at_configured_path() {
        let store = Store::open_in_memory().unwrap();
        let target = ModemService::compute_fingerprint("target");
        store.set_modem_fingerprint(target.clone()).await.unwrap();
        let service = ModemService::new_with_runner(IdentityRunner);

        let resolved =
            resolve_monitor_path("/org/freedesktop/ModemManager1/Modem/0", &service, &store)
                .await
                .unwrap();

        assert_eq!(
            resolved.as_deref(),
            Some("/org/freedesktop/ModemManager1/Modem/1")
        );
        assert_eq!(
            store.modem_fingerprint().await.unwrap().as_deref(),
            Some(target.as_str())
        );
    }

    #[tokio::test]
    async fn matching_enrolled_fingerprint_backfills_legacy_messages_before_monitoring() {
        let store = Store::open_in_memory().unwrap();
        store
            .sqlite()
            .insert_message(crate::storage::NewMessage {
                direction: crate::message::MessageDirection::Inbound,
                phone_number: "+1".to_string(),
                body: "legacy".to_string(),
                timestamp: "2026-01-01T00:00:00Z".to_string(),
                status: crate::message::MessageStatus::Received,
                source: crate::message::MessageSource::Modem,
                modem_sms_path: Some("/org/freedesktop/ModemManager1/SMS/1".to_string()),
                read_at: None,
                error: None,
                inbound_dedupe_key: None,
            })
            .unwrap();
        let enrolled = ModemService::compute_fingerprint("other");
        store.set_modem_fingerprint(enrolled.clone()).await.unwrap();
        let service = ModemService::new_with_runner(IdentityRunner);

        let resolved =
            resolve_monitor_path("/org/freedesktop/ModemManager1/Modem/0", &service, &store)
                .await
                .unwrap();

        assert_eq!(
            resolved.as_deref(),
            Some("/org/freedesktop/ModemManager1/Modem/0")
        );
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
