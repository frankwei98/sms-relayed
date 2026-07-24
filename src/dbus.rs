use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use futures_util::StreamExt;
use log::{error, info, warn};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use zbus::zvariant::{OwnedValue, Value};
use zbus::Connection;

use crate::config::AppConfig;
use crate::modem::ModemService;
use crate::persistence::Store;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedSms {
    pub modem_sms_path: String,
}

#[derive(Debug)]
pub enum SendAttemptOutcome {
    Accepted,
    Rejected(anyhow::Error),
    NotAttempted(anyhow::Error),
    Unknown(anyhow::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModemSmsState {
    Stored,
    Sending,
    Sent,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmsSnapshot {
    pub state: ModemSmsState,
    pub phone_number: String,
    pub body: String,
}

pub trait SmsSender: Send + Sync {
    fn prepare<'a>(
        &'a self,
        modem_path: &'a str,
        tel_number: &'a str,
        sms_text: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<PreparedSms>> + Send + 'a>>;

    fn send_prepared<'a>(
        &'a self,
        modem_sms_path: &'a str,
    ) -> Pin<Box<dyn Future<Output = SendAttemptOutcome> + Send + 'a>>;

    fn sms_state<'a>(
        &'a self,
        modem_sms_path: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<ModemSmsState>> + Send + 'a>>;

    fn sms_snapshot<'a>(
        &'a self,
        _modem_path: Option<&'a str>,
        modem_sms_path: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<SmsSnapshot>> + Send + 'a>> {
        Box::pin(async move {
            Ok(SmsSnapshot {
                state: self.sms_state(modem_sms_path).await?,
                phone_number: String::new(),
                body: String::new(),
            })
        })
    }
}

#[derive(Clone, Default)]
pub struct SystemSmsSender {
    connection: Arc<tokio::sync::Mutex<Option<Arc<Connection>>>>,
}

impl SystemSmsSender {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn connect() -> Result<Self> {
        let connection = Connection::system().await?;
        Ok(Self {
            connection: Arc::new(tokio::sync::Mutex::new(Some(Arc::new(connection)))),
        })
    }

    async fn get_or_connect(&self) -> Result<Arc<Connection>> {
        let mut connection = self.connection.lock().await;
        if let Some(connection) = connection.as_ref() {
            return Ok(connection.clone());
        }
        let new_connection = Arc::new(Connection::system().await?);
        *connection = Some(new_connection.clone());
        Ok(new_connection)
    }

    async fn discard_connection(&self, failed: &Arc<Connection>) {
        let mut connection = self.connection.lock().await;
        if connection
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, failed))
        {
            *connection = None;
        }
    }
}

impl SmsSender for SystemSmsSender {
    fn prepare<'a>(
        &'a self,
        modem_path: &'a str,
        tel_number: &'a str,
        sms_text: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<PreparedSms>> + Send + 'a>> {
        Box::pin(async move {
            let connection = self.get_or_connect().await?;
            let result = create_sms(&connection, modem_path, tel_number, sms_text).await;
            if result.is_err() {
                self.discard_connection(&connection).await;
            }
            result
        })
    }

    fn send_prepared<'a>(
        &'a self,
        modem_sms_path: &'a str,
    ) -> Pin<Box<dyn Future<Output = SendAttemptOutcome> + Send + 'a>> {
        Box::pin(async move {
            let connection = match self.get_or_connect().await {
                Ok(connection) => connection,
                Err(error) => return SendAttemptOutcome::NotAttempted(error),
            };
            let outcome = send_prepared_sms(&connection, modem_sms_path).await;
            if !matches!(outcome, SendAttemptOutcome::Accepted) {
                self.discard_connection(&connection).await;
            }
            outcome
        })
    }

    fn sms_state<'a>(
        &'a self,
        modem_sms_path: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<ModemSmsState>> + Send + 'a>> {
        Box::pin(async move {
            let connection = self.get_or_connect().await?;
            let result = get_sms_state(&connection, modem_sms_path).await;
            if result.is_err() {
                self.discard_connection(&connection).await;
            }
            result
        })
    }

    fn sms_snapshot<'a>(
        &'a self,
        modem_path: Option<&'a str>,
        modem_sms_path: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<SmsSnapshot>> + Send + 'a>> {
        Box::pin(async move {
            let connection = self.get_or_connect().await?;
            let result = get_sms_snapshot(&connection, modem_path, modem_sms_path).await;
            if result.is_err() {
                self.discard_connection(&connection).await;
            }
            result
        })
    }
}

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

