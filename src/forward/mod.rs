pub mod bark;
pub mod dingtalk;
pub mod pushplus;
pub mod shell;
pub mod telegram;
pub mod wecom;

use reqwest::StatusCode;

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

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;
    use std::time::Duration;

    use super::*;
    use crate::config::AppConfig;
    use crate::runner::ProcessRunner;

    struct TimeoutRunner;

    impl ProcessRunner for TimeoutRunner {
        fn run_command<'a>(
            &'a self,
            _program: &'a str,
            _arguments: &'a [&'a str],
            _timeout: Duration,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<std::process::ExitStatus>> + Send + 'a>>
        {
            Box::pin(async { Err(anyhow::anyhow!("shell timeout")) })
        }
    }

    struct ShellFailedRunner;

    impl ProcessRunner for ShellFailedRunner {
        fn run_command<'a>(
            &'a self,
            _program: &'a str,
            _arguments: &'a [&'a str],
            _timeout: Duration,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<std::process::ExitStatus>> + Send + 'a>>
        {
            Box::pin(async { Err(anyhow::anyhow!("executable not found")) })
        }
    }

    #[tokio::test]
    async fn shell_timeout_produces_shell_timeout_code() {
        let outcome = crate::forward::shell::send(
            &TimeoutRunner,
            Duration::from_secs(1),
            "+1",
            "body",
            "2026-01-01T00:00:00Z",
            "device",
            &crate::config::ShellConfig {
                path: "/bin/sleep".to_string(),
            },
            &AppConfig::default(),
        )
        .await;
        assert!(
            matches!(&outcome, ForwardOutcome::TransientFailure(code) if code == "shell_timeout"),
            "expected shell_timeout, got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn shell_execution_failure_produces_execution_failed_code() {
        let outcome = crate::forward::shell::send(
            &ShellFailedRunner,
            Duration::from_secs(1),
            "+1",
            "body",
            "2026-01-01T00:00:00Z",
            "device",
            &crate::config::ShellConfig {
                path: "/bin/does-not-exist".to_string(),
            },
            &AppConfig::default(),
        )
        .await;
        assert!(
            matches!(&outcome, ForwardOutcome::TransientFailure(code) if code == "shell_execution_failed"),
            "expected shell_execution_failed, got {outcome:?}"
        );
    }

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
