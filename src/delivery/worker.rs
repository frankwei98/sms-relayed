use std::collections::VecDeque;
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use log::{error, info};
use time::OffsetDateTime;

use crate::persistence::{
    ClaimedDelivery, CompleteDelivery, CompletionResult, DeliveryAttempt, DeliveryAttemptOutcome,
    DeliveryClaim, DeliveryDisposition, DeliveryTime, Store,
};

use super::dispatcher::{DispatchOutcome, DispatchRequest, DispatchResult, Dispatcher};
use super::DeliveryWakeup;

const LEASE_SECS: u64 = 90;
const RETRY_INITIAL_DELAY: u64 = 30;
const RETRY_MAX_DELAY: u64 = 3600;
const RETRY_MAX_AGE: Duration = Duration::from_secs(86400);
const SAFETY_SCAN_INTERVAL: Duration = Duration::from_secs(30);
const WORKER_ERROR_INITIAL_DELAY: Duration = Duration::from_secs(1);
const WORKER_ERROR_MAX_DELAY: Duration = Duration::from_secs(30);
const CLAIMED_WAVES_PER_BATCH: usize = 4;

pub(crate) struct DeliverySettings {
    pub(super) concurrency: usize,
    pub(super) channel_timeout: Duration,
}

#[derive(Clone, Copy)]
struct WorkerTiming {
    safety_scan_interval: Duration,
    error_initial_delay: Duration,
    error_max_delay: Duration,
}

impl Default for WorkerTiming {
    fn default() -> Self {
        Self {
            safety_scan_interval: SAFETY_SCAN_INTERVAL,
            error_initial_delay: WORKER_ERROR_INITIAL_DELAY,
            error_max_delay: WORKER_ERROR_MAX_DELAY,
        }
    }
}

#[cfg(test)]
#[derive(Default)]
struct WorkerTestProbe {
    idle_waits: AtomicUsize,
    idle_wait_changed: tokio::sync::Notify,
    backoffs: AtomicUsize,
}

#[cfg(test)]
impl WorkerTestProbe {
    fn record_idle_wait(&self) {
        self.idle_waits.fetch_add(1, Ordering::SeqCst);
        self.idle_wait_changed.notify_waiters();
    }

    fn record_backoff(&self) {
        self.backoffs.fetch_add(1, Ordering::SeqCst);
    }

    async fn wait_for_idle_waits(&self, expected: usize) {
        wait_for_probe_count(&self.idle_waits, &self.idle_wait_changed, expected).await;
    }
}

#[cfg(test)]
async fn wait_for_probe_count(count: &AtomicUsize, changed: &tokio::sync::Notify, expected: usize) {
    loop {
        let notified = changed.notified();
        if count.load(Ordering::SeqCst) >= expected {
            return;
        }
        notified.await;
    }
}

pub(crate) struct DeliveryWorker {
    store: Store,
    settings: DeliverySettings,
    dispatcher: Arc<dyn Dispatcher>,
    wakeup: DeliveryWakeup,
    timing: WorkerTiming,
    #[cfg(test)]
    test_probe: Option<Arc<WorkerTestProbe>>,
}

impl DeliveryWorker {
    pub(super) fn with_dispatcher(
        store: Store,
        settings: DeliverySettings,
        dispatcher: Arc<dyn Dispatcher>,
        wakeup: DeliveryWakeup,
    ) -> Self {
        Self {
            store,
            settings,
            dispatcher,
            wakeup,
            timing: WorkerTiming::default(),
            #[cfg(test)]
            test_probe: None,
        }
    }

    #[cfg(test)]
    fn with_test_timing(mut self, timing: WorkerTiming) -> Self {
        self.timing = timing;
        self
    }

    #[cfg(test)]
    fn with_test_probe(mut self, probe: Arc<WorkerTestProbe>) -> Self {
        self.test_probe = Some(probe);
        self
    }

