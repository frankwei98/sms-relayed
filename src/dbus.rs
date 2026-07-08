use std::collections::HashMap;
use std::io::{self, Write};

use anyhow::Result;
use futures_util::StreamExt;
use log::{error, info, warn};
use zbus::zvariant::{OwnedValue, Value};
use zbus::Connection;

use crate::cli::Channel;
use crate::config::Config;
use crate::forward;

#[derive(Debug, Clone, Copy)]
pub enum StorageType {
    Unknown = 0,
    Sm = 1,
    Me = 2,
    Mt = 3,
    Sr = 4,
    Bm = 5,
    Ta = 6,
    All = 100,
}

impl StorageType {
    fn from_config(s: &str) -> Self {
        match s {
            "unknown" => StorageType::Unknown,
            "sm" => StorageType::Sm,
            "me" => StorageType::Me,
            "mt" => StorageType::Mt,
            "sr" => StorageType::Sr,
            "bm" => StorageType::Bm,
            "ta" => StorageType::Ta,
            _ => StorageType::All,
        }
    }

    fn should_ignore(&self, storage: u32) -> bool {
        match self {
            StorageType::All => false,
            _ => *self as u32 == storage,
        }
    }
}

const MODEM_PATH: &str = "/org/freedesktop/ModemManager1/Modem/0";
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

pub async fn monitor_dbus(channels: &[Channel], config: &Config) -> Result<()> {
    println!("短信转发模式正在启动，正在连接系统 D-Bus。");
    info!("正在运行. 按下 Ctrl-C 停止.");
    let connection = Connection::system().await?;

    // Add match rule for SMS Added signals
    let match_rule = format!(
        "type='signal',path='{}',interface='{}'",
        MODEM_PATH, MM_MESSAGING_INTERFACE
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
    let storage_filter = StorageType::from_config(config.get_or_empty("forwardIgnoreStorageType"));
    let mut stream = zbus::MessageStream::from(connection.clone());

    while let Some(msg) = stream.next().await {
        let msg = msg?;
        let header = msg.header();

        let interface = header.interface().map(|s| s.to_string()).unwrap_or_default();
        let member = header.member().map(|s| s.to_string()).unwrap_or_default();

        if interface == MM_MESSAGING_INTERFACE && member.as_str() == "Added" {
            // Extract args: object_path (arg0), is_received (arg1 bool)
            if let Ok(body) = msg.body().deserialize::<(zbus::zvariant::ObjectPath, bool)>() {
                let sms_path = body.0.to_string();
                let is_received = body.1;
                if is_received {
                    info!("SmsPath:\n{}", sms_path);
                    if let Err(e) =
                        get_sms_content(&connection, &sms_path, storage_filter, channels, config)
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

async fn get_sms_content(
    connection: &Connection,
    sms_path: &str,
    storage_filter: StorageType,
    channels: &[Channel],
    config: &Config,
) -> Result<()> {
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

        if storage_filter.should_ignore(storage) {
            warn!("已过滤不转发");
            return Ok(());
        }

        if !smscontent.is_empty() {
            forward::forward_sms(channels, &telnum, &smscontent, &smsdate, config).await?;
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
    tel_number: &str,
    sms_text: &str,
    target: &str,
) -> Result<()> {
    // Create SMS via ModemManager Messaging.Create
    let mut properties = HashMap::new();
    properties.insert("text", Value::from(sms_text));
    properties.insert("number", Value::from(tel_number));

    let reply = connection
        .call_method(
            Some(MM_DESTINATION),
            MODEM_PATH,
            Some(MM_MESSAGING_INTERFACE),
            "Create",
            &(&properties,),
        )
        .await?;

    let sms_path: zbus::zvariant::OwnedObjectPath = reply.body().deserialize()?;
    let sms_path_str = sms_path.as_str();

    if target == "command" {
        print!("短信创建成功，是否发送？(1.发送短信,其他按键退出程序)\n");
        io::stdout().flush().unwrap();
        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();
        if input.trim() != "1" {
            // Delete the draft SMS
            let _ = connection
                .call_method(
                    Some(MM_DESTINATION),
                    MODEM_PATH,
                    Some(MM_MESSAGING_INTERFACE),
                    "Delete",
                    &(&sms_path,),
                )
                .await;
            println!("短信缓存已清理，按任意键退出程序");
            let mut temp = String::new();
            io::stdin().read_line(&mut temp).unwrap();
            return Ok(());
        }
    }

    // Send SMS
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
    Ok(())
}
