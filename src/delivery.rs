use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use anyhow::Result;
use log::{error, info};
use time::OffsetDateTime;
use tokio::sync::Notify;

use crate::config::AppConfig;
use crate::forward::ForwardOutcome;
use crate::persistence::{
    ClaimedDelivery, CompleteDelivery, CompletionResult, DeliveryAttempt, DeliveryAttemptOutcome,
    DeliveryClaim, DeliveryDisposition, DeliveryTime, Store,
};
use crate::runner::ProcessRunner;

const LEASE_SECS: u64 = 90;
const RETRY_INITIAL_DELAY: u64 = 30;
const RETRY_MAX_DELAY: u64 = 3600;
const RETRY_MAX_AGE: Duration = Duration::from_secs(86400);
const SAFETY_SCAN_INTERVAL: Duration = Duration::from_secs(30);
const WORKER_ERROR_INITIAL_DELAY: Duration = Duration::from_secs(1);
const WORKER_ERROR_MAX_DELAY: Duration = Duration::from_secs(30);
const CLAIMED_WAVES_PER_BATCH: usize = 4;

struct DeliveryTaskContext<'a> {
    store: &'a Store,
    config: &'a AppConfig,
    client: &'a reqwest::Client,
    shell_runner: &'a Arc<dyn ProcessRunner>,
    shell_timeout: Duration,
}

#[derive(Clone, Default)]
pub struct DeliveryWakeup {
    notify: Arc<Notify>,
}

impl DeliveryWakeup {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn notify(&self) {
        self.notify.notify_one();
    }

    pub(crate) async fn wait(&self) {
        self.notify.notified().await;
    }
}

pub async fn run_delivery_worker(
    store: Store,
    config: AppConfig,
    client: Arc<reqwest::Client>,
    shell_runner: Arc<dyn ProcessRunner>,
    shell_timeout: Duration,
    wakeup: DeliveryWakeup,
) {
    let mut error_delay = WORKER_ERROR_INITIAL_DELAY;
    loop {
        match drain_due_deliveries(&store, &config, &client, &shell_runner, shell_timeout).await {
            Ok(processed) => {
                if processed > 0 {
                    log::debug!("delivery queue drained; processed={processed}");
                }
            }
            Err(e) => {
                backoff_after_worker_error("queue drain", &e, &mut error_delay).await;
                continue;
            }
        }

        let retry_delay = match next_delivery_delay(&store).await {
            Ok(delay) => delay,
            Err(e) => {
                backoff_after_worker_error("deadline scheduling", &e, &mut error_delay).await;
                continue;
            }
        };
        error_delay = WORKER_ERROR_INITIAL_DELAY;

        let reason = tokio::select! {
            _ = wakeup.wait() => "new_delivery",
            _ = wait_for_retry_deadline(retry_delay) => "retry_deadline",
            _ = tokio::time::sleep(SAFETY_SCAN_INTERVAL) => "safety_scan",
        };
        log::debug!("delivery worker woke; reason={reason}");
    }
}

async fn backoff_after_worker_error(operation: &str, error: &anyhow::Error, delay: &mut Duration) {
    log::error!("delivery worker {operation} failed: {error}");
    crate::monitoring::capture_failure("delivery", "delivery.worker_failed");
    log::debug!(
        "delivery worker backing off; operation={operation} delay_secs={}",
        delay.as_secs()
    );
    tokio::time::sleep(*delay).await;
    *delay = (*delay * 2).min(WORKER_ERROR_MAX_DELAY);
}

async fn drain_due_deliveries(
    store: &Store,
    config: &AppConfig,
    client: &reqwest::Client,
    shell_runner: &Arc<dyn ProcessRunner>,
    shell_timeout: Duration,
) -> Result<usize> {
    let mut processed = 0;
    loop {
        let count =
            process_delivery_batch(store, config, client, shell_runner, shell_timeout).await?;
        if count == 0 {
            return Ok(processed);
        }
        processed += count;
        tokio::task::yield_now().await;
    }
}