    pub(crate) async fn run(self) {
        let mut error_delay = self.timing.error_initial_delay;
        loop {
            match drain_due_deliveries(&self.store, &self.settings, &self.dispatcher).await {
                Ok(processed) => {
                    if processed > 0 {
                        log::debug!("delivery queue drained; processed={processed}");
                    }
                }
                Err(e) => {
                    #[cfg(test)]
                    if let Some(probe) = &self.test_probe {
                        probe.record_backoff();
                    }
                    backoff_after_worker_error(
                        "queue drain",
                        &e,
                        &mut error_delay,
                        self.timing.error_max_delay,
                    )
                    .await;
                    continue;
                }
            }

            let retry_delay = match next_delivery_delay(&self.store).await {
                Ok(delay) => delay,
                Err(e) => {
                    #[cfg(test)]
                    if let Some(probe) = &self.test_probe {
                        probe.record_backoff();
                    }
                    backoff_after_worker_error(
                        "deadline scheduling",
                        &e,
                        &mut error_delay,
                        self.timing.error_max_delay,
                    )
                    .await;
                    continue;
                }
            };
            error_delay = self.timing.error_initial_delay;

            #[cfg(test)]
            if retry_delay.is_none() {
                if let Some(probe) = &self.test_probe {
                    probe.record_idle_wait();
                }
            }

            let reason = tokio::select! {
                _ = self.wakeup.wait() => "new_delivery",
                _ = wait_for_retry_deadline(retry_delay) => "retry_deadline",
                _ = tokio::time::sleep(self.timing.safety_scan_interval) => "safety_scan",
            };
            log::debug!("delivery worker woke; reason={reason}");
        }
    }
}

async fn backoff_after_worker_error(
    operation: &str,
    error: &anyhow::Error,
    delay: &mut Duration,
    max_delay: Duration,
) {
    log::error!("delivery worker {operation} failed: {error}");
    crate::monitoring::capture_failure("delivery", "delivery.worker_failed");
    log::debug!(
        "delivery worker backing off; operation={operation} delay_secs={}",
        delay.as_secs()
    );
    tokio::time::sleep(*delay).await;
    *delay = (*delay * 2).min(max_delay);
}

async fn drain_due_deliveries(
    store: &Store,
    settings: &DeliverySettings,
    dispatcher: &Arc<dyn Dispatcher>,
) -> Result<usize> {
    let mut processed = 0;
    loop {
        let count = process_delivery_batch(store, settings, dispatcher).await?;
        if count == 0 {
            return Ok(processed);
        }
        processed += count;
        tokio::task::yield_now().await;
    }
}

