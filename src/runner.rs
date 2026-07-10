use std::future::Future;
use std::pin::Pin;
use std::process::ExitStatus;
use std::time::Duration;

use anyhow::Result;
use time::OffsetDateTime;

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
#[allow(dead_code)]
pub trait ProcessRunner: Send + Sync {
    fn run_shell<'a>(
        &'a self,
        cmd: &'a str,
        timeout: Duration,
    ) -> Pin<Box<dyn Future<Output = Result<ExitStatus>> + Send + 'a>>;
}

/// Real process runner that shells out to `sh -c`.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
pub struct RealProcessRunner;

impl ProcessRunner for RealProcessRunner {
    fn run_shell<'a>(
        &'a self,
        cmd: &'a str,
        _timeout: Duration,
    ) -> Pin<Box<dyn Future<Output = Result<ExitStatus>> + Send + 'a>> {
        let cmd = cmd.to_string();
        Box::pin(async move {
            let status = tokio::process::Command::new("sh")
                .arg("-c")
                .arg(&cmd)
                .status()
                .await?;
            Ok(status)
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
}