async fn process_delivery_batch(
    store: &Store,
    config: &AppConfig,
    client: &reqwest::Client,
    shell_runner: &Arc<dyn ProcessRunner>,
    shell_timeout: Duration,
) -> Result<usize> {
    let concurrency = config.delivery.concurrency;
    let mut rows: VecDeque<_> = claim_delivery_batch(store, config).await?.into();
    if rows.is_empty() {
        return Ok(0);
    }

    let mut claimed_count = rows.len();
    let mut tasks = tokio::task::JoinSet::new();
    let task_context = DeliveryTaskContext {
        store,
        config,
        client,
        shell_runner,
        shell_timeout,
    };
    fill_delivery_slots(&mut tasks, &mut rows, concurrency, &task_context);

    let mut first_error = None;
    while let Some(result) = tasks.join_next().await {
        let result = match result {
            Ok(result) => result,
            Err(error) => Err(error.into()),
        };
        if let Err(error) = result {
            first_error.get_or_insert(error);
        }

        if rows.is_empty() && first_error.is_none() {
            match claim_delivery_batch(store, config).await {
                Ok(claimed) => {
                    claimed_count += claimed.len();
                    rows.extend(claimed);
                }
                Err(error) => {
                    first_error = Some(error);
                }
            }
        }

        fill_delivery_slots(&mut tasks, &mut rows, concurrency, &task_context);
    }
    if let Some(error) = first_error {
        return Err(error);
    }

    Ok(claimed_count)
}

async fn claim_delivery_batch(store: &Store, config: &AppConfig) -> Result<Vec<ClaimedDelivery>> {
    let concurrency = config.delivery.concurrency;
    let batch_size = concurrency.saturating_mul(CLAIMED_WAVES_PER_BATCH) as u32;
    let channel_timeout = config
        .http
        .request_timeout_secs
        .max(config.http.shell_timeout_secs);
    let lease_secs =
        LEASE_SECS.saturating_add(channel_timeout.saturating_mul(CLAIMED_WAVES_PER_BATCH as u64));
    let rows = store
        .claim_deliveries(batch_size, Duration::from_secs(lease_secs))
        .await?;
    let count = rows.len();
    if count > 0 {
        log::debug!("delivery worker claimed batch; count={count}");
    }
    Ok(rows)
}

fn fill_delivery_slots(
    tasks: &mut tokio::task::JoinSet<Result<()>>,
    rows: &mut VecDeque<ClaimedDelivery>,
    concurrency: usize,
    context: &DeliveryTaskContext<'_>,
) {
    while tasks.len() < concurrency {
        let Some(row) = rows.pop_front() else {
            break;
        };
        spawn_delivery_task(tasks, context, row);
    }
}

fn spawn_delivery_task(
    tasks: &mut tokio::task::JoinSet<Result<()>>,
    context: &DeliveryTaskContext<'_>,
    row: ClaimedDelivery,
) {
    let store = context.store.clone();
    let config = context.config.clone();
    let client = context.client.clone();
    let runner = context.shell_runner.clone();
    let shell_timeout = context.shell_timeout;
    tasks.spawn(async move {
        process_delivery_inner(&store, &config, &client, &*runner, shell_timeout, row).await
    });
}

async fn next_delivery_delay(store: &Store) -> Result<Option<Duration>> {
    let Some(value) = store.next_delivery_due().await? else {
        return Ok(None);
    };
    Ok(Some(time_until(value, OffsetDateTime::now_utc())))
}

async fn wait_for_retry_deadline(delay: Option<Duration>) {
    match delay {
        Some(delay) => tokio::time::sleep(delay).await,
        None => std::future::pending::<()>().await,
    }
}

async fn complete_delivery_with_attempt(store: &Store, completion: CompleteDelivery) -> Result<()> {
    if store.complete_delivery(completion).await? == CompletionResult::OwnershipLost {
        log::warn!("delivery completion ignored after lease ownership was lost");
    }
    Ok(())
}

