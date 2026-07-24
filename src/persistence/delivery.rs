use std::time::Duration;

use anyhow::Result;
use time::OffsetDateTime;

use crate::message::Message;
use crate::storage::{
    DeliveryCompletion as SqliteDeliveryCompletion, DeliveryState, ForwardAttemptOutcome,
    NewForwardAttemptSample,
};

use super::Store;

#[derive(Clone)]
pub struct DeliveryClaim {
    delivery_id: i64,
    profile_key: String,
    attempt_count: i64,
    lease_token: String,
}

#[derive(Clone)]
pub struct ClaimedDelivery {
    pub id: i64,
    pub message_id: i64,
    pub profile_key: String,
    pub attempt_count: i64,
    pub next_attempt_at: Option<DeliveryTime>,
    pub created_at: DeliveryTime,
    pub claim: DeliveryClaim,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryTime {
    Valid(OffsetDateTime),
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryAttemptOutcome {
    Success,
    TransientFailure,
    PermanentFailure,
}

#[derive(Debug, Clone)]
pub struct DeliveryAttempt {
    pub started_at: OffsetDateTime,
    pub completed_at: OffsetDateTime,
    pub latency: Duration,
    pub dispatch_delay: Duration,
    pub outcome: DeliveryAttemptOutcome,
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveryDisposition {
    Succeeded,
    RetryAt {
        error_code: String,
        at: OffsetDateTime,
    },
    PermanentFailure {
        error_code: String,
    },
}

#[derive(Clone)]
pub struct CompleteDelivery {
    pub claim: DeliveryClaim,
    pub disposition: DeliveryDisposition,
    pub attempt: Option<DeliveryAttempt>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionResult {
    Applied,
    OwnershipLost,
}

impl Store {
    pub async fn claim_deliveries(
        &self,
        batch_size: u32,
        lease_for: Duration,
    ) -> Result<Vec<ClaimedDelivery>> {
        self.run(move |sqlite| {
            sqlite
                .claim_due_deliveries(batch_size, lease_for)?
                .into_iter()
                .map(|row| {
                    let lease_token = row
                        .lease_token
                        .ok_or_else(|| anyhow::anyhow!("claimed delivery missing lease token"))?;
                    Ok(ClaimedDelivery {
                        id: row.id,
                        message_id: row.message_id,
                        profile_key: row.profile_key.clone(),
                        attempt_count: row.attempt_count,
                        next_attempt_at: row.next_attempt_at.map(|value| delivery_time(&value)),
                        created_at: delivery_time(&row.created_at),
                        claim: DeliveryClaim {
                            delivery_id: row.id,
                            profile_key: row.profile_key,
                            attempt_count: row.attempt_count,
                            lease_token,
                        },
                    })
                })
                .collect()
        })
        .await
    }

    pub async fn next_delivery_due(&self) -> Result<Option<OffsetDateTime>> {
        self.run(|sqlite| {
            sqlite
                .next_delivery_due_at()?
                .map(|value| parse_timestamp(&value))
                .transpose()
        })
        .await
    }

    pub async fn message_for_delivery(&self, message_id: i64) -> Result<Option<Message>> {
        self.run(move |sqlite| sqlite.get_message_optional(message_id))
            .await
    }

    pub async fn complete_delivery(
        &self,
        completion: CompleteDelivery,
    ) -> Result<CompletionResult> {
        self.run(move |sqlite| {
            let CompleteDelivery {
                claim,
                disposition,
                attempt,
            } = completion;
            let (state, error, next_attempt_at) = match disposition {
                DeliveryDisposition::Succeeded => (DeliveryState::Succeeded, None, None),
                DeliveryDisposition::RetryAt { error_code, at } => (
                    DeliveryState::RetryWait,
                    Some(error_code),
                    Some(format_timestamp(at)?),
                ),
                DeliveryDisposition::PermanentFailure { error_code } => {
                    (DeliveryState::PermanentFailed, Some(error_code), None)
                }
            };
            let attempt_count = claim
                .attempt_count
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("delivery attempt count overflow"))?;
            let applied = match attempt {
                Some(attempt) => {
                    let sample = NewForwardAttemptSample {
                        profile_key: claim.profile_key.clone(),
                        delivery_id: Some(claim.delivery_id),
                        attempt_number: attempt_count as i32,
                        started_at: format_timestamp(attempt.started_at)?,
                        completed_at: format_timestamp(attempt.completed_at)?,
                        latency_ms: duration_millis(attempt.latency).max(1),
                        dispatch_delay_ms: duration_millis(attempt.dispatch_delay),
                        outcome: match attempt.outcome {
                            DeliveryAttemptOutcome::Success => ForwardAttemptOutcome::Success,
                            DeliveryAttemptOutcome::TransientFailure => {
                                ForwardAttemptOutcome::TransientFailure
                            }
                            DeliveryAttemptOutcome::PermanentFailure => {
                                ForwardAttemptOutcome::PermanentFailure
                            }
                        },
                        error_code: attempt.error_code,
                    };
                    // The SQLite operation intentionally inserts the real provider
                    // attempt even when the delivery CAS reports ownership loss.
                    sqlite.complete_delivery_with_attempt(SqliteDeliveryCompletion {
                        id: claim.delivery_id,
                        state,
                        error: error.as_deref(),
                        attempt_count,
                        next_attempt_at: next_attempt_at.as_deref(),
                        lease_token: &claim.lease_token,
                        sample,
                    })?
                }
                None => sqlite.complete_delivery(
                    claim.delivery_id,
                    state,
                    error.as_deref(),
                    attempt_count,
                    next_attempt_at.as_deref(),
                    &claim.lease_token,
                )?,
            };
            Ok(if applied {
                CompletionResult::Applied
            } else {
                CompletionResult::OwnershipLost
            })
        })
        .await
    }
}

fn parse_timestamp(value: &str) -> Result<OffsetDateTime> {
    OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339).map_err(Into::into)
}

fn delivery_time(value: &str) -> DeliveryTime {
    parse_timestamp(value)
        .map(DeliveryTime::Valid)
        .unwrap_or(DeliveryTime::Invalid)
}

fn format_timestamp(value: OffsetDateTime) -> Result<String> {
    value
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(Into::into)
}

fn duration_millis(duration: Duration) -> i64 {
    duration.as_millis().min(i64::MAX as u128) as i64
}
