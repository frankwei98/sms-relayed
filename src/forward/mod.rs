pub mod bark;
pub mod dingtalk;
pub mod pushplus;
pub mod shell;
pub mod telegram;
pub mod wecom;

use anyhow::Result;
use log::error;

use crate::cli::Channel;
use crate::config::Config;
use crate::util;

pub async fn forward_sms(
    channels: &[Channel],
    tel_number: &str,
    sms_text: &str,
    sms_date: &str,
    config: &Config,
) -> Result<()> {
    let device_name = resolve_device_name(config);
    let sms_date = sms_date.replace('T', " ");
    let body = format!(
        "发信电话:{}\n时间:{}\n短信内容:{}",
        tel_number, sms_date, sms_text
    );
    println!("{}", body);

    for channel in channels {
        let result = match channel {
            Channel::PushPlus => {
                pushplus::send(tel_number, sms_text, &sms_date, &device_name, config).await
            }
            Channel::WeCom => {
                wecom::send(tel_number, sms_text, &sms_date, &device_name, config).await
            }
            Channel::Telegram => {
                telegram::send(tel_number, sms_text, &sms_date, &device_name, config).await
            }
            Channel::DingTalk => {
                dingtalk::send(tel_number, sms_text, &sms_date, &device_name, config).await
            }
            Channel::Bark => {
                bark::send(tel_number, sms_text, &sms_date, &device_name, config).await
            }
            Channel::Shell => {
                shell::send(tel_number, sms_text, &sms_date, &device_name, config).await
            }
        };
        if let Err(e) = result {
            error!("{}转发失败: {}", channel.name(), e);
        }
    }
    Ok(())
}

fn resolve_device_name(config: &Config) -> String {
    let name = config.get_or_empty("ForwardDeviceName");
    if name == "*Host*Name*" || name.is_empty() {
        util::hostname()
    } else {
        name.to_string()
    }
}