async fn process_delivery_inner(
    store: &Store,
    config: &AppConfig,
    client: &reqwest::Client,
    shell_runner: &dyn ProcessRunner,
    shell_timeout: Duration,
    row: ClaimedDelivery,
) -> Result<()> {
    let profile_key = &row.profile_key;
    let retry_after = compute_retry_delay(row.id, row.attempt_count + 1);
    let retry_at = OffsetDateTime::now_utc() + time::Duration::try_from(retry_after)?;

    if delivery_age(row.created_at) > RETRY_MAX_AGE {
        error!("delivery {}: max age exceeded, permanent failure", row.id);
        process_no_sample_path(store, row.claim.clone(), "max_age_exceeded").await?;
        return Ok(());
    }

    let message = match store.message_for_delivery(row.message_id).await? {
        Some(message) => message,
        None => {
            error!("delivery {}: message not found", row.id);
            process_no_sample_path(store, row.claim.clone(), "message_not_found").await?;
            return Ok(());
        }
    };

    let profiles = config.enabled_profiles().unwrap_or_default();
    let profile = profiles.iter().find(|p| p.key() == *profile_key).cloned();
    let attempt_started_at = OffsetDateTime::now_utc();
    let dispatch_delay_ms = dispatch_delay_ms(&row, attempt_started_at);

    let (outcome, latency_us) = match profile {
        Some(ref p) => {
            let start = Instant::now();
            let result = forward_to_profile(
                client,
                shell_runner,
                shell_timeout,
                ForwardRequest {
                    profile: p,
                    phone_number: &message.phone_number,
                    body: &message.body,
                    timestamp: &message.timestamp,
                },
                config,
            )
            .await;
            let elapsed = start.elapsed();
            (result, elapsed.as_micros() as i64)
        }
        None => {
            process_no_sample_path(store, row.claim.clone(), "profile_missing").await?;
            return Ok(());
        }
    };

    let error_code = map_outcome_to_delivery_state(&outcome);
    let sample = DeliveryAttempt {
        started_at: attempt_started_at,
        completed_at: OffsetDateTime::now_utc(),
        latency: Duration::from_micros(latency_us.max(1) as u64),
        dispatch_delay: Duration::from_millis(dispatch_delay_ms.max(0) as u64),
        outcome: map_outcome_to_attempt(&outcome),
        error_code: error_code.clone(),
    };

    match outcome {
        ForwardOutcome::Success => {
            info!("delivery {}: success", row.id);
            complete_delivery_with_attempt(
                store,
                CompleteDelivery {
                    claim: row.claim.clone(),
                    disposition: DeliveryDisposition::Succeeded,
                    attempt: Some(sample),
                },
            )
            .await?;
        }
        ForwardOutcome::PermanentFailure(_) => {
            error!("delivery {}: permanent failure", row.id);
            complete_delivery_with_attempt(
                store,
                CompleteDelivery {
                    claim: row.claim.clone(),
                    disposition: DeliveryDisposition::PermanentFailure {
                        error_code: error_code
                            .clone()
                            .unwrap_or_else(|| "unknown_error".to_string()),
                    },
                    attempt: Some(sample),
                },
            )
            .await?;
        }
        ForwardOutcome::TransientFailure(_) => {
            let age = delivery_age(row.created_at);
            if age > RETRY_MAX_AGE {
                error!("delivery {}: max age exceeded, permanent failure", row.id);
                complete_delivery_with_attempt(
                    store,
                    CompleteDelivery {
                        claim: row.claim.clone(),
                        disposition: DeliveryDisposition::PermanentFailure {
                            error_code: "max_age_exceeded".to_string(),
                        },
                        attempt: Some(sample),
                    },
                )
                .await?;
            } else {
                info!(
                    "delivery {}: transient failure, retry in {}s",
                    row.id,
                    retry_after.as_secs()
                );
                complete_delivery_with_attempt(
                    store,
                    CompleteDelivery {
                        claim: row.claim.clone(),
                        disposition: DeliveryDisposition::RetryAt {
                            error_code: error_code
                                .clone()
                                .unwrap_or_else(|| "unknown_error".to_string()),
                            at: retry_at,
                        },
                        attempt: Some(sample),
                    },
                )
                .await?;
            }
        }
    }
    Ok(())
}

fn dispatch_delay_ms(row: &ClaimedDelivery, started_at: OffsetDateTime) -> i64 {
    let due_at = if row.attempt_count == 0 {
        row.created_at
    } else {
        row.next_attempt_at.unwrap_or(row.created_at)
    };
    duration_millis(elapsed_since(due_at, started_at))
}