const DBUS_METHOD_TIMEOUT: Duration = Duration::from_secs(10);
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
    let connection = Connection::system().await?;

    let match_rule = format!(
        "type='signal',path='{}',interface='{}'",
        actual_path, MM_MESSAGING_INTERFACE
    );
    let add_args = (&match_rule,);
    let call = connection.call_method(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        Some("org.freedesktop.DBus"),
        "AddMatch",
        &add_args,
    );
    tokio::time::timeout(DBUS_METHOD_TIMEOUT, call).await??;

    let owner_rule = format!(
        "type='signal',interface='{}',member='NameOwnerChanged',arg0='{}'",
        DBUS_INTERFACE, MM_DESTINATION
    );
    add_match_rule(&connection, &owner_rule).await?;
    let removed_rule = format!(
        "type='signal',interface='{}',member='InterfacesRemoved'",
        OBJECT_MANAGER_INTERFACE
    );
    add_match_rule(&connection, &removed_rule).await?;

    info!("SMS monitor ready on {}", actual_path);

    let ignored_storage: Vec<StorageType> = config
        .sms
        .ignore_storage
        .iter()
        .map(|s| StorageType::from_config(s))
        .collect();

    let mut stream = zbus::MessageStream::from(connection.clone());
    while let Some(msg) = stream.next().await {
        let msg = msg?;
        let header = msg.header();

        let interface = header
            .interface()
            .map(|s| s.to_string())
            .unwrap_or_default();
        let member = header.member().map(|s| s.to_string()).unwrap_or_default();

        if interface == DBUS_INTERFACE && member == "NameOwnerChanged" {
            if let Ok((name, old_owner, new_owner)) =
                msg.body().deserialize::<(String, String, String)>()
            {
                if modem_owner_changed(&name, &old_owner, &new_owner) {
                    return Err(anyhow::anyhow!("ModemManager owner changed"));
                }
            }
        }

        if interface == OBJECT_MANAGER_INTERFACE && member == "InterfacesRemoved" {
            if let Ok((removed_path, _interfaces)) = msg
                .body()
                .deserialize::<(zbus::zvariant::ObjectPath, Vec<String>)>()
            {
                if removed_path.as_str() == actual_path {
                    return Err(anyhow::anyhow!("monitored modem object removed"));
                }
            }
        }

        if interface == MM_MESSAGING_INTERFACE && member.as_str() == "Added" {
            if let Ok(body) = msg
                .body()
                .deserialize::<(zbus::zvariant::ObjectPath, bool)>()
            {
                let sms_path = body.0.to_string();
                let is_received = body.1;
                if is_received {
                    info!("SmsPath:\n{}", sms_path);
                    let permit = inbound_limit
                        .clone()
                        .acquire_owned()
                        .await
                        .map_err(|_| anyhow::anyhow!("inbound task limiter closed"))?;
                    let task_connection = connection.clone();
                    let task_storage_filters = ignored_storage.clone();
                    let task_handler = on_received.clone();
                    spawn_inbound_task(permit, async move {
                        handle_incoming_sms(
                            &task_connection,
                            &sms_path,
                            &task_storage_filters,
                            task_handler,
                        )
                        .await
                    });
                }
            }
        }
    }
    Err(anyhow::anyhow!("ModemManager signal stream ended"))
}

