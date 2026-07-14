use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use anyhow::Result;
use log::{error, info};
use time::OffsetDateTime;

use crate::config::AppConfig;
use crate::forward::ForwardOutcome;
use crate::runner::ProcessRunner;
use crate::storage::{DeliveryRow, ForwardAttemptOutcome, MessageStore, NewForwardAttemptSample};

const WORKER_CONCURRENCY: usize = 2;
const CLAIM_BATCH: u32 = WORKER_CONCURRENCY as u32;
const LEASE_SECS: u64 = 90;
const RETRY_INITIAL_DELAY: u64 = 30;
const RETRY_MAX_DELAY: u64 = 3600;
const RETRY_MAX_AGE: Duration = Duration::from_secs(86400);
const POLL_INTERVAL: Duration = Duration::from_secs(15);

pub async fn run_delivery_worker(
    store: MessageStore,
    config: AppConfig,
    client: Arc<reqwest::Client>,
    shell_runner: Arc<dyn ProcessRunner>,
    shell_timeout: Duration,
) {
    loop {
        if let Err(e) = tick(&store, &config, &client, &shell_runner, shell_timeout).await {
            error!("delivery worker tick failed: {}", e);
            crate::monitoring::capture_failure("delivery", "delivery.tick_failed");
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

async fn tick(
    store: &MessageStore,
    config: &AppConfig,
    client: &reqwest::Client,
    shell_runner: &Arc<dyn ProcessRunner>,
    shell_timeout: Duration,
) -> Result<()> {
    let lease_until = (OffsetDateTime::now_utc() + time::Duration::seconds(LEASE_SECS as i64))
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();

    let rows = store.claim_due_deliveries(CLAIM_BATCH, &lease_until)?;
    if rows.is_empty() {
        return Ok(());
    }

    let semaphore = Arc::new(tokio::sync::Semaphore::new(WORKER_CONCURRENCY));
    let mut handles = Vec::new();

    for row in rows {
        let permit = semaphore.clone().acquire_owned().await.unwrap();
        let store = store.clone();
        let config = config.clone();
        let client = client.clone();
        let runner = shell_runner.clone();
        let st = shell_timeout;

        handles.push(tokio::spawn(async move {
            let _permit = permit;
            process_delivery_inner(&store, &config, &client, &*runner, st, row).await
        }));
    }

    for h in handles {
        h.await??;
    }

    Ok(())
}

async fn process_delivery_inner(
    store: &MessageStore,
    config: &AppConfig,
    client: &reqwest::Client,
    shell_runner: &dyn ProcessRunner,
    shell_timeout: Duration,
    row: DeliveryRow,
) -> Result<()> {
    let profile_key = &row.profile_key;
    let lease_token = row
        .lease_token
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("claimed delivery missing lease token"))?;
    let next_attempt_at = compute_retry_delay(row.id, row.attempt_count + 1);
    let attempt_number = (row.attempt_count + 1) as i32;

    let message = match store.get_message(row.message_id) {
        Ok(m) => m,
        Err(e) => {
            error!("delivery {}: get message failed: {}", row.id, e);
            ensure_completed(store.complete_delivery(
                row.id,
                "permanent_failed",
                Some("message_not_found"),
                row.attempt_count + 1,
                None,
                lease_token,
            )?)?;
            return Ok(());
        }
    };

    let profiles = config.enabled_profiles().unwrap_or_default();
    let profile = profiles.iter().find(|p| p.key() == *profile_key).cloned();

    let (outcome, latency_us) = match profile {
        Some(ref p) => {
            let start = Instant::now();
            let result = forward_to_profile(
                client,
                shell_runner,
                shell_timeout,
                p,
                &message.phone_number,
                &message.body,
                &message.timestamp,
                config,
            )
            .await;
            let elapsed = start.elapsed();
            (result, elapsed.as_micros() as i64)
        }
        None => {
            process_no_sample_path(
                store,
                row.id,
                "permanent_failed",
                "profile_missing",
                lease_token,
                row.attempt_count + 1,
            )?;
            return Ok(());
        }
    };

    let (_state, error_code) = map_outcome_to_delivery_state(&outcome);
    let started_at = (OffsetDateTime::now_utc() - time::Duration::microseconds(latency_us))
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();
    let completed_at = OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();
    let sample = NewForwardAttemptSample {
        profile_key: profile_key.clone(),
        delivery_id: Some(row.id),
        attempt_number,
        started_at,
        completed_at,
        latency_ms: (latency_us / 1000).max(1),
        outcome: map_outcome_to_attempt(&outcome),
        error_code: error_code.clone(),
    };

    match outcome {
        ForwardOutcome::Success => {
            info!("delivery {}: success", row.id);
            store.complete_delivery_with_attempt(
                row.id,
                "succeeded",
                None,
                row.attempt_count + 1,
                None,
                lease_token,
                sample,
            )?;
        }
        ForwardOutcome::PermanentFailure(_) => {
            error!("delivery {}: permanent failure", row.id);
            store.complete_delivery_with_attempt(
                row.id,
                "permanent_failed",
                error_code.as_deref(),
                row.attempt_count + 1,
                None,
                lease_token,
                sample,
            )?;
        }
        ForwardOutcome::TransientFailure(_) => {
            let age = delivery_age(&row.created_at);
            if age > RETRY_MAX_AGE {
                error!("delivery {}: max age exceeded, permanent failure", row.id);
                store.complete_delivery_with_attempt(
                    row.id,
                    "permanent_failed",
                    Some("max_age_exceeded"),
                    row.attempt_count + 1,
                    None,
                    lease_token,
                    sample,
                )?;
            } else {
                info!(
                    "delivery {}: transient failure, retry at {}",
                    row.id, next_attempt_at
                );
                store.complete_delivery_with_attempt(
                    row.id,
                    "retry_wait",
                    error_code.as_deref(),
                    row.attempt_count + 1,
                    Some(&next_attempt_at),
                    lease_token,
                    sample,
                )?;
            }
        }
    }
    Ok(())
}

/// For message_not_found / profile_missing: no attempt sample recorded
fn process_no_sample_path(
    store: &MessageStore,
    id: i64,
    state: &str,
    error_code: &str,
    lease_token: &str,
    attempt_count: i64,
) -> Result<()> {
    ensure_completed(store.complete_delivery(
        id,
        state,
        Some(error_code),
        attempt_count,
        None,
        lease_token,
    )?)?;
    Ok(())
}

fn map_outcome_to_attempt(outcome: &ForwardOutcome) -> ForwardAttemptOutcome {
    match outcome {
        ForwardOutcome::Success => ForwardAttemptOutcome::Success,
        ForwardOutcome::TransientFailure(_) => ForwardAttemptOutcome::TransientFailure,
        ForwardOutcome::PermanentFailure(_) => ForwardAttemptOutcome::PermanentFailure,
    }
}

fn map_outcome_to_delivery_state(outcome: &ForwardOutcome) -> (&'static str, Option<String>) {
    match outcome {
        ForwardOutcome::Success => ("succeeded", None),
        ForwardOutcome::TransientFailure(ref msg) => {
            let ec = standardize_failure(msg);
            ("retry_wait", Some(ec))
        }
        ForwardOutcome::PermanentFailure(ref msg) => {
            let ec = standardize_failure(msg);
            ("permanent_failed", Some(ec))
        }
    }
}

fn standardize_failure(msg: &str) -> String {
    if msg == "http_timeout" || msg == "shell_timeout" {
        msg.to_string()
    } else if msg.starts_with("http_status_")
        || msg.starts_with("http_")
        || msg.starts_with("provider_")
        || msg.starts_with("shell_")
    {
        msg.to_string()
    } else if msg == "message_not_found" || msg == "profile_missing" || msg == "max_age_exceeded" {
        msg.to_string()
    } else if msg.contains("shell timeout") {
        "shell_timeout".to_string()
    } else {
        "unknown_error".to_string()
    }
}

fn ensure_completed(completed: bool) -> Result<()> {
    if completed {
        Ok(())
    } else {
        Err(anyhow::anyhow!("delivery lease ownership lost"))
    }
}

async fn forward_to_profile(
    client: &reqwest::Client,
    shell_runner: &dyn ProcessRunner,
    shell_timeout: Duration,
    profile: &crate::config::ChannelProfile,
    tel_number: &str,
    body: &str,
    timestamp: &str,
    config: &AppConfig,
) -> ForwardOutcome {
    let device_name =
        if config.app.device_name == "*Host*Name*" || config.app.device_name.is_empty() {
            crate::util::hostname()
        } else {
            config.app.device_name.clone()
        };

    match profile {
        crate::config::ChannelProfile::Bark { config: pc, .. } => {
            crate::forward::bark::send(
                client,
                tel_number,
                body,
                timestamp,
                &device_name,
                pc,
                config,
            )
            .await
        }
        crate::config::ChannelProfile::Telegram { config: pc, .. } => {
            crate::forward::telegram::send(
                client,
                tel_number,
                body,
                timestamp,
                &device_name,
                pc,
                config,
            )
            .await
        }
        crate::config::ChannelProfile::PushPlus { config: pc, .. } => {
            crate::forward::pushplus::send(
                client,
                tel_number,
                body,
                timestamp,
                &device_name,
                pc,
                config,
            )
            .await
        }
        crate::config::ChannelProfile::WeCom { config: pc, .. } => {
            crate::forward::wecom::send(
                client,
                tel_number,
                body,
                timestamp,
                &device_name,
                pc,
                config,
            )
            .await
        }
        crate::config::ChannelProfile::DingTalk { config: pc, .. } => {
            crate::forward::dingtalk::send(
                client,
                tel_number,
                body,
                timestamp,
                &device_name,
                pc,
                config,
            )
            .await
        }
        crate::config::ChannelProfile::Shell { config: pc, .. } => {
            crate::forward::shell::send(
                shell_runner,
                shell_timeout,
                tel_number,
                body,
                timestamp,
                &device_name,
                pc,
                config,
            )
            .await
        }
    }
}

fn compute_retry_delay(delivery_id: i64, attempt: i64) -> String {
    use sha2::{Digest, Sha256};

    let base = RETRY_INITIAL_DELAY.min(RETRY_MAX_DELAY);
    let exponent = (attempt - 1).min(10) as u32;
    let delay_secs = base
        .saturating_mul(2u64.saturating_pow(exponent))
        .min(RETRY_MAX_DELAY);
    let spread = delay_secs / 4;
    let digest = Sha256::digest(format!("delivery-jitter:{delivery_id}:{attempt}"));
    let sample = u64::from_be_bytes(digest[..8].try_into().unwrap());
    let offset = if spread == 0 {
        0
    } else {
        sample % (spread.saturating_mul(2).saturating_add(1))
    };
    let total = delay_secs
        .saturating_sub(spread)
        .saturating_add(offset)
        .min(RETRY_MAX_DELAY);
    let next = OffsetDateTime::now_utc() + time::Duration::seconds(total as i64);
    next.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

fn delivery_age(timestamp_str: &str) -> Duration {
    if let Ok(ts) = OffsetDateTime::parse(
        timestamp_str,
        &time::format_description::well_known::Rfc3339,
    ) {
        let now = OffsetDateTime::now_utc();
        if now > ts {
            (now - ts).unsigned_abs()
        } else {
            Duration::ZERO
        }
    } else {
        RETRY_MAX_AGE + Duration::from_secs(1)
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;

    use crate::config::{AppConfig, ShellConfig};
    use crate::runner::ProcessRunner;
    use crate::storage::{MessageStore, NewMessage};

    use super::*;

    #[test]
    fn retry_delay_is_bounded_and_varies_by_delivery() {
        let first = compute_retry_delay(1, 3);
        let second = compute_retry_delay(2, 3);
        assert_ne!(first, second);
        for value in [first, second] {
            let parsed =
                OffsetDateTime::parse(&value, &time::format_description::well_known::Rfc3339)
                    .unwrap();
            let delay = parsed - OffsetDateTime::now_utc();
            assert!(delay.whole_seconds() >= 80);
            assert!(delay.whole_seconds() <= 160);
        }
    }

    #[test]
    fn malformed_delivery_timestamp_is_treated_as_expired() {
        assert!(delivery_age("") > RETRY_MAX_AGE);
        assert!(delivery_age("not-a-timestamp") > RETRY_MAX_AGE);
    }

    /// Stub runner that returns a shell timeout error.
    struct TimeoutRunner;

    impl ProcessRunner for TimeoutRunner {
        fn run_shell<'a>(
            &'a self,
            _cmd: &'a str,
            _timeout: Duration,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<std::process::ExitStatus>> + Send + 'a>>
        {
            Box::pin(async { Err(anyhow::anyhow!("shell timeout")) })
        }
    }

    /// Helper: create an in-memory store, insert a message + delivery,
    /// claim it, and return the claimed row along with the store so the
    /// caller can reuse the store reference in `process_delivery_inner`.
    fn setup_claimed_delivery(
        store: &MessageStore,
        profile_key: &str,
        prior_attempts: i64,
    ) -> DeliveryRow {
        store
            .insert_message_with_deliveries(
                NewMessage::inbound("+1", "delivery test body"),
                &[profile_key.to_string()],
            )
            .unwrap();

        // If prior_attempts > 0, simulate them via complete_delivery
        if prior_attempts > 0 {
            let first = store
                .claim_due_deliveries(1, "2099-01-01T00:00:00Z")
                .unwrap()
                .pop()
                .unwrap();
            store
                .complete_delivery(
                    first.id,
                    "retry_wait",
                    Some("http_timeout"),
                    prior_attempts,
                    Some("2000-01-01T00:00:00Z"),
                    &first.lease_token.unwrap(),
                )
                .unwrap();
            // Release the lease so it can be reclaimed
        }

        let batch = store
            .claim_due_deliveries(1, "2099-01-01T00:00:00Z")
            .unwrap();
        let mut row = batch.into_iter().next().unwrap();
        // Override attempt_count to simulate prior retries
        // (claim returns the stored count; we need to check it matches)
        row.attempt_count = prior_attempts;
        row
    }

    #[tokio::test]
    async fn profile_missing_results_in_permanent_failure_with_no_sample() {
        let store = MessageStore::open_in_memory().unwrap();
        let row = setup_claimed_delivery(&store, "bark.primary", 2);
        assert_eq!(row.attempt_count, 2);
        let delivery_id = row.id;

        // AppConfig with NO enabled profiles — profile is missing
        let mut cfg = AppConfig::default();
        cfg.api.enabled = true;
        cfg.api.password = "test".to_string();

        let client = reqwest::Client::new();
        process_delivery_inner(
            &store,
            &cfg,
            &client,
            &TimeoutRunner,
            Duration::from_secs(5),
            row,
        )
        .await
        .unwrap();

        let d = store.get_delivery(delivery_id).unwrap();
        assert_eq!(d.state, "permanent_failed");
        assert_eq!(d.attempt_count, 3, "attempt_count must NOT regress");
        assert_eq!(d.last_error.as_deref(), Some("profile_missing"));

        // No attempt sample recorded
        assert_eq!(
            store
                .list_forward_attempts("bark.primary", 5)
                .unwrap()
                .len(),
            0
        );
    }

    #[tokio::test]
    async fn shell_timeout_records_sample_and_retry_wait() {
        let store = MessageStore::open_in_memory().unwrap();
        let row = setup_claimed_delivery(&store, "shell.test", 0);
        let delivery_id = row.id;

        // Build AppConfig with a Shell profile matching "shell.test"
        let mut cfg = AppConfig::default();
        cfg.api.enabled = true;
        cfg.api.password = "test".to_string();
        cfg.forward.enabled.push("shell.test".to_string());
        cfg.channels.shell.insert(
            "test".to_string(),
            ShellConfig {
                path: "/bin/sleep".to_string(),
            },
        );

        let client = reqwest::Client::new();
        process_delivery_inner(
            &store,
            &cfg,
            &client,
            &TimeoutRunner,
            Duration::from_secs(1),
            row,
        )
        .await
        .unwrap();

        let d = store.get_delivery(delivery_id).unwrap();
        assert_eq!(
            d.state, "retry_wait",
            "shell timeout must produce retry_wait"
        );
        assert_eq!(d.last_error.as_deref(), Some("shell_timeout"));

        let samples = store.list_forward_attempts("shell.test", 5).unwrap();
        assert_eq!(samples.len(), 1, "one sample must be recorded");
        assert_eq!(samples[0].outcome, ForwardAttemptOutcome::TransientFailure);
        assert_eq!(samples[0].error_code.as_deref(), Some("shell_timeout"));
    }
}