fn elapsed_since(value: DeliveryTime, now: OffsetDateTime) -> Duration {
    match value {
        DeliveryTime::Valid(timestamp) if now > timestamp => (now - timestamp).unsigned_abs(),
        DeliveryTime::Valid(_) => Duration::ZERO,
        DeliveryTime::Invalid => RETRY_MAX_AGE + Duration::from_secs(1),
    }
}

fn time_until(timestamp: OffsetDateTime, now: OffsetDateTime) -> Duration {
    if timestamp > now {
        (timestamp - now).unsigned_abs()
    } else {
        Duration::ZERO
    }
}

fn duration_millis(duration: Duration) -> i64 {
    duration.as_millis().min(i64::MAX as u128) as i64
}

/// For message_not_found / profile_missing: no attempt sample recorded
async fn process_no_sample_path(
    store: &Store,
    claim: DeliveryClaim,
    error_code: &'static str,
) -> Result<()> {
    if store
        .complete_delivery(CompleteDelivery {
            claim,
            disposition: DeliveryDisposition::PermanentFailure {
                error_code: error_code.to_string(),
            },
            attempt: None,
        })
        .await?
        == CompletionResult::OwnershipLost
    {
        log::warn!("delivery completion ignored after lease ownership was lost");
    }
    Ok(())
}

fn map_outcome_to_attempt(outcome: &ForwardOutcome) -> DeliveryAttemptOutcome {
    match outcome {
        ForwardOutcome::Success => DeliveryAttemptOutcome::Success,
        ForwardOutcome::TransientFailure(_) => DeliveryAttemptOutcome::TransientFailure,
        ForwardOutcome::PermanentFailure(_) => DeliveryAttemptOutcome::PermanentFailure,
    }
}

fn map_outcome_to_delivery_state(outcome: &ForwardOutcome) -> Option<String> {
    match outcome {
        ForwardOutcome::Success => None,
        ForwardOutcome::TransientFailure(ref msg) | ForwardOutcome::PermanentFailure(ref msg) => {
            Some(standardize_failure(msg))
        }
    }
}

fn standardize_failure(msg: &str) -> String {
    if msg == "http_timeout"
        || msg == "shell_timeout"
        || msg.starts_with("http_status_")
        || msg.starts_with("http_")
        || msg.starts_with("provider_")
        || msg.starts_with("shell_")
        || msg == "message_not_found"
        || msg == "profile_missing"
        || msg == "max_age_exceeded"
    {
        msg.to_string()
    } else if msg.contains("shell timeout") {
        "shell_timeout".to_string()
    } else {
        "unknown_error".to_string()
    }
}

struct ForwardRequest<'a> {
    profile: &'a crate::config::ChannelProfile,
    phone_number: &'a str,
    body: &'a str,
    timestamp: &'a str,
}

async fn forward_to_profile(
    client: &reqwest::Client,
    shell_runner: &dyn ProcessRunner,
    shell_timeout: Duration,
    request: ForwardRequest<'_>,
    config: &AppConfig,
) -> ForwardOutcome {
    let ForwardRequest {
        profile,
        phone_number: tel_number,
        body,
        timestamp,
    } = request;
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
                crate::forward::shell::ShellMessage {
                    tel_number,
                    sms_text: body,
                    sms_date: timestamp,
                    device_name: &device_name,
                },
                pc,
                config,
            )
            .await
        }
    }
}

fn compute_retry_delay(delivery_id: i64, attempt: i64) -> Duration {
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
    Duration::from_secs(total)
}