async fn handle_incoming_sms<F, Fut>(
    connection: &Connection,
    sms_path: &str,
    storage_filters: &[StorageType],
    on_received: F,
) -> Result<()>
where
    F: Fn(ReceivedSms) -> Fut + Send,
    Fut: std::future::Future<Output = Result<()>> + Send,
{
    let mut retries = 0;
    loop {
        let call = connection.call_method(
            Some(MM_DESTINATION),
            sms_path,
            Some(DBUS_PROPERTIES_INTERFACE),
            "GetAll",
            &(MM_SMS_INTERFACE,),
        );
        let reply = tokio::time::timeout(Duration::from_secs(5), call)
            .await
            .map_err(|_| anyhow::anyhow!("dbus getAll timeout"))??;

        let props: HashMap<String, OwnedValue> = reply.body().deserialize()?;
        let telnum = extract_string(&props, "Number");
        let smscontent = extract_string(&props, "Text");
        let smsdate = extract_string(&props, "Timestamp");
        let storage = extract_u32(&props, "Storage");

        if should_ignore_storage(storage, storage_filters) {
            warn!("已过滤不转发");
            return Ok(());
        }

        if !smscontent.is_empty() {
            let received = ReceivedSms {
                phone_number: telnum,
                body: smscontent,
                timestamp: smsdate,
                modem_sms_path: sms_path.to_string(),
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

async fn add_match_rule(connection: &Connection, rule: &str) -> Result<()> {
    let args = (rule,);
    let call = connection.call_method(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        Some("org.freedesktop.DBus"),
        "AddMatch",
        &args,
    );
    tokio::time::timeout(DBUS_METHOD_TIMEOUT, call).await??;
    Ok(())
}

fn modem_owner_changed(name: &str, old_owner: &str, new_owner: &str) -> bool {
    name == MM_DESTINATION && old_owner != new_owner
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

pub async fn create_sms(
    connection: &Connection,
    modem_path: &str,
    tel_number: &str,
    sms_text: &str,
) -> Result<PreparedSms> {
    let mut properties = HashMap::new();
    properties.insert("text", Value::from(sms_text));
    properties.insert("number", Value::from(tel_number));

    let create_args = (&properties,);
    let call = connection.call_method(
        Some(MM_DESTINATION),
        modem_path,
        Some(MM_MESSAGING_INTERFACE),
        "Create",
        &create_args,
    );
    let reply = tokio::time::timeout(Duration::from_secs(15), call)
        .await
        .map_err(|_| anyhow::anyhow!("dbus Create timeout"))??;

    let sms_path: zbus::zvariant::OwnedObjectPath = reply.body().deserialize()?;
    Ok(PreparedSms {
        modem_sms_path: sms_path.to_string(),
    })
}

pub async fn send_prepared_sms(
    connection: &Connection,
    modem_sms_path: &str,
) -> SendAttemptOutcome {
    let send_call = connection.call_method(
        Some(MM_DESTINATION),
        modem_sms_path,
        Some(MM_SMS_INTERFACE),
        "Send",
        &(),
    );
    match tokio::time::timeout(Duration::from_secs(30), send_call).await {
        Err(_) => SendAttemptOutcome::Unknown(anyhow::anyhow!("dbus Send timeout")),
        Ok(Ok(_)) => {
            println!("短信已发送");
            SendAttemptOutcome::Accepted
        }
        Ok(Err(error)) if is_explicit_send_rejection(&error) => {
            SendAttemptOutcome::Rejected(error.into())
        }
        Ok(Err(error)) => SendAttemptOutcome::Unknown(error.into()),
    }
}

fn is_explicit_send_rejection(error: &zbus::Error) -> bool {
    let zbus::Error::MethodError(name, _, _) = error else {
        return false;
    };
    is_explicit_send_rejection_name(name.as_str())
}

fn is_explicit_send_rejection_name(name: &str) -> bool {
    name.starts_with("org.freedesktop.ModemManager1.Error.")
        || matches!(
            name,
            "org.freedesktop.DBus.Error.AccessDenied"
                | "org.freedesktop.DBus.Error.InvalidArgs"
                | "org.freedesktop.DBus.Error.UnknownMethod"
                | "org.freedesktop.DBus.Error.UnknownObject"
                | "org.freedesktop.DBus.Error.UnknownInterface"
        )
}

pub async fn get_sms_state(connection: &Connection, modem_sms_path: &str) -> Result<ModemSmsState> {
    Ok(get_sms_snapshot(connection, None, modem_sms_path)
        .await?
        .state)
}

pub async fn get_sms_snapshot(
    connection: &Connection,
    modem_path: Option<&str>,
    modem_sms_path: &str,
) -> Result<SmsSnapshot> {
    if let Some(modem_path) = modem_path {
        let list_call = connection.call_method(
            Some(MM_DESTINATION),
            modem_path,
            Some(MM_MESSAGING_INTERFACE),
            "List",
            &(),
        );
        let list_reply = tokio::time::timeout(Duration::from_secs(5), list_call)
            .await
            .map_err(|_| anyhow::anyhow!("dbus SMS list timeout"))??;
        let paths: Vec<zbus::zvariant::OwnedObjectPath> = list_reply.body().deserialize()?;
        if !paths.iter().any(|path| path.as_str() == modem_sms_path) {
            return Ok(SmsSnapshot {
                state: ModemSmsState::Unknown,
                phone_number: String::new(),
                body: String::new(),
            });
        }
    }
    let call = connection.call_method(
        Some(MM_DESTINATION),
        modem_sms_path,
        Some(DBUS_PROPERTIES_INTERFACE),
        "GetAll",
        &(MM_SMS_INTERFACE,),
    );
    let reply = tokio::time::timeout(Duration::from_secs(5), call)
        .await
        .map_err(|_| anyhow::anyhow!("dbus SMS state timeout"))?;
    let reply = match reply {
        Ok(reply) => reply,
        Err(error) if is_missing_sms_object(&error) => {
            return Ok(SmsSnapshot {
                state: ModemSmsState::Unknown,
                phone_number: String::new(),
                body: String::new(),
            });
        }
        Err(error) => return Err(error.into()),
    };
    let properties: HashMap<String, OwnedValue> = reply.body().deserialize()?;
    Ok(SmsSnapshot {
        state: sms_state_from_raw(extract_u32(&properties, "State")),
        phone_number: extract_string(&properties, "Number"),
        body: extract_string(&properties, "Text"),
    })
}

fn is_missing_sms_object(error: &zbus::Error) -> bool {
    let zbus::Error::MethodError(name, _, _) = error else {
        return false;
    };
    is_missing_sms_object_error_name(name.as_str())
}

fn is_missing_sms_object_error_name(name: &str) -> bool {
    matches!(
        name,
        "org.freedesktop.DBus.Error.UnknownObject"
            | "org.freedesktop.ModemManager1.Error.Core.NotFound"
    )
}

fn sms_state_from_raw(state: u32) -> ModemSmsState {
    match state {
        1 => ModemSmsState::Stored,
        4 => ModemSmsState::Sending,
        5 => ModemSmsState::Sent,
        _ => ModemSmsState::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::time::Duration as StdDuration;

    use tokio::sync::Notify;

    use super::*;

    #[test]
    fn modem_sms_states_map_to_recovery_states() {
        assert_eq!(sms_state_from_raw(1), ModemSmsState::Stored);
        assert_eq!(sms_state_from_raw(4), ModemSmsState::Sending);
        assert_eq!(sms_state_from_raw(5), ModemSmsState::Sent);
        assert_eq!(sms_state_from_raw(0), ModemSmsState::Unknown);
        assert_eq!(sms_state_from_raw(6), ModemSmsState::Unknown);
    }

    #[test]
    fn missing_sms_object_errors_are_terminal_for_recovery() {
        assert!(is_missing_sms_object_error_name(
            "org.freedesktop.DBus.Error.UnknownObject"
        ));
        assert!(is_missing_sms_object_error_name(
            "org.freedesktop.ModemManager1.Error.Core.NotFound"
        ));
        assert!(!is_missing_sms_object_error_name(
            "org.freedesktop.DBus.Error.NoReply"
        ));
    }

    #[test]
    fn no_reply_is_unknown_but_modem_rejection_is_explicit() {
        assert!(!is_explicit_send_rejection_name(
            "org.freedesktop.DBus.Error.NoReply"
        ));
        assert!(!is_explicit_send_rejection_name(
            "org.freedesktop.DBus.Error.Disconnected"
        ));
        assert!(is_explicit_send_rejection_name(
            "org.freedesktop.ModemManager1.Error.Core.WrongState"
        ));
        assert!(is_explicit_send_rejection_name(
            "org.freedesktop.DBus.Error.AccessDenied"
        ));
    }
    use crate::modem::{MmcliOutput, MmcliRunner, ModemError};

    #[test]
    fn system_sms_sender_defers_connecting_until_a_send_is_requested() {
        let sender = SystemSmsSender::new();
        assert!(sender.connection.try_lock().unwrap().is_none());
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

    #[test]
    fn owner_change_requires_reconnect_only_for_modem_manager() {
        assert!(modem_owner_changed(MM_DESTINATION, ":1.1", ":1.2"));
        assert!(!modem_owner_changed(MM_DESTINATION, ":1.1", ":1.1"));
        assert!(!modem_owner_changed("org.example.Other", ":1.1", ":1.2"));
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
