pub mod bark;
pub mod dingtalk;
pub mod pushplus;
pub mod shell;
pub mod telegram;
pub mod wecom;

use std::time::Duration;

use log::error;

use crate::config::{AppConfig, ChannelProfile};
use crate::runner::ProcessRunner;
use crate::util;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForwardOutcome {
    Success,
    TransientFailure(String),
    PermanentFailure(String),
}

#[allow(dead_code)]
pub async fn forward_sms(
    client: &reqwest::Client,
    shell_runner: &dyn ProcessRunner,
    shell_timeout: Duration,
    profiles: &[ChannelProfile],
    tel_number: &str,
    sms_text: &str,
    sms_date: &str,
    config: &AppConfig,
) -> Vec<(String, ForwardOutcome)> {
    let device_name = resolve_device_name(config);
    let sms_date = sms_date.replace('T', " ");
    println!(
        "发信电话:{}\n时间:{}\n短信内容:{}",
        tel_number, sms_date, sms_text
    );

    let mut results: Vec<(String, ForwardOutcome)> = Vec::new();
    for profile in profiles {
        let key = profile.key();
        let outcome = match profile {
            ChannelProfile::PushPlus {
                config: profile_config,
                ..
            } => {
                pushplus::send(
                    client,
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
                    client,
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
                    client,
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
                    client,
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
                    client,
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
                    shell_runner,
                    shell_timeout,
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
        results.push((key, outcome));
    }

    let failures: Vec<_> = results
        .iter()
        .filter(|(_, o)| !matches!(o, ForwardOutcome::Success))
        .map(|(k, _)| k.clone())
        .collect();
    if failures.len() == profiles.len() && !profiles.is_empty() {
        error!("all forwarding profiles failed for this SMS");
    }
    for (key, outcome) in &results {
        if matches!(outcome, ForwardOutcome::Success) {
            log::info!("{} forward success", key);
        } else {
            error!("{} forward failed: {:?}", key, outcome);
        }
    }

    results
}

#[allow(dead_code)]
fn resolve_device_name(config: &AppConfig) -> String {
    let name = config.app.device_name.as_str();
    if name == "*Host*Name*" || name.is_empty() {
        util::hostname()
    } else {
        name.to_string()
    }
}
