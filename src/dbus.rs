use std::collections::HashMap;
use std::io::{self, Write};

use anyhow::Result;
use futures_util::StreamExt;
use log::{error, info, warn};
use zbus::zvariant::{OwnedValue, Value};
use zbus::Connection;

use crate::config::{AppConfig, ChannelProfile};
use crate::forward;

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

pub async fn monitor_dbus(
    modem_path: &str,
    profiles: &[ChannelProfile],
    config: &AppConfig,
) -> Result<()> {
    monitor_dbus_with_handler(modem_path, profiles, config, |_| async { Ok(()) }).await
}

pub async fn monitor_dbus_with_handler<F, Fut>(
    modem_path: &str,
    profiles: &[ChannelProfile],
    config: &AppConfig,
    mut on_received: F,
) -> Result<()>
where
    F: FnMut(ReceivedSms) -> Fut + Send,
    Fut: std::future::Future<Output = Result<()>> + Send,
{
    println!("短信转发模式正在启动，正在连接系统 D-Bus。");
    info!("正在运行. 按下 Ctrl-C 停止.");
    let connection = Connection::system().await?;

    let match_rule = format!(
        "type='signal',path='{}',interface='{}'",
        modem_path, MM_MESSAGING_INTERFACE
    );
    connection
        .call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"),
            "AddMatch",
            &(&match_rule,),
        )
        .await?;

    println!("短信转发监听已就绪。按 Ctrl-C 停止。");

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

        if interface == MM_MESSAGING_INTERFACE && member.as_str() == "Added" {
            if let Ok(body) = msg
                .body()
                .deserialize::<(zbus::zvariant::ObjectPath, bool)>()
            {
                let sms_path = body.0.to_string();
                let is_received = body.1;
                if is_received {
                    info!("SmsPath:\n{}", sms_path);
                    if let Err(e) = get_sms_content(
                        &connection,
                        &sms_path,
                        &ignored_storage,
                        profiles,
                        config,
                        &mut on_received,
                    )
                    .await
                    {
                        error!("处理短信失败: {}", e);
                    }
                }
            }
        }
    }
    Ok(())
}

fn should_ignore_storage(storage: u32, filters: &[StorageType]) -> bool {
    filters
        .iter()
        .any(|filter| !matches!(filter, StorageType::All) && filter.should_ignore(storage))
}

async fn get_sms_content<F, Fut>(
    connection: &Connection,
    sms_path: &str,
    storage_filters: &[StorageType],
    profiles: &[ChannelProfile],
    config: &AppConfig,
    on_received: &mut F,
) -> Result<()>
where
    F: FnMut(ReceivedSms) -> Fut + Send,
    Fut: std::future::Future<Output = Result<()>> + Send,
{
    let mut retries = 0;
    loop {
        let reply = connection
            .call_method(
                Some(MM_DESTINATION),
                sms_path,
                Some(DBUS_PROPERTIES_INTERFACE),
                "GetAll",
                &(MM_SMS_INTERFACE,),
            )
            .await?;

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
            forward::forward_sms(profiles, &phone_number, &body, &timestamp, config).await?;
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

    let reply = connection
        .call_method(
            Some(MM_DESTINATION),
            modem_path,
            Some(MM_MESSAGING_INTERFACE),
            "Create",
            &(&properties,),
        )
        .await?;

    let sms_path: zbus::zvariant::OwnedObjectPath = reply.body().deserialize()?;
    let sms_path_str = sms_path.as_str();

    if target == SendTarget::Command {
        println!("短信创建成功，是否发送？(1.发送短信,其他按键退出程序)");
        io::stdout().flush().unwrap();
        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();
        if input.trim() != "1" {
            let _ = connection
                .call_method(
                    Some(MM_DESTINATION),
                    modem_path,
                    Some(MM_MESSAGING_INTERFACE),
                    "Delete",
                    &(&sms_path,),
                )
                .await;
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

    let _reply = connection
        .call_method(
            Some(MM_DESTINATION),
            sms_path_str,
            Some("org.freedesktop.ModemManager1.Sms"),
            "Send",
            &(),
        )
        .await?;
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
