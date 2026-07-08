pub mod bark;
pub mod dingtalk;
pub mod pushplus;
pub mod shell;
pub mod telegram;
pub mod wecom;

use anyhow::Result;
use log::error;

use crate::config::{AppConfig, ChannelProfile};
use crate::util;

pub async fn forward_sms(
    profiles: &[ChannelProfile],
    tel_number: &str,
    sms_text: &str,
    sms_date: &str,
    config: &AppConfig,
) -> Result<()> {
    let device_name = resolve_device_name(config);
    let sms_date = sms_date.replace('T', " ");
    println!(
        "发信电话:{}\n时间:{}\n短信内容:{}",
        tel_number, sms_date, sms_text
    );

    let mut failures = 0usize;
    for profile in profiles {
        let result = match profile {
            ChannelProfile::PushPlus {
                config: profile_config,
                ..
            } => {
                pushplus::send(
                    tel_number,
                    sms_text,
                    &sms_date,
                    &device_name,
                    profile_config,
                    config,
                )
                .await
            }
            ChannelProfile::WeCom {
                config: profile_config,
                ..
            } => {
                wecom::send(
                    tel_number,
                    sms_text,
                    &sms_date,
                    &device_name,
                    profile_config,
                    config,
                )
                .await
            }
            ChannelProfile::Telegram {
                config: profile_config,
                ..
            } => {
                telegram::send(
                    tel_number,
                    sms_text,
                    &sms_date,
                    &device_name,
                    profile_config,
                    config,
                )
                .await
            }
            ChannelProfile::DingTalk {
                config: profile_config,
                ..
            } => {
                dingtalk::send(
                    tel_number,
                    sms_text,
                    &sms_date,
                    &device_name,
                    profile_config,
                    config,
                )
                .await
            }
            ChannelProfile::Bark {
                config: profile_config,
                ..
            } => {
                bark::send(
                    tel_number,
                    sms_text,
                    &sms_date,
                    &device_name,
                    profile_config,
                    config,
                )
                .await
            }
            ChannelProfile::Shell {
                config: profile_config,
                ..
            } => {
                shell::send(
                    tel_number,
                    sms_text,
                    &sms_date,
                    &device_name,
                    profile_config,
                    config,
                )
                .await
            }
        };
        if let Err(e) = result {
            failures += 1;
            error!("profile forward failed: {}", e);
        }
    }

    if failures == profiles.len() && !profiles.is_empty() {
        error!("all forwarding profiles failed for this SMS");
    }
    Ok(())
}

fn resolve_device_name(config: &AppConfig) -> String {
    let name = config.app.device_name.as_str();
    if name == "*Host*Name*" || name.is_empty() {
        util::hostname()
    } else {
        name.to_string()
    }
}
