use std::future::Future;
use std::pin::Pin;
use std::process::ExitStatus;
use std::time::Duration;

use anyhow::Result;
use time::OffsetDateTime;

use crate::config::HttpSection;

/// Build a shared `reqwest::Client` configured from the `[http]` config section.
pub fn build_http_client(config: &HttpSection) -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(config.connect_timeout_secs))
        .timeout(Duration::from_secs(config.request_timeout_secs))
        .build()
        .expect("valid reqwest client config")
}

/// Clock abstraction for deterministic time in tests.
#[allow(dead_code)]
pub trait Clock: Send + Sync {
    fn now(&self) -> OffsetDateTime;
}

/// Real clock using system time.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
pub struct RealClock;

impl Clock for RealClock {
    fn now(&self) -> OffsetDateTime {
        OffsetDateTime::now_utc()
    }
}

/// Process execution abstraction for testable shell commands.
pub trait ProcessRunner: Send + Sync {
    fn run_shell<'a>(
        &'a self,
        cmd: &'a str,
        timeout: Duration,
    ) -> Pin<Box<dyn Future<Output = Result<ExitStatus>> + Send + 'a>>;
}

/// Real process runner that shells out to `sh -c`, with timeout and reap.
#[derive(Clone, Copy, Debug)]
pub struct RealProcessRunner;

impl ProcessRunner for RealProcessRunner {
    fn run_shell<'a>(
        &'a self,
        cmd: &'a str,
        timeout: Duration,
    ) -> Pin<Box<dyn Future<Output = Result<ExitStatus>> + Send + 'a>> {
        let cmd = cmd.to_string();
        Box::pin(async move {
            let mut child = tokio::process::Command::new("sh")
                .arg("-c")
                .arg(&cmd)
                .kill_on_drop(true)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::piped())
                .spawn()?;
            match tokio::time::timeout(timeout, child.wait()).await {
                Ok(result) => Ok(result?),
                Err(_) => {
                    let _ = child.kill().await;
                    child.wait().await?;
                    Err(anyhow::anyhow!("shell timeout"))
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_clock_returns_recent_time() {
        let clock = RealClock;
        let now = clock.now();
        let system_now = OffsetDateTime::now_utc();
        let diff = system_now - now;
        assert!(diff.abs().whole_seconds() < 2);
    }

    #[tokio::test]
    async fn real_process_runner_executes_shell_true() {
        let runner = RealProcessRunner;
        let status = runner
            .run_shell("exit 0", Duration::from_secs(5))
            .await
            .unwrap();
        assert!(status.success());
    }

    #[tokio::test]
    async fn real_process_runner_executes_shell_false() {
        let runner = RealProcessRunner;
        let status = runner
            .run_shell("exit 42", Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(status.code(), Some(42));
    }

    #[tokio::test]
    async fn real_process_runner_timeout_kills_child() {
        let runner = RealProcessRunner;
        let err = runner
            .run_shell("sleep 10", Duration::from_millis(50))
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("shell timeout"),
            "expected timeout error: {}",
            msg
        );
    }

    #[tokio::test]
    async fn build_http_client_uses_configured_timeouts() {
        let config = HttpSection::default();
        let _client = build_http_client(&config);
    }
}
