use rusqlite::Row;
use time::OffsetDateTime;

use crate::message::{Message, MessageDirection, MessageSource, MessageStatus};

use super::{DeliveryRow, DeliveryState, ForwardAttemptOutcome, ForwardAttemptSample};

pub(super) fn row_to_message(row: &Row) -> rusqlite::Result<Message> {
    Ok(Message {
        id: row.get(0)?,
        direction: str_to_direction(&row.get::<_, String>(1)?, 1)?,
        phone_number: row.get(2)?,
        body: row.get(3)?,
        timestamp: row.get(4)?,
        status: str_to_status(&row.get::<_, String>(5)?, 5)?,
        source: str_to_source(&row.get::<_, String>(6)?, 6)?,
        modem_sms_path: row.get(7)?,
        read_at: row.get(8)?,
        error: row.get(9)?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
    })
}

pub(super) fn direction_to_str(direction: MessageDirection) -> &'static str {
    match direction {
        MessageDirection::Inbound => "inbound",
        MessageDirection::Outbound => "outbound",
    }
}

pub(super) fn status_to_str(status: MessageStatus) -> &'static str {
    match status {
        MessageStatus::Received => "received",
        MessageStatus::Sending => "sending",
        MessageStatus::Sent => "sent",
        MessageStatus::Failed => "failed",
    }
}

pub(super) fn source_to_str(source: MessageSource) -> &'static str {
    match source {
        MessageSource::Modem => "modem",
        MessageSource::Web => "web",
        MessageSource::Cli => "cli",
    }
}

pub(super) fn str_to_direction(value: &str, column: usize) -> rusqlite::Result<MessageDirection> {
    match value {
        "inbound" => Ok(MessageDirection::Inbound),
        "outbound" => Ok(MessageDirection::Outbound),
        _ => Err(invalid_enum_value(column, "message direction", value)),
    }
}

pub(super) fn str_to_status(value: &str, column: usize) -> rusqlite::Result<MessageStatus> {
    match value {
        "received" => Ok(MessageStatus::Received),
        "sending" => Ok(MessageStatus::Sending),
        "sent" => Ok(MessageStatus::Sent),
        "failed" => Ok(MessageStatus::Failed),
        _ => Err(invalid_enum_value(column, "message status", value)),
    }
}

pub(super) fn str_to_source(value: &str, column: usize) -> rusqlite::Result<MessageSource> {
    match value {
        "modem" => Ok(MessageSource::Modem),
        "web" => Ok(MessageSource::Web),
        "cli" => Ok(MessageSource::Cli),
        _ => Err(invalid_enum_value(column, "message source", value)),
    }
}

pub(super) fn outcome_to_str(outcome: &ForwardAttemptOutcome) -> &'static str {
    match outcome {
        ForwardAttemptOutcome::Success => "success",
        ForwardAttemptOutcome::TransientFailure => "transient_failure",
        ForwardAttemptOutcome::PermanentFailure => "permanent_failure",
    }
}

fn str_to_outcome(value: &str, column: usize) -> rusqlite::Result<ForwardAttemptOutcome> {
    match value {
        "success" => Ok(ForwardAttemptOutcome::Success),
        "transient_failure" => Ok(ForwardAttemptOutcome::TransientFailure),
        "permanent_failure" => Ok(ForwardAttemptOutcome::PermanentFailure),
        _ => Err(invalid_enum_value(column, "forward attempt outcome", value)),
    }
}

fn invalid_enum_value(column: usize, kind: &str, value: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        column,
        rusqlite::types::Type::Text,
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("unknown {kind}: {value}"),
        )
        .into(),
    )
}

pub(super) fn row_to_attempt_sample(row: &Row) -> rusqlite::Result<ForwardAttemptSample> {
    Ok(ForwardAttemptSample {
        id: row.get(0)?,
        profile_key: row.get(1)?,
        delivery_id: row.get(2)?,
        attempt_number: row.get(3)?,
        started_at: row.get(4)?,
        completed_at: row.get(5)?,
        latency_ms: row.get(6)?,
        dispatch_delay_ms: row.get(7)?,
        outcome: str_to_outcome(&row.get::<_, String>(8)?, 8)?,
        error_code: row.get(9)?,
    })
}

pub(super) fn row_to_delivery(row: &Row) -> rusqlite::Result<DeliveryRow> {
    let state = row.get::<_, String>(3)?;
    let state = str_to_delivery_state(&state, 3)?;
    Ok(DeliveryRow {
        id: row.get(0)?,
        message_id: row.get(1)?,
        profile_key: row.get(2)?,
        state,
        attempt_count: row.get(4)?,
        next_attempt_at: row.get(5)?,
        lease_at: row.get(6)?,
        lease_token: row.get(7)?,
        last_error: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
    })
}

fn str_to_delivery_state(value: &str, column: usize) -> rusqlite::Result<DeliveryState> {
    match value {
        "pending" => Ok(DeliveryState::Pending),
        "in_flight" => Ok(DeliveryState::InFlight),
        "retry_wait" => Ok(DeliveryState::RetryWait),
        "succeeded" => Ok(DeliveryState::Succeeded),
        "permanent_failed" => Ok(DeliveryState::PermanentFailed),
        _ => Err(invalid_enum_value(column, "delivery state", value)),
    }
}

pub(super) fn now_string() -> String {
    OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}
