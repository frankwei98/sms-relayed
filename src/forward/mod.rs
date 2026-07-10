pub mod bark;
pub mod dingtalk;
pub mod pushplus;
pub mod shell;
pub mod telegram;
pub mod wecom;

use std::time::Duration;

use log::error;
use reqwest::StatusCode;

use crate::config::{AppConfig, ChannelProfile};
use crate::runner::ProcessRunner;
use crate::util;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForwardOutcome {
    Success,
    TransientFailure(String),
    PermanentFailure(String),
}

pub(crate) fn transport_failure(error: &reqwest::Error) -> ForwardOutcome {
    let code = if error.is_timeout() {
        "http_timeout"
    } else if error.is_connect() {
        "http_connect"
    } else if error.is_decode() {
        "http_decode"
    } else if error.is_body() {
        "http_body"
    } else if error.is_redirect() {
        "http_redirect"
    } else if error.is_request() {
        "http_request"
    } else {
        "http_transport"
    };
    ForwardOutcome::TransientFailure(code.to_string())
}

pub(crate) fn classify_http_status(status: StatusCode) -> Option<ForwardOutcome> {
    if status.is_success() {
        return None;
    }

    let code = format!("http_status_{}", status.as_u16());
    if status == StatusCode::REQUEST_TIMEOUT
        || status == StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
        || !status.is_client_error()
    {
        Some(ForwardOutcome::TransientFailure(code))
    } else {
        Some(ForwardOutcome::PermanentFailure(code))
    }
}

pub(crate) fn classify_provider_rejection(
    code: Option<i64>,
    permanent_codes: &[i64],
) -> ForwardOutcome {
    if code.is_some_and(|value| permanent_codes.contains(&value)) {
        ForwardOutcome::PermanentFailure("provider_invalid_credentials_or_parameters".to_string())
    } else {
        ForwardOutcome::TransientFailure("provider_rejected".to_string())
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_status_classification_matches_retry_contract() {
        let cases = [
            (200, None),
            (204, None),
            (400, Some(false)),
            (401, Some(false)),
            (404, Some(false)),
            (408, Some(true)),
            (429, Some(true)),
            (500, Some(true)),
            (503, Some(true)),
            (302, Some(true)),
        ];

        for (status, expected_transient) in cases {
            let status = StatusCode::from_u16(status).unwrap();
            let outcome = classify_http_status(status);
            match (outcome, expected_transient) {
                (None, None) => {}
                (Some(ForwardOutcome::TransientFailure(code)), Some(true)) => {
                    assert_eq!(code, format!("http_status_{}", status.as_u16()));
                }
                (Some(ForwardOutcome::PermanentFailure(code)), Some(false)) => {
                    assert_eq!(code, format!("http_status_{}", status.as_u16()));
                }
                (actual, expected) => panic!(
                    "unexpected classification for {status}: {actual:?}, expected {expected:?}"
                ),
            }
        }
    }

    #[test]
    fn unknown_provider_rejections_retry_by_default() {
        let cases = [
            (None, true),
            (Some(999_999), true),
            (Some(401), false),
            (Some(400), false),
        ];

        for (code, expected_transient) in cases {
            let outcome = classify_provider_rejection(code, &[400, 401]);
            assert_eq!(
                matches!(outcome, ForwardOutcome::TransientFailure(_)),
                expected_transient,
                "unexpected provider classification for {code:?}: {outcome:?}"
            );
        }
    }

    #[tokio::test]
    async fn transport_errors_return_fixed_safe_codes() {
        let secret = "secret-token-and-phone-15551234567";
        let error = reqwest::Client::new()
            .get(format!("http://[{secret}"))
            .send()
            .await
            .unwrap_err();

        let ForwardOutcome::TransientFailure(code) = transport_failure(&error) else {
            panic!("transport failures must be retryable");
        };
        assert!(code.starts_with("http_"));
        assert!(!code.contains(secret));
        assert!(!code.contains("15551234567"));
        assert!(!code.contains("token"));
    }
}