async fn process_delivery_batch(
    store: &Store,
    settings: &DeliverySettings,
    dispatcher: &Arc<dyn Dispatcher>,
) -> Result<usize> {
    let concurrency = settings.concurrency;
    let mut rows: VecDeque<_> = claim_delivery_batch(store, settings).await?.into();
    if rows.is_empty() {
        return Ok(0);
    }

    let mut claimed_count = rows.len();
    let mut tasks = tokio::task::JoinSet::new();
    let task_context = DeliveryTaskContext { store, dispatcher };
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
            match claim_delivery_batch(store, settings).await {
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

struct DeliveryTaskContext<'a> {
    store: &'a Store,
    dispatcher: &'a Arc<dyn Dispatcher>,
}

async fn claim_delivery_batch(
    store: &Store,
    settings: &DeliverySettings,
) -> Result<Vec<ClaimedDelivery>> {
    let batch_size = settings.concurrency.saturating_mul(CLAIMED_WAVES_PER_BATCH) as u32;
    let lease_secs = LEASE_SECS.saturating_add(
        settings
            .channel_timeout
            .as_secs()
            .saturating_mul(CLAIMED_WAVES_PER_BATCH as u64),
    );
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
    let dispatcher = context.dispatcher.clone();
    tasks.spawn(async move { process_delivery_inner(&store, &*dispatcher, row).await });
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
    dispatcher: &dyn Dispatcher,
    row: ClaimedDelivery,
) -> Result<()> {
    if matches!(row.next_attempt_at, Some(DeliveryTime::Invalid)) {
        error!("delivery {}: invalid retry deadline", row.id);
        process_no_sample_path(store, row.claim.clone(), "invalid_retry_deadline").await?;
        return Ok(());
    }

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

    let dispatch = dispatcher.dispatch(DispatchRequest {
        profile_key: &row.profile_key,
        phone_number: &message.phone_number,
        body: &message.body,
        timestamp: &message.timestamp,
    });
    let attempt_started_at = OffsetDateTime::now_utc();
    let dispatch_delay_ms = dispatch_delay_ms(&row, attempt_started_at);
    let start = Instant::now();
    let outcome = match dispatch.await {
        DispatchResult::Attempted(outcome) => outcome,
        DispatchResult::ProfileMissing => {
            process_no_sample_path(store, row.claim.clone(), "profile_missing").await?;
            return Ok(());
        }
    };
    let latency_us = start.elapsed().as_micros() as i64;

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
        DispatchOutcome::Success => {
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
        DispatchOutcome::PermanentFailure(_) => {
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
        DispatchOutcome::TransientFailure(_) => {
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

fn map_outcome_to_attempt(outcome: &DispatchOutcome) -> DeliveryAttemptOutcome {
    match outcome {
        DispatchOutcome::Success => DeliveryAttemptOutcome::Success,
        DispatchOutcome::TransientFailure(_) => DeliveryAttemptOutcome::TransientFailure,
        DispatchOutcome::PermanentFailure(_) => DeliveryAttemptOutcome::PermanentFailure,
    }
}

fn map_outcome_to_delivery_state(outcome: &DispatchOutcome) -> Option<String> {
    match outcome {
        DispatchOutcome::Success => None,
        DispatchOutcome::TransientFailure(ref msg) | DispatchOutcome::PermanentFailure(ref msg) => {
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
    use std::sync::Arc;
    use std::time::Duration;

    use crate::persistence::{
        CompleteDelivery, DeliveryDisposition, InboundMessage, InboundOutcome, Store,
    };

    use super::super::dispatcher::{Dispatcher, ScriptedAction, ScriptedDispatcher};
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

    async fn memory_store() -> Store {
        let store = Store::open_in_memory().unwrap();
        store
            .set_modem_fingerprint("delivery-test-modem".to_string())
            .await
            .unwrap();
        store
    }

    async fn insert_delivery(store: &Store, body: &str, profile_keys: Vec<String>) -> i64 {
        let outcome = store
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
        match outcome {
            InboundOutcome::Inserted(message) => message.id,
            InboundOutcome::Duplicate => panic!("test delivery must be unique"),
        }
    }

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

    fn test_settings(concurrency: usize) -> DeliverySettings {
        DeliverySettings {
            concurrency,
            channel_timeout: Duration::from_secs(1),
        }
    }

    fn scripted_worker(
        store: Store,
        concurrency: usize,
        dispatcher: Arc<ScriptedDispatcher>,
        wakeup: DeliveryWakeup,
    ) -> DeliveryWorker {
        let dispatcher: Arc<dyn Dispatcher> = dispatcher;
        DeliveryWorker::with_dispatcher(store, test_settings(concurrency), dispatcher, wakeup)
    }

    fn assert_stored_delivery(
        store: &Store,
        id: i64,
        state: crate::storage::DeliveryState,
        attempt_count: i64,
        last_error: Option<&str>,
        has_retry_deadline: bool,
    ) {
        let delivery = store.sqlite().get_delivery(id).unwrap();
        assert_eq!(delivery.state, state);
        assert_eq!(delivery.attempt_count, attempt_count);
        assert_eq!(delivery.last_error.as_deref(), last_error);
        assert_eq!(delivery.next_attempt_at.is_some(), has_retry_deadline);
        assert!(delivery.lease_at.is_none());
        assert!(delivery.lease_token.is_none());
    }

    async fn wait_for_attempt_count(store: &Store, profile_keys: &[String], expected: usize) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let mut count = 0;
                for profile_key in profile_keys {
                    count += store
                        .forwarding_attempts(profile_key.clone(), 5)
                        .await
                        .unwrap()
                        .len();
                }
                if count == expected {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("delivery attempts should be persisted");
    }

    #[tokio::test]
    async fn scripted_success_records_success_sample_and_completes_delivery() {
        let store = memory_store().await;
        let row = setup_claimed_delivery(&store, "bark.primary", 0).await;
        let delivery_id = row.id;
        let dispatcher = ScriptedDispatcher::new([ScriptedAction::Success]);

        process_delivery_inner(&store, &dispatcher, row)
            .await
            .unwrap();

        assert_stored_delivery(
            &store,
            delivery_id,
            crate::storage::DeliveryState::Succeeded,
            1,
            None,
            false,
        );
        let samples = store
            .forwarding_attempts("bark.primary".to_string(), 5)
            .await
            .unwrap();
        assert_eq!(samples.len(), 1);
        assert!(matches!(
            samples[0].outcome,
            crate::storage::ForwardAttemptOutcome::Success
        ));
        assert!(samples[0].error_code.is_none());
    }

    #[tokio::test]
    async fn scripted_transient_failure_records_sample_and_retry_deadline() {
        let store = memory_store().await;
        let mut row = setup_claimed_delivery(&store, "bark.primary", 0).await;
        let delivery_id = row.id;
        row.created_at =
            DeliveryTime::Valid(OffsetDateTime::now_utc() - time::Duration::seconds(1));
        let dispatcher =
            ScriptedDispatcher::new([ScriptedAction::TransientFailure("http_timeout".to_string())]);
        let before = OffsetDateTime::now_utc();

        process_delivery_inner(&store, &dispatcher, row)
            .await
            .unwrap();

        let due = store.next_delivery_due().await.unwrap().unwrap();
        let delay = due - before;
        assert!((22..=38).contains(&delay.whole_seconds()));
        assert_stored_delivery(
            &store,
            delivery_id,
            crate::storage::DeliveryState::RetryWait,
            1,
            Some("http_timeout"),
            true,
        );
        let samples = store
            .forwarding_attempts("bark.primary".to_string(), 5)
            .await
            .unwrap();
        assert_eq!(samples.len(), 1);
        assert!(matches!(
            samples[0].outcome,
            crate::storage::ForwardAttemptOutcome::TransientFailure
        ));
        assert_eq!(samples[0].error_code.as_deref(), Some("http_timeout"));
    }

    #[tokio::test]
    async fn scripted_permanent_failure_records_sample_and_completes_delivery() {
        let store = memory_store().await;
        let row = setup_claimed_delivery(&store, "bark.primary", 0).await;
        let delivery_id = row.id;
        let dispatcher = ScriptedDispatcher::new([ScriptedAction::PermanentFailure(
            "http_status_400".to_string(),
        )]);

        process_delivery_inner(&store, &dispatcher, row)
            .await
            .unwrap();

        assert_stored_delivery(
            &store,
            delivery_id,
            crate::storage::DeliveryState::PermanentFailed,
            1,
            Some("http_status_400"),
            false,
        );
        let samples = store
            .forwarding_attempts("bark.primary".to_string(), 5)
            .await
            .unwrap();
        assert_eq!(samples.len(), 1);
        assert!(matches!(
            samples[0].outcome,
            crate::storage::ForwardAttemptOutcome::PermanentFailure
        ));
        assert_eq!(samples[0].error_code.as_deref(), Some("http_status_400"));
    }

    #[tokio::test]
    async fn profile_missing_results_in_permanent_failure_with_no_sample() {
        let store = memory_store().await;
        let row = setup_claimed_delivery(&store, "bark.primary", 2).await;
        assert_eq!(row.attempt_count, 2);
        let delivery_id = row.id;
        let dispatcher = ScriptedDispatcher::new([ScriptedAction::ProfileMissing]);

        process_delivery_inner(&store, &dispatcher, row)
            .await
            .unwrap();

        assert_stored_delivery(
            &store,
            delivery_id,
            crate::storage::DeliveryState::PermanentFailed,
            3,
            Some("profile_missing"),
            false,
        );
        assert!(store
            .forwarding_attempts("bark.primary".to_string(), 5)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn message_missing_results_in_no_fabricated_attempt() {
        let store = memory_store().await;
        let mut row = setup_claimed_delivery(&store, "bark.primary", 0).await;
        let delivery_id = row.id;
        row.message_id = i64::MAX;
        let dispatcher = ScriptedDispatcher::new([ScriptedAction::Success]);

        process_delivery_inner(&store, &dispatcher, row)
            .await
            .unwrap();

        assert!(dispatcher.requests().is_empty());
        assert_stored_delivery(
            &store,
            delivery_id,
            crate::storage::DeliveryState::PermanentFailed,
            1,
            Some("message_not_found"),
            false,
        );
        assert!(store
            .forwarding_attempts("bark.primary".to_string(), 5)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn malformed_retry_deadline_is_failed_before_forwarding() {
        let store = memory_store().await;
        let message_id = insert_delivery(
            &store,
            "malformed deadline",
            vec!["bark.primary".to_string()],
        )
        .await;
        store
            .sqlite()
            .set_delivery_retry_deadline(message_id, "not-rfc3339")
            .unwrap();
        let row = store
            .claim_deliveries(1, Duration::from_secs(90))
            .await
            .unwrap()
            .pop()
            .unwrap();
        let delivery_id = row.id;
        let dispatcher = ScriptedDispatcher::new([ScriptedAction::Success]);

        process_delivery_inner(&store, &dispatcher, row)
            .await
            .unwrap();

        assert!(dispatcher.requests().is_empty());
        assert_stored_delivery(
            &store,
            delivery_id,
            crate::storage::DeliveryState::PermanentFailed,
            1,
            Some("invalid_retry_deadline"),
            false,
        );
    }

    #[tokio::test]
    async fn recovered_first_attempt_measures_dispatch_from_original_creation() {
        let store = memory_store().await;
        let mut row = setup_claimed_delivery(&store, "bark.primary", 0).await;
        row.created_at =
            DeliveryTime::Valid(OffsetDateTime::now_utc() - time::Duration::seconds(1));
        row.next_attempt_at = Some(DeliveryTime::Valid(OffsetDateTime::now_utc()));
        let dispatcher = ScriptedDispatcher::new([ScriptedAction::Success]);

        process_delivery_inner(&store, &dispatcher, row)
            .await
            .unwrap();

        let samples = store
            .forwarding_attempts("bark.primary".to_string(), 1)
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
        let mut row = setup_claimed_delivery(&store, "bark.primary", 0).await;
        let delivery_id = row.id;
        row.created_at = DeliveryTime::Valid(
            OffsetDateTime::now_utc() - time::Duration::seconds(RETRY_MAX_AGE.as_secs() as i64 + 1),
        );
        let dispatcher = ScriptedDispatcher::new([ScriptedAction::Success]);

        process_delivery_inner(&store, &dispatcher, row)
            .await
            .unwrap();

        assert!(dispatcher.requests().is_empty());
        assert_stored_delivery(
            &store,
            delivery_id,
            crate::storage::DeliveryState::PermanentFailed,
            1,
            Some("max_age_exceeded"),
            false,
        );
        assert!(store
            .forwarding_attempts("bark.primary".to_string(), 1)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn committed_delivery_notification_wakes_idle_worker_immediately() {
        let store = memory_store().await;
        let dispatcher = Arc::new(ScriptedDispatcher::new([ScriptedAction::Success]));
        let wakeup = DeliveryWakeup::new();
        let worker = tokio::spawn(
            scripted_worker(store.clone(), 1, dispatcher.clone(), wakeup.clone()).run(),
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
        insert_delivery(&store, "wake now", vec!["bark.primary".to_string()]).await;

        wakeup.notify();

        tokio::time::timeout(Duration::from_secs(1), dispatcher.wait_for_calls(1))
            .await
            .expect("notified delivery should not wait for the safety scan");
        worker.abort();
        let _ = worker.await;
    }

    #[tokio::test]
    async fn worker_drains_more_than_one_batch_at_configured_concurrency() {
        let store = memory_store().await;
        let profile_keys: Vec<_> = (0..5).map(|index| format!("bark.test{index}")).collect();
        insert_delivery(&store, "drain queue", profile_keys.clone()).await;
        let dispatcher =
            Arc::new(ScriptedDispatcher::new((0..5).map(|_| {
                ScriptedAction::SuccessAfter(Duration::from_millis(20))
            })));
        let worker = tokio::spawn(
            scripted_worker(store.clone(), 2, dispatcher.clone(), DeliveryWakeup::new()).run(),
        );

        wait_for_attempt_count(&store, &profile_keys, 5).await;
        assert_eq!(dispatcher.max_active(), 2);
        worker.abort();
        let _ = worker.await;
    }

    #[tokio::test]
    async fn worker_replenishes_a_free_slot_without_waiting_for_slow_peer() {
        let store = memory_store().await;
        let profile_keys: Vec<_> = (0..10).map(|index| format!("bark.test{index}")).collect();
        insert_delivery(&store, "rolling queue", profile_keys).await;
        let actions = std::iter::once(ScriptedAction::SuccessAfter(Duration::from_millis(500)))
            .chain((1..10).map(|_| ScriptedAction::SuccessAfter(Duration::from_millis(5))));
        let dispatcher = Arc::new(ScriptedDispatcher::new(actions));
        let worker = tokio::spawn(
            scripted_worker(store, 2, dispatcher.clone(), DeliveryWakeup::new()).run(),
        );

        tokio::time::timeout(Duration::from_millis(250), dispatcher.wait_for_calls(10))
            .await
            .expect("a free slot should claim beyond the buffered batch");
        assert_eq!(dispatcher.max_active(), 2);
        worker.abort();
        let _ = worker.await;
    }

    #[tokio::test]
    async fn worker_refills_concurrency_after_an_initial_single_delivery() {
        let store = memory_store().await;
        insert_delivery(&store, "initial delivery", vec!["bark.primary".to_string()]).await;
        let dispatcher =
            Arc::new(ScriptedDispatcher::new((0..6).map(|_| {
                ScriptedAction::SuccessAfter(Duration::from_millis(100))
            })));
        let worker = tokio::spawn(
            scripted_worker(store.clone(), 2, dispatcher.clone(), DeliveryWakeup::new()).run(),
        );
        tokio::time::timeout(Duration::from_secs(1), dispatcher.wait_for_calls(1))
            .await
            .unwrap();

        for index in 0..5 {
            insert_delivery(
                &store,
                &format!("burst {index}"),
                vec!["bark.primary".to_string()],
            )
            .await;
        }

        tokio::time::timeout(Duration::from_secs(1), dispatcher.wait_for_calls(3))
            .await
            .expect("burst deliveries should start promptly");
        assert_eq!(dispatcher.max_active(), 2);
        worker.abort();
        let _ = worker.await;
    }

    #[tokio::test]
    async fn retry_deadline_wakes_worker_without_a_new_delivery_notification() {
        let store = memory_store().await;
        insert_delivery(&store, "retry deadline", vec!["bark.primary".to_string()]).await;
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
        let dispatcher = Arc::new(ScriptedDispatcher::new([ScriptedAction::Success]));
        let worker = tokio::spawn(
            scripted_worker(store, 1, dispatcher.clone(), DeliveryWakeup::new()).run(),
        );

        tokio::time::timeout(Duration::from_secs(1), dispatcher.wait_for_calls(1))
            .await
            .expect("retry should run at its own deadline");
        worker.abort();
        let _ = worker.await;
    }

    #[tokio::test]
    async fn safety_scan_finds_due_delivery_without_notification() {
        let store = memory_store().await;
        let dispatcher = Arc::new(ScriptedDispatcher::new([ScriptedAction::Success]));
        let probe = Arc::new(WorkerTestProbe::default());
        let worker = scripted_worker(store.clone(), 1, dispatcher.clone(), DeliveryWakeup::new())
            .with_test_timing(WorkerTiming {
                safety_scan_interval: Duration::from_millis(50),
                error_initial_delay: Duration::from_millis(10),
                error_max_delay: Duration::from_millis(20),
            })
            .with_test_probe(probe.clone());
        let worker = tokio::spawn(worker.run());
        tokio::time::timeout(Duration::from_secs(1), probe.wait_for_idle_waits(1))
            .await
            .expect("worker should be waiting with no known delivery deadline");

        insert_delivery(&store, "safety scan", vec!["bark.primary".to_string()]).await;

        tokio::time::timeout(Duration::from_secs(1), dispatcher.wait_for_calls(1))
            .await
            .expect("safety scan should eventually discover a due delivery");
        worker.abort();
        let _ = worker.await;
    }

    #[tokio::test]
    async fn malformed_deadline_does_not_backoff_or_block_valid_delivery() {
        let store = memory_store().await;
        let message_id =
            insert_delivery(&store, "malformed", vec!["bark.primary".to_string()]).await;
        store
            .sqlite()
            .set_delivery_retry_deadline(message_id, "not-rfc3339")
            .unwrap();
        insert_delivery(&store, "valid", vec!["bark.primary".to_string()]).await;
        let dispatcher = Arc::new(ScriptedDispatcher::new([ScriptedAction::Success]));
        let probe = Arc::new(WorkerTestProbe::default());
        let worker = scripted_worker(store.clone(), 1, dispatcher.clone(), DeliveryWakeup::new())
            .with_test_timing(WorkerTiming {
                safety_scan_interval: Duration::from_secs(1),
                error_initial_delay: Duration::from_millis(120),
                error_max_delay: Duration::from_millis(120),
            })
            .with_test_probe(probe.clone());
        let worker = tokio::spawn(worker.run());

        tokio::time::timeout(Duration::from_secs(1), dispatcher.wait_for_calls(1))
            .await
            .expect("valid delivery should run after malformed deadline is recovered");
        assert_eq!(probe.backoffs.load(std::sync::atomic::Ordering::SeqCst), 0);
        worker.abort();
        let _ = worker.await;
    }

    #[tokio::test]
    async fn ownership_lost_is_control_flow_and_retains_real_attempt_sample() {
        let store = memory_store().await;
        insert_delivery(&store, "ownership lost", vec!["bark.primary".to_string()]).await;
        let stale = store
            .claim_deliveries(1, Duration::ZERO)
            .await
            .unwrap()
            .pop()
            .unwrap();
        let _current = store
            .claim_deliveries(1, Duration::from_secs(90))
            .await
            .unwrap()
            .pop()
            .unwrap();
        let dispatcher = ScriptedDispatcher::new([ScriptedAction::Success]);

        process_delivery_inner(&store, &dispatcher, stale)
            .await
            .unwrap();

        assert_eq!(dispatcher.requests().len(), 1);
        assert_eq!(
            store
                .forwarding_attempts("bark.primary".to_string(), 5)
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(store
            .claim_deliveries(1, Duration::from_secs(90))
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn dropping_worker_cancels_hanging_dispatch_without_completion() {
        let store = memory_store().await;
        insert_delivery(&store, "hang", vec!["bark.primary".to_string()]).await;
        let dispatcher = Arc::new(ScriptedDispatcher::new([ScriptedAction::Hang]));
        let worker = tokio::spawn(
            scripted_worker(store.clone(), 1, dispatcher.clone(), DeliveryWakeup::new()).run(),
        );
        tokio::time::timeout(Duration::from_secs(1), dispatcher.wait_for_calls(1))
            .await
            .unwrap();
        assert_eq!(dispatcher.active(), 1);

        worker.abort();
        let _ = worker.await;

        assert_eq!(dispatcher.active(), 0);
        assert_eq!(dispatcher.cancellations(), 1);
        assert!(store
            .forwarding_attempts("bark.primary".to_string(), 5)
            .await
            .unwrap()
            .is_empty());
        assert!(store.next_delivery_due().await.unwrap().is_none());
        assert!(store
            .claim_deliveries(1, Duration::from_secs(90))
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn claim_wave_remains_four_times_concurrency() {
        let store = memory_store().await;
        let profile_keys: Vec<_> = (0..10).map(|index| format!("bark.test{index}")).collect();
        insert_delivery(&store, "claim wave", profile_keys).await;
        let dispatcher = Arc::new(ScriptedDispatcher::new([
            ScriptedAction::Hang,
            ScriptedAction::Hang,
        ]));
        let worker = tokio::spawn(
            scripted_worker(store.clone(), 2, dispatcher.clone(), DeliveryWakeup::new()).run(),
        );
        tokio::time::timeout(Duration::from_secs(1), dispatcher.wait_for_calls(2))
            .await
            .unwrap();

        worker.abort();
        let _ = worker.await;

        let still_due = store
            .claim_deliveries(20, Duration::from_secs(90))
            .await
            .unwrap();
        assert_eq!(
            still_due.len(),
            2,
            "concurrency 2 must claim exactly 8 rows per wave"
        );
    }
}
