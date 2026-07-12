use std::collections::HashMap;
use std::io::{self, Write};
use std::time::Duration;

use anyhow::Result;
use futures_util::StreamExt;
use log::{error, info, warn};
use zbus::zvariant::{OwnedValue, Value};
use zbus::Connection;

use crate::config::AppConfig;
use crate::modem::ModemService;
use crate::storage::MessageStore;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendTarget {
    Command,
    #[allow(dead_code)]
    Api,
    #[allow(dead_code)]
    Cli,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendSmsOutcome {
    pub modem_sms_path: String,
}

#[derive(Debug, Clone)]
pub struct ReceivedSms {
    pub phone_number: String,
    pub body: String,
    pub timestamp: String,
    pub storage: u32,
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

pub const FINGERPRINT_META_KEY: &str = "modem_fingerprint";

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
async fn resolve_monitor_path(
    configured_path: &str,
    modem_service: &ModemService,
    store: &MessageStore,
) -> Option<String> {
    // Try configured path first
    let stored_fp = store.get_meta(FINGERPRINT_META_KEY);
    let identity = modem_service.extract_identity(configured_path).await;
    if let Some(identity) = identity {
        let current_fp = ModemService::compute_fingerprint(&identity);
        match stored_fp.as_deref() {
            Some(enrolled_fp) if enrolled_fp == current_fp => {
                return Some(configured_path.to_string());
            }
            Some(enrolled_fp) => {
                warn!("configured modem identity changed; refusing path reuse");
                return modem_service.scan_and_match_fingerprint(enrolled_fp).await;
            }
            None => {
                if let Err(e) = store.set_meta(FINGERPRINT_META_KEY, &current_fp) {
                    error!("failed to enroll modem identity: {}", e);
                    return None;
                }
                // Backfill dedupe keys for legacy modem-inbound messages now
                // that the fingerprint is available for stable hashing.
                if let Err(e) = store.backfill_dedupe_keys() {
                    error!("failed to backfill dedupe keys: {}", e);
                    return None;
                }
                return Some(configured_path.to_string());
            }
        }
    }

    // Configured path failed; try fingerprint match
    modem_service
        .scan_and_match_fingerprint(stored_fp.as_deref()?)
        .await
}

async fn run_subscription<F, Fut>(
    actual_path: &str,
    config: &AppConfig,
    on_received: &mut F,
) -> Result<()>
where
    F: FnMut(crate::dbus::ReceivedSms) -> Fut + Send,
    Fut: std::future::Future<Output = Result<()>> + Send,
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
                    handle_incoming_sms(&connection, &sms_path, &ignored_storage, on_received)
                        .await?;
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
    on_received: &mut F,
) -> Result<()>
where
    F: FnMut(crate::dbus::ReceivedSms) -> Fut + Send,
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
                storage,
                modem_sms_path: sms_path.to_string(),
            };
            let mut delay = Duration::from_millis(100);
            loop {
                match on_received(received.clone()).await {
                    Ok(()) => break,
                    Err(e) => {
                        error!("persist incoming SMS failed; retrying: {}", e);
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
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
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
    store: &MessageStore,
) -> Result<()>
where
    F: FnMut(ReceivedSms) -> Fut + Send + Clone + 'static,
    Fut: std::future::Future<Output = Result<()>> + Send + 'static,
{
    println!("短信转发模式正在启动，正在连接系统 D-Bus。");
    info!("正在运行. 按下 Ctrl-C 停止.");

    let mut current_path = resolve_monitor_path(configured_modem_path, modem_service, store).await;
    if current_path.is_none() {
        warn!("initial modem resolution failed; will retry with backoff");
        current_path = Some(configured_modem_path.to_string());
    }

    let mut delay = Duration::from_secs(5);
    let max_delay = Duration::from_secs(60);

    loop {
        let path = current_path
            .clone()
            .unwrap_or_else(|| configured_modem_path.to_string());
        let mut cb = on_received.clone();

        match run_subscription(&path, config, &mut cb).await {
            Ok(()) => {
                delay = Duration::from_secs(5);
            }
            Err(e) => {
                error!("D-Bus monitor lost: {}", e);
                let resolved =
                    resolve_monitor_path(configured_modem_path, modem_service, store).await;
                if let Some(new_path) = resolved {
                    if new_path != path {
                        info!("modem path changed from {} to {}", path, new_path);
                    }
                    current_path = Some(new_path);
                } else {
                    warn!("modem re-resolution failed; will retry");
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

pub async fn send_sms(
    connection: &Connection,
    modem_path: &str,
    tel_number: &str,
    sms_text: &str,
    target: SendTarget,
) -> Result<SendSmsOutcome> {
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
    let sms_path_str = sms_path.as_str();

    if target == SendTarget::Command {
        println!("短信创建成功，是否发送？(1.发送短信,其他按键退出程序)");
        io::stdout().flush().unwrap();
        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();
        if input.trim() != "1" {
            let del_args = (&sms_path,);
            let del_call = connection.call_method(
                Some(MM_DESTINATION),
                modem_path,
                Some(MM_MESSAGING_INTERFACE),
                "Delete",
                &del_args,
            );
            let _ = tokio::time::timeout(Duration::from_secs(5), del_call).await;
            println!("短信缓存已清理，按任意键退出程序");
            let mut temp = String::new();
            io::stdin().read_line(&mut temp).unwrap();
            return Ok(SendSmsOutcome {
                modem_sms_path: sms_path_str.to_string(),
            });
        }
    } else if target == SendTarget::Cli {
        // Confirmation already handled by runtime::send_interactive; skip prompt.
    }

    let send_call = connection.call_method(
        Some(MM_DESTINATION),
        sms_path_str,
        Some("org.freedesktop.ModemManager1.Sms"),
        "Send",
        &(),
    );
    let _reply = tokio::time::timeout(Duration::from_secs(30), send_call)
        .await
        .map_err(|_| anyhow::anyhow!("dbus Send timeout"))??;

    println!("短信已发送");
    Ok(SendSmsOutcome {
        modem_sms_path: sms_path_str.to_string(),
    })
}

pub async fn send_sms_via_system(
    modem_path: &str,
    tel_number: &str,
    sms_text: &str,
) -> Result<SendSmsOutcome> {
    let connection = Connection::system().await?;
    send_sms(
        &connection,
        modem_path,
        tel_number,
        sms_text,
        SendTarget::Api,
    )
    .await
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;

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

    #[test]
    fn owner_change_requires_reconnect_only_for_modem_manager() {
        assert!(modem_owner_changed(MM_DESTINATION, ":1.1", ":1.2"));
        assert!(!modem_owner_changed(MM_DESTINATION, ":1.1", ":1.1"));
        assert!(!modem_owner_changed("org.example.Other", ":1.1", ":1.2"));
    }

    #[tokio::test]
    async fn enrolled_fingerprint_rejects_unrelated_modem_at_configured_path() {
        let store = MessageStore::open_in_memory().unwrap();
        let target = ModemService::compute_fingerprint("target");
        store.set_meta(FINGERPRINT_META_KEY, &target).unwrap();
        let service = ModemService::new_with_runner(IdentityRunner);

        let resolved =
            resolve_monitor_path("/org/freedesktop/ModemManager1/Modem/0", &service, &store).await;

        assert_eq!(
            resolved.as_deref(),
            Some("/org/freedesktop/ModemManager1/Modem/1")
        );
        assert_eq!(
            store.get_meta(FINGERPRINT_META_KEY).as_deref(),
            Some(target.as_str())
        );
    }
}
