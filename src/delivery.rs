use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use log::{error, info};
use time::OffsetDateTime;

use crate::config::AppConfig;
use crate::forward::ForwardOutcome;
use crate::runner::ProcessRunner;
use crate::storage::{DeliveryRow, MessageStore};

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

    let outcome = match profile {
        Some(p) => {
            forward_to_profile(
                client,
                shell_runner,
                shell_timeout,
                &p,
                &message.phone_number,
                &message.body,
                &message.timestamp,
                config,
            )
            .await
        }
        None => ForwardOutcome::PermanentFailure("profile_missing".to_string()),
    };

    match outcome {
        ForwardOutcome::Success => {
            info!("delivery {}: success", row.id);
            ensure_completed(store.complete_delivery(
                row.id,
                "succeeded",
                None,
                row.attempt_count + 1,
                None,
                lease_token,
            )?)?;
        }
        ForwardOutcome::PermanentFailure(msg) => {
            error!("delivery {}: permanent failure: {}", row.id, msg);
            ensure_completed(store.complete_delivery(
                row.id,
                "permanent_failed",
                Some(&msg),
                row.attempt_count + 1,
                None,
                lease_token,
            )?)?;
        }
        ForwardOutcome::TransientFailure(msg) => {
            let age = delivery_age(&row.created_at);
            if age > RETRY_MAX_AGE {
                error!(
                    "delivery {}: max age exceeded, permanent failure: {}",
                    row.id, msg
                );
                ensure_completed(store.complete_delivery(
                    row.id,
                    "permanent_failed",
                    Some("max_age_exceeded"),
                    row.attempt_count + 1,
                    None,
                    lease_token,
                )?)?;
            } else {
                info!(
                    "delivery {}: transient failure, retry at {}: {}",
                    row.id, next_attempt_at, msg
                );
                ensure_completed(store.complete_delivery(
                    row.id,
                    "retry_wait",
                    Some(&msg),
                    row.attempt_count + 1,
                    Some(&next_attempt_at),
                    lease_token,
                )?)?;
            }
        }
    }
    Ok(())
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
}
