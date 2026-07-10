use std::collections::HashMap;
use std::io::{self, Write};
use std::time::Duration;

use anyhow::Result;
use futures_util::StreamExt;
use log::{error, info, warn};
use zbus::zvariant::{OwnedValue, Value};
use zbus::Connection;

use crate::config::{AppConfig, ChannelProfile};
use crate::forward;
use crate::modem::ModemService;
use crate::runner::ProcessRunner;
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
const MM_DESTINATION: &str = "org.freedesktop.ModemManager1";

const DBUS_METHOD_TIMEOUT: Duration = Duration::from_secs(10);

const FINGERPRINT_META_KEY: &str = "modem_fingerprint";

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
    let identity = modem_service.extract_identity(configured_path).await;
    if identity.is_some() {
        let fp = ModemService::compute_fingerprint(&identity.unwrap());
        store.set_meta(FINGERPRINT_META_KEY, &fp).ok();
        return Some(configured_path.to_string());
    }

    // Configured path failed; try fingerprint match
    let stored_fp = store.get_meta(FINGERPRINT_META_KEY)?;
    let matched = modem_service.scan_and_match_fingerprint(&stored_fp).await;
    matched
}

async fn run_subscription<F, Fut>(
    actual_path: &str,
    config: &AppConfig,
    on_received: &mut F,
    client: &reqwest::Client,
    shell_runner: &dyn ProcessRunner,
    shell_timeout: Duration,
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

    info!("SMS monitor ready on {}", actual_path);

    let ignored_storage: Vec<StorageType> = config
        .sms
        .ignore_storage
        .iter()
        .map(|s| StorageType::from_config(s))
        .collect();

    let profiles = config.enabled_profiles().unwrap_or_default();

    let mut stream = zbus::MessageStream::from(connection.clone());

    while let Some(msg) = stream.next().await {
        let msg = msg?;
        let header = msg.header();

        let interface = header
            .interface()
            .map(|s| s.to_string())
            .unwrap_or_default();
        let member = header.member().map(|s| s.to_string()).unwrap_or_default();

        if interface == MM_MESSAGING_INTERFACE && member.as_str() == "Added" {
            if let Ok(body) = msg
                .body()
                .deserialize::<(zbus::zvariant::ObjectPath, bool)>()
            {
                let sms_path = body.0.to_string();
                let is_received = body.1;
                if is_received {
                    info!("SmsPath:\n{}", sms_path);
                    if let Err(e) = handle_incoming_sms(
                        &connection,
                        &sms_path,
                        &ignored_storage,
                        &profiles,
                        config,
                        on_received,
                        client,
                        shell_runner,
                        shell_timeout,
                    )
                    .await
                    {
                        error!("handle incoming SMS failed: {}", e);
                    }
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
    profiles: &[ChannelProfile],
    config: &AppConfig,
    on_received: &mut F,
    client: &reqwest::Client,
    shell_runner: &dyn ProcessRunner,
    shell_timeout: Duration,
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
            let phone_number = received.phone_number.clone();
            let body = received.body.clone();
            let timestamp = received.timestamp.clone();
            if let Err(e) = on_received(received).await {
                error!("处理短信失败: {}", e);
            }
            forward::forward_sms(
                client,
                shell_runner,
                shell_timeout,
                profiles,
                &phone_number,
                &body,
                &timestamp,
                config,
            )
            .await?;
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

pub async fn monitor_dbus_with_handler<F, Fut>(
    configured_modem_path: &str,
    config: &AppConfig,
    on_received: F,
    client: &reqwest::Client,
    shell_runner: &dyn ProcessRunner,
    shell_timeout: Duration,
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

        match run_subscription(&path, config, &mut cb, client, shell_runner, shell_timeout).await {
            Ok(()) => {
                delay = Duration::from_secs(5);
            }
            Err(e) => {
                error!("D-Bus monitor lost: {}", e);
                // Re-resolve the modem path
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