fn delivery_age(timestamp: DeliveryTime) -> Duration {
    elapsed_since(timestamp, OffsetDateTime::now_utc())
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[cfg(unix)]
    use std::os::unix::process::ExitStatusExt;
    use tokio::sync::Notify;

    use crate::config::{AppConfig, ShellConfig};
    use crate::persistence::{InboundMessage, Store};
    use crate::runner::ProcessRunner;

    use super::*;

    #[test]
    fn retry_delay_is_bounded_and_varies_by_delivery() {
        let first = compute_retry_delay(1, 3);
        let second = compute_retry_delay(2, 3);
        assert_ne!(first, second);
        for value in [first, second] {
            assert!(value.as_secs() >= 80);
            assert!(value.as_secs() <= 160);
        }
    }

    #[test]
    fn malformed_delivery_timestamp_is_treated_as_expired() {
        assert!(delivery_age(DeliveryTime::Invalid) > RETRY_MAX_AGE);
    }

    /// Stub runner that returns a shell timeout error.
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

    struct RecordingRunner {
        calls: AtomicUsize,
        completions: AtomicUsize,
        active: AtomicUsize,
        max_active: AtomicUsize,
        called: Notify,
        delays: Vec<Duration>,
    }

    impl RecordingRunner {
        fn new() -> Self {
            Self::with_delays(Vec::new())
        }

        fn with_delays(delays: Vec<Duration>) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                completions: AtomicUsize::new(0),
                active: AtomicUsize::new(0),
                max_active: AtomicUsize::new(0),
                called: Notify::new(),
                delays,
            }
        }

        async fn wait_for_calls(&self, expected: usize) {
            while self.calls.load(Ordering::SeqCst) < expected {
                self.called.notified().await;
            }
        }

        async fn wait_for_completions(&self, expected: usize) {
            while self.completions.load(Ordering::SeqCst) < expected {
                self.called.notified().await;
            }
        }
    }

    impl ProcessRunner for RecordingRunner {
        fn run_command<'a>(
            &'a self,
            _program: &'a str,
            _arguments: &'a [&'a str],
            _timeout: Duration,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<std::process::ExitStatus>> + Send + 'a>>
        {
            Box::pin(async move {
                let call = self.calls.fetch_add(1, Ordering::SeqCst);
                let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
                self.max_active.fetch_max(active, Ordering::SeqCst);
                self.called.notify_waiters();
                if let Some(delay) = self.delays.get(call) {
                    tokio::time::sleep(*delay).await;
                }
                self.active.fetch_sub(1, Ordering::SeqCst);
                self.completions.fetch_add(1, Ordering::SeqCst);
                self.called.notify_waiters();
                #[cfg(unix)]
                {
                    Ok(std::process::ExitStatus::from_raw(0))
                }
                #[cfg(not(unix))]
                {
                    unreachable!("delivery tests run on Unix")
                }
            })
        }
    }

    fn shell_config(profile_count: usize, concurrency: usize) -> AppConfig {
        let mut cfg = AppConfig::default();
        cfg.delivery.concurrency = concurrency;
        for index in 0..profile_count {
            let name = format!("test{index}");
            cfg.forward.enabled.push(format!("shell.{name}"));
            cfg.channels.shell.insert(
                name,
                ShellConfig {
                    path: "/bin/true".to_string(),
                },
            );
        }
        cfg
    }

    async fn memory_store() -> Store {
        let store = Store::open_in_memory().unwrap();
        store
            .set_modem_fingerprint("delivery-test-modem".to_string())
            .await
            .unwrap();
        store
    }

    async fn insert_delivery(store: &Store, body: &str, profile_keys: Vec<String>) {
        store
            .receive_inbound(
                InboundMessage {
                    phone_number: "+1".to_string(),
                    body: body.to_string(),
                    timestamp: OffsetDateTime::now_utc()
                        .format(&time::format_description::well_known::Rfc3339)
                        .unwrap(),
                    modem_sms_path: format!(
                        "/org/freedesktop/ModemManager1/SMS/{}",
                        uuid::Uuid::new_v4()
                    ),
                },
                profile_keys,
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn committed_delivery_notification_wakes_idle_worker_immediately() {
        let store = memory_store().await;
        let config = shell_config(1, 1);
        let runner = Arc::new(RecordingRunner::new());
        let wakeup = DeliveryWakeup::new();
        let worker = tokio::spawn(run_delivery_worker(
            store.clone(),
            config,
            Arc::new(reqwest::Client::new()),
            runner.clone(),
            Duration::from_secs(1),
            wakeup.clone(),
        ));
        tokio::task::yield_now().await;
        insert_delivery(&store, "wake now", vec!["shell.test0".to_string()]).await;

        wakeup.notify();

        tokio::time::timeout(Duration::from_secs(1), runner.wait_for_calls(1))
            .await
            .expect("notified delivery should not wait for the safety scan");
        worker.abort();
    }

    #[tokio::test]
    async fn worker_drains_more_than_one_batch_at_configured_concurrency() {
        let store = memory_store().await;
        let config = shell_config(5, 2);
        let profile_keys = config.forward.enabled.clone();
        insert_delivery(&store, "drain queue", profile_keys).await;
        let runner = Arc::new(RecordingRunner::with_delays(vec![
            Duration::from_millis(20);
            5
        ]));
        let worker = tokio::spawn(run_delivery_worker(
            store,
            config,
            Arc::new(reqwest::Client::new()),
            runner.clone(),
            Duration::from_secs(1),
            DeliveryWakeup::new(),
        ));

        tokio::time::timeout(Duration::from_secs(1), runner.wait_for_completions(5))
            .await
            .expect("worker should drain every due delivery");
        assert_eq!(runner.max_active.load(Ordering::SeqCst), 2);
        worker.abort();
    }

    #[tokio::test]
    async fn worker_replenishes_a_free_slot_without_waiting_for_slow_peer() {
        let store = memory_store().await;
        let config = shell_config(10, 2);
        insert_delivery(&store, "rolling queue", config.forward.enabled.clone()).await;
        let mut delays = vec![Duration::from_millis(5); 10];
        delays[0] = Duration::from_millis(500);
        let runner = Arc::new(RecordingRunner::with_delays(delays));
        let worker = tokio::spawn(run_delivery_worker(
            store,
            config,
            Arc::new(reqwest::Client::new()),
            runner.clone(),
            Duration::from_secs(1),
            DeliveryWakeup::new(),
        ));

        tokio::time::timeout(Duration::from_millis(200), runner.wait_for_calls(10))
            .await
            .expect("a free slot should claim beyond the buffered batch");
        assert_eq!(runner.max_active.load(Ordering::SeqCst), 2);
        worker.abort();
    }

    #[tokio::test]
    async fn worker_refills_concurrency_after_an_initial_single_delivery() {
        let store = memory_store().await;
        let config = shell_config(1, 2);
        insert_delivery(&store, "initial delivery", config.forward.enabled.clone()).await;
        let runner = Arc::new(RecordingRunner::with_delays(vec![
            Duration::from_millis(100);
            6
        ]));
        let worker = tokio::spawn(run_delivery_worker(
            store.clone(),
            config.clone(),
            Arc::new(reqwest::Client::new()),
            runner.clone(),
            Duration::from_secs(1),
            DeliveryWakeup::new(),
        ));
        tokio::time::timeout(Duration::from_secs(1), runner.wait_for_calls(1))
            .await
            .unwrap();

        for index in 0..5 {
            insert_delivery(
                &store,
                &format!("burst {index}"),
                config.forward.enabled.clone(),
            )
            .await;
        }

        tokio::time::timeout(Duration::from_secs(1), runner.wait_for_calls(3))
            .await
            .expect("burst deliveries should start promptly");
        assert_eq!(runner.max_active.load(Ordering::SeqCst), 2);
        worker.abort();
    }

    #[tokio::test]
    async fn retry_deadline_wakes_worker_without_a_new_delivery_notification() {
        let store = memory_store().await;
        let config = shell_config(1, 1);
        insert_delivery(&store, "retry deadline", vec!["shell.test0".to_string()]).await;
        let claimed = store
            .claim_deliveries(1, Duration::from_secs(90))
            .await
            .unwrap()
            .pop()
            .unwrap();
        store
            .complete_delivery(CompleteDelivery {
                claim: claimed.claim,
                disposition: DeliveryDisposition::RetryAt {
                    error_code: "http_timeout".to_string(),
                    at: OffsetDateTime::now_utc() + time::Duration::milliseconds(100),
                },
                attempt: None,
            })
            .await
            .unwrap();
        let runner = Arc::new(RecordingRunner::new());
        let worker = tokio::spawn(run_delivery_worker(
            store,
            config,
            Arc::new(reqwest::Client::new()),
            runner.clone(),
            Duration::from_secs(1),
            DeliveryWakeup::new(),
        ));

        tokio::time::timeout(Duration::from_secs(1), runner.wait_for_calls(1))
            .await
            .expect("retry should run at its own deadline");
        worker.abort();
    }

    /// Helper: create an in-memory store, insert a message + delivery,
    /// claim it, and return the claimed row along with the store so the
    /// caller can reuse the store reference in `process_delivery_inner`.
    async fn setup_claimed_delivery(
        store: &Store,
        profile_key: &str,
        prior_attempts: i64,
    ) -> ClaimedDelivery {
        insert_delivery(store, "delivery test body", vec![profile_key.to_string()]).await;

        for _ in 0..prior_attempts {
            let claim = store
                .claim_deliveries(1, Duration::from_secs(90))
                .await
                .unwrap()
                .pop()
                .unwrap();
            store
                .complete_delivery(CompleteDelivery {
                    claim: claim.claim,
                    disposition: DeliveryDisposition::RetryAt {
                        error_code: "http_timeout".to_string(),
                        at: OffsetDateTime::now_utc(),
                    },
                    attempt: None,
                })
                .await
                .unwrap();
        }

        store
            .claim_deliveries(1, Duration::from_secs(90))
            .await
            .unwrap()
            .into_iter()
            .next()
            .unwrap()
    }

    #[tokio::test]
    async fn profile_missing_results_in_permanent_failure_with_no_sample() {
        let store = memory_store().await;
        let row = setup_claimed_delivery(&store, "bark.primary", 2).await;
        assert_eq!(row.attempt_count, 2);

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

        assert!(store
            .claim_deliveries(1, Duration::from_secs(90))
            .await
            .unwrap()
            .is_empty());
        assert!(store.next_delivery_due().await.unwrap().is_none());
        assert_eq!(
            store
                .forwarding_attempts("bark.primary".to_string(), 5)
                .await
                .unwrap()
                .len(),
            0
        );
    }

    #[tokio::test]
    async fn shell_timeout_records_sample_and_retry_wait() {
        let store = memory_store().await;
        let mut row = setup_claimed_delivery(&store, "shell.test", 0).await;
        row.created_at =
            DeliveryTime::Valid(OffsetDateTime::now_utc() - time::Duration::seconds(1));

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

        assert!(store.next_delivery_due().await.unwrap().is_some());

        let samples = store
            .forwarding_attempts("shell.test".to_string(), 5)
            .await
            .unwrap();
        assert_eq!(samples.len(), 1, "one sample must be recorded");
        assert_eq!(samples[0].error_code.as_deref(), Some("shell_timeout"));
        assert!(matches!(
            samples[0].dispatch_delay_ms,
            Some(delay) if (900..=2_000).contains(&delay)
        ));
    }

    #[tokio::test]
    async fn recovered_first_attempt_measures_dispatch_from_original_creation() {
        let store = memory_store().await;
        let mut row = setup_claimed_delivery(&store, "shell.test0", 0).await;
        row.created_at =
            DeliveryTime::Valid(OffsetDateTime::now_utc() - time::Duration::seconds(1));
        row.next_attempt_at = Some(DeliveryTime::Valid(OffsetDateTime::now_utc()));
        let config = shell_config(1, 1);

        process_delivery_inner(
            &store,
            &config,
            &reqwest::Client::new(),
            &TimeoutRunner,
            Duration::from_secs(1),
            row,
        )
        .await
        .unwrap();

        let samples = store
            .forwarding_attempts("shell.test0".to_string(), 1)
            .await
            .unwrap();
        assert!(matches!(
            samples[0].dispatch_delay_ms,
            Some(delay) if (900..=2_000).contains(&delay)
        ));
    }

    #[tokio::test]
    async fn expired_delivery_is_failed_before_forwarding() {
        let store = memory_store().await;
        let mut row = setup_claimed_delivery(&store, "shell.test0", 0).await;
        row.created_at = DeliveryTime::Valid(
            OffsetDateTime::now_utc() - time::Duration::seconds(RETRY_MAX_AGE.as_secs() as i64 + 1),
        );
        let runner = RecordingRunner::new();

        process_delivery_inner(
            &store,
            &shell_config(1, 1),
            &reqwest::Client::new(),
            &runner,
            Duration::from_secs(1),
            row,
        )
        .await
        .unwrap();

        assert_eq!(runner.calls.load(Ordering::SeqCst), 0);
        assert!(store.next_delivery_due().await.unwrap().is_none());
        assert!(store
            .forwarding_attempts("shell.test0".to_string(), 1)
            .await
            .unwrap()
            .is_empty());
    }
}
