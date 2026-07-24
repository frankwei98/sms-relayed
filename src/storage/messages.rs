use std::time::Duration;

use anyhow::Result;
use rusqlite::{
    params, params_from_iter, types::Value, Connection, OptionalExtension, TransactionBehavior,
};
use time::OffsetDateTime;

use crate::message::{ConversationSummary, Message, MessageCursor, MessageFilter, MessageStatus};
use crate::message::{IdempotencyConflict, IdempotencyReplayUnavailable};

use super::codecs::{
    direction_to_str, now_string, row_to_message, source_to_str, status_to_str, str_to_direction,
    str_to_source, str_to_status,
};
use super::{
    compute_outbound_request_hash, InboundInsertResult, InvalidMessageCursor, MessageStore,
    NewMessage,
};

fn operation_timestamps(after: Duration) -> Result<(String, String)> {
    let now = OffsetDateTime::now_utc();
    let deadline = now + time::Duration::try_from(after)?;
    let format = &time::format_description::well_known::Rfc3339;
    Ok((now.format(format)?, deadline.format(format)?))
}

struct PhaseTransition<'a> {
    id: i64,
    owner: &'a str,
    expected_phase: &'a str,
    next_phase: &'a str,
    modem_sms_path: Option<&'a str>,
    error: Option<&'a str>,
    next_attempt_at: Option<&'a str>,
    lease_for: Duration,
}

impl MessageStore {
    pub fn create_or_get_outbound(
        &self,
        input: NewMessage,
        idempotency_key: Option<&str>,
        owner: &str,
        lease_for: Duration,
    ) -> Result<(Message, bool)> {
        let mut conn = self.conn.lock().unwrap();
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(key) = idempotency_key {
            let request_hash =
                compute_outbound_request_hash(&input.phone_number, &input.body, input.source);
            let existing = transaction
                .query_row(
                    "SELECT request_hash, message_id
                     FROM outbound_idempotency
                     WHERE key = ?1",
                    params![key],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<i64>>(1)?)),
                )
                .optional()?;
            if let Some((existing_hash, message_id)) = existing {
                if existing_hash != request_hash {
                    return Err(IdempotencyConflict.into());
                }
                let message_id = message_id.ok_or(IdempotencyReplayUnavailable)?;
                let message =
                    map_find(&transaction, message_id)?.ok_or(IdempotencyReplayUnavailable)?;
                return Ok((message, false));
            }
        }

        let message = insert_message_on(&transaction, input)?;
        let (now, lease_until) = operation_timestamps(lease_for)?;
        let changed = transaction.execute(
            "UPDATE messages
             SET outbound_phase = 'created',
                 outbound_owner = ?1,
                 outbound_lease_until = ?2,
                 outbound_next_attempt_at = NULL
             WHERE id = ?3",
            params![owner, lease_until, message.id],
        )?;
        if changed != 1 {
            anyhow::bail!("failed to initialize outbound operation {}", message.id);
        }
        if let Some(key) = idempotency_key {
            transaction.execute(
                "INSERT INTO outbound_idempotency (key, request_hash, message_id, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    key,
                    compute_outbound_request_hash(
                        &message.phone_number,
                        &message.body,
                        message.source
                    ),
                    message.id,
                    now
                ],
            )?;
        }
        transaction.commit()?;
        Ok((message, true))
    }

    pub fn claim_due_outbound(
        &self,
        owner: &str,
        lease_for: Duration,
    ) -> Result<Option<(Message, String)>> {
        let mut conn = self.conn.lock().unwrap();
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (now, lease_until) = operation_timestamps(lease_for)?;
        let candidate = transaction
            .query_row(
                "SELECT id, outbound_phase
                 FROM messages
                 WHERE direction = 'outbound'
                   AND status = 'sending'
                   AND outbound_phase IN ('created', 'prepared', 'send_started', 'uncertain')
                   AND (outbound_next_attempt_at IS NULL
                        OR julianday(outbound_next_attempt_at) <= julianday(?1))
                   AND (outbound_owner IS NULL
                        OR outbound_lease_until IS NULL
                        OR julianday(outbound_lease_until) <= julianday(?1))
                 ORDER BY id
                 LIMIT 1",
                params![now],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        let Some((id, phase)) = candidate else {
            transaction.commit()?;
            return Ok(None);
        };
        let changed = transaction.execute(
            "UPDATE messages
             SET outbound_owner = ?1,
                 outbound_lease_until = ?2,
                 outbound_next_attempt_at = NULL,
                 updated_at = ?3
             WHERE id = ?4
               AND status = 'sending'
               AND (outbound_owner IS NULL
                    OR outbound_lease_until IS NULL
                    OR julianday(outbound_lease_until) <= julianday(?3))",
            params![owner, lease_until, now, id],
        )?;
        if changed != 1 {
            transaction.commit()?;
            return Ok(None);
        }
        let message = map_get(&transaction, id)?;
        transaction.commit()?;
        Ok(Some((message, phase)))
    }

    pub fn set_outbound_prepared(
        &self,
        id: i64,
        owner: &str,
        modem_sms_path: &str,
        lease_for: Duration,
    ) -> Result<(Message, bool)> {
        self.transition_outbound(PhaseTransition {
            id,
            owner,
            expected_phase: "created",
            next_phase: "prepared",
            modem_sms_path: Some(modem_sms_path),
            error: None,
            next_attempt_at: None,
            lease_for,
        })
    }

    pub fn begin_outbound_send(
        &self,
        id: i64,
        owner: &str,
        lease_for: Duration,
    ) -> Result<(Message, bool)> {
        self.transition_outbound(PhaseTransition {
            id,
            owner,
            expected_phase: "prepared",
            next_phase: "send_started",
            modem_sms_path: None,
            error: None,
            next_attempt_at: None,
            lease_for,
        })
    }

    pub fn defer_outbound(
        &self,
        id: i64,
        owner: &str,
        phase: &str,
        error: Option<&str>,
        retry_after: Option<Duration>,
    ) -> Result<(Message, bool)> {
        let conn = self.conn.lock().unwrap();
        let now = OffsetDateTime::now_utc();
        let format = &time::format_description::well_known::Rfc3339;
        let now_string = now.format(format)?;
        let next_attempt_at = retry_after
            .map(|duration| -> Result<String> {
                Ok((now + time::Duration::try_from(duration)?).format(format)?)
            })
            .transpose()?;
        let changed = conn.execute(
            "UPDATE messages
             SET outbound_phase = ?1,
                 outbound_owner = NULL,
                 outbound_lease_until = NULL,
                 outbound_next_attempt_at = ?2,
                 error = ?3,
                 updated_at = ?4
             WHERE id = ?5
               AND direction = 'outbound'
               AND status = 'sending'
               AND outbound_owner = ?6",
            params![phase, next_attempt_at, error, now_string, id, owner],
        )?;
        Ok((map_get(&conn, id)?, changed == 1))
    }

    pub fn finish_claimed_outbound(
        &self,
        id: i64,
        owner: &str,
        status: MessageStatus,
        error: Option<&str>,
    ) -> Result<(Message, bool)> {
        let mut conn = self.conn.lock().unwrap();
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = now_string();
        let changed = transaction.execute(
            "UPDATE messages
             SET status = ?1,
                 error = ?2,
                 outbound_phase = 'complete',
                 outbound_next_attempt_at = NULL,
                 updated_at = ?3
             WHERE id = ?4
               AND direction = 'outbound'
               AND status = 'sending'
               AND outbound_owner = ?5",
            params![status_to_str(status), error, now, id, owner],
        )?;
        let message = map_get(&transaction, id)?;
        transaction.commit()?;
        Ok((message, changed == 1))
    }

    pub fn claim_pending_outbound_event(
        &self,
        owner: &str,
        lease_for: Duration,
    ) -> Result<Option<Message>> {
        let mut conn = self.conn.lock().unwrap();
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (now, lease_until) = operation_timestamps(lease_for)?;
        let id = transaction
            .query_row(
                "SELECT id FROM messages
                 WHERE direction = 'outbound'
                   AND status IN ('sent', 'failed')
                   AND outbound_phase = 'complete'
                   AND (outbound_owner IS NULL
                        OR outbound_lease_until IS NULL
                        OR julianday(outbound_lease_until) <= julianday(?1))
                 ORDER BY id
                 LIMIT 1",
                params![now],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        let Some(id) = id else {
            transaction.commit()?;
            return Ok(None);
        };
        let changed = transaction.execute(
            "UPDATE messages
             SET outbound_owner = ?1, outbound_lease_until = ?2
             WHERE id = ?3
               AND outbound_phase = 'complete'
               AND (outbound_owner IS NULL
                    OR outbound_lease_until IS NULL
                    OR julianday(outbound_lease_until) <= julianday(?4))",
            params![owner, lease_until, id, now],
        )?;
        let message = if changed == 1 {
            Some(map_get(&transaction, id)?)
        } else {
            None
        };
        transaction.commit()?;
        Ok(message)
    }

    pub fn acknowledge_outbound_event(&self, id: i64, owner: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        Ok(conn.execute(
            "UPDATE messages
             SET outbound_phase = NULL,
                 outbound_owner = NULL,
                 outbound_lease_until = NULL
             WHERE id = ?1
               AND outbound_phase = 'complete'
               AND outbound_owner = ?2",
            params![id, owner],
        )? == 1)
    }

    pub fn renew_outbound_lease(&self, id: i64, owner: &str, lease_for: Duration) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let (_, lease_until) = operation_timestamps(lease_for)?;
        Ok(conn.execute(
            "UPDATE messages
             SET outbound_lease_until = ?1
             WHERE id = ?2
               AND direction = 'outbound'
               AND status = 'sending'
               AND outbound_owner = ?3",
            params![lease_until, id, owner],
        )? == 1)
    }

    pub fn has_pending_outbound(&self) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        Ok(conn.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM messages
                WHERE direction = 'outbound' AND status = 'sending'
             )",
            [],
            |row| row.get(0),
        )?)
    }

    #[cfg(test)]
    pub fn expire_outbound_lease(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE messages
             SET outbound_lease_until = '1970-01-01T00:00:00Z'
             WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    fn transition_outbound(&self, transition: PhaseTransition<'_>) -> Result<(Message, bool)> {
        let conn = self.conn.lock().unwrap();
        let (now, lease_until) = operation_timestamps(transition.lease_for)?;
        let changed = conn.execute(
            "UPDATE messages
             SET outbound_phase = ?1,
                 modem_sms_path = COALESCE(?2, modem_sms_path),
                 error = ?3,
                 outbound_next_attempt_at = ?4,
                 outbound_lease_until = ?5,
                 updated_at = ?6
             WHERE id = ?7
               AND direction = 'outbound'
               AND status = 'sending'
               AND outbound_owner = ?8
               AND outbound_phase = ?9",
            params![
                transition.next_phase,
                transition.modem_sms_path,
                transition.error,
                transition.next_attempt_at,
                lease_until,
                now,
                transition.id,
                transition.owner,
                transition.expected_phase
            ],
        )?;
        Ok((map_get(&conn, transition.id)?, changed == 1))
    }

    #[cfg(test)]
    pub fn insert_message(&self, input: NewMessage) -> Result<Message> {
        let conn = self.conn.lock().unwrap();
        insert_message_on(&conn, input)
    }

    #[cfg(test)]
    pub fn insert_message_with_deliveries(
        &self,
        input: NewMessage,
        profile_keys: &[String],
    ) -> Result<Message> {
        let mut conn = self.conn.lock().unwrap();
        let transaction = conn.transaction()?;
        let message = insert_message_on(&transaction, input)?;
        let now = now_string();
        for key in profile_keys {
            transaction.execute(
                "INSERT INTO forward_deliveries (message_id, profile_key, state, created_at, updated_at)
                 VALUES (?1, ?2, 'pending', ?3, ?3)",
                params![message.id, key, now],
            )?;
        }
        transaction.commit()?;
        Ok(message)
    }

    pub fn for_each_export_message<F>(&self, filter: &MessageFilter, mut visit: F) -> Result<()>
    where
        F: FnMut(Message) -> Result<bool>,
    {
        if let Some(path) = &self.path {
            let conn = Connection::open_with_flags(
                path.as_ref(),
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                    | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )?;
            conn.busy_timeout(std::time::Duration::from_secs(5))?;
            for_each_export_on(&conn, filter, &mut visit)
        } else {
            let conn = self.conn.lock().unwrap();
            for_each_export_on(&conn, filter, &mut visit)
        }
    }

    #[cfg(test)]
    pub fn count_messages(&self) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        Ok(conn.query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))?)
    }

    #[cfg(test)]
    pub fn get_message(&self, id: i64) -> Result<Message> {
        let conn = self.conn.lock().unwrap();
        map_get(&conn, id)
    }

    pub fn get_message_optional(&self, id: i64) -> Result<Option<Message>> {
        let conn = self.conn.lock().unwrap();
        map_find(&conn, id)
    }

    pub fn list_messages(&self, filter: &MessageFilter) -> Result<Vec<Message>> {
        self.query_messages(filter, true)
    }

    fn query_messages(&self, filter: &MessageFilter, apply_limit: bool) -> Result<Vec<Message>> {
        let conn = self.conn.lock().unwrap();
        let cursor = resolve_message_cursor(&conn, filter.before.as_ref())?;
        let (sql, values) = build_message_query(&conn, filter, cursor.as_ref(), apply_limit)?;
        let mut statement = conn.prepare(&sql)?;
        let rows = statement.query_map(params_from_iter(values.iter()), row_to_message)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn mark_read(&self, id: i64) -> Result<Message> {
        let now = now_string();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE messages SET read_at = ?1, updated_at = ?1 WHERE id = ?2",
            params![now, id],
        )?;
        map_get(&conn, id)
    }

    pub fn mark_unread(&self, id: i64) -> Result<Message> {
        let now = now_string();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE messages SET read_at = NULL, updated_at = ?1 WHERE id = ?2",
            params![now, id],
        )?;
        map_get(&conn, id)
    }

    pub fn mark_conversation_read(&self, phone_number: &str) -> Result<i64> {
        let now = now_string();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE messages SET read_at = ?1, updated_at = ?1 WHERE phone_number = ?2 AND direction = 'inbound' AND read_at IS NULL",
            params![now, phone_number],
        )?;
        Ok(conn.changes() as i64)
    }

    pub fn delete_messages(&self, ids: &[i64]) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let transaction = conn.transaction()?;
        for id in ids {
            let sending_phase = transaction
                .query_row(
                    "SELECT status, outbound_phase FROM messages WHERE id = ?1",
                    params![id],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
                )
                .optional()?;
            if matches!(
                sending_phase,
                Some((ref status, ref phase))
                    if status == "sending" && phase.as_deref() != Some("unknown")
            ) {
                anyhow::bail!("message {id} cannot be deleted while sending");
            }
        }
        for id in ids {
            transaction.execute("DELETE FROM messages WHERE id = ?1", params![id])?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn list_conversations(&self) -> Result<Vec<ConversationSummary>> {
        let conn = self.conn.lock().unwrap();
        let mut statement = conn.prepare(
            "SELECT summaries.phone_number, summaries.total_count, summaries.unread_count,
                    messages.id, messages.direction, messages.phone_number, messages.body,
                    messages.timestamp, messages.status, messages.source, messages.modem_sms_path,
                    messages.read_at, messages.error, messages.created_at, messages.updated_at
             FROM conversation_summaries AS summaries
             INNER JOIN messages ON messages.id = summaries.last_message_id
             ORDER BY COALESCE(julianday(messages.timestamp), julianday(messages.created_at)) DESC,
                      messages.id DESC",
        )?;
        let rows = statement.query_map([], |row| {
            let phone_number: String = row.get(0)?;
            let total_count: i64 = row.get(1)?;
            let unread_count: i64 = row.get(2)?;
            let last_message = Message {
                id: row.get(3)?,
                direction: str_to_direction(&row.get::<_, String>(4)?, 4)?,
                phone_number: row.get(5)?,
                body: row.get(6)?,
                timestamp: row.get(7)?,
                status: str_to_status(&row.get::<_, String>(8)?, 8)?,
                source: str_to_source(&row.get::<_, String>(9)?, 9)?,
                modem_sms_path: row.get(10)?,
                read_at: row.get(11)?,
                error: row.get(12)?,
                created_at: row.get(13)?,
                updated_at: row.get(14)?,
            };
            Ok(ConversationSummary {
                phone_number,
                last_message,
                unread_count,
                total_count,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn health_check(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.query_row("SELECT 1", [], |_| Ok(()))?;
        Ok(())
    }

    pub fn insert_inbound_message_with_deliveries(
        &self,
        input: NewMessage,
        profile_keys: &[String],
    ) -> Result<InboundInsertResult> {
        let dedupe_key = input.inbound_dedupe_key.as_deref();
        let mut conn = self.conn.lock().unwrap();
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        if let Some(key) = dedupe_key {
            if let Ok(existing) = transaction.query_row(
                "SELECT * FROM messages WHERE inbound_dedupe_key = ?1",
                params![key],
                row_to_message,
            ) {
                return Ok(InboundInsertResult::Duplicate(existing));
            }
        }

        let message = insert_message_on(&transaction, input)?;
        let now = now_string();
        for key in profile_keys {
            transaction.execute(
                "INSERT INTO forward_deliveries (message_id, profile_key, state, created_at, updated_at)
                 VALUES (?1, ?2, 'pending', ?3, ?3)",
                params![message.id, key, now],
            )?;
        }
        transaction.commit()?;
        Ok(InboundInsertResult::Inserted(message))
    }
}

fn map_get(conn: &Connection, id: i64) -> Result<Message> {
    map_find(conn, id)?.ok_or_else(|| anyhow::anyhow!("message {} not found", id))
}

fn map_find(conn: &Connection, id: i64) -> Result<Option<Message>> {
    conn.query_row(
        "SELECT * FROM messages WHERE id = ?1",
        params![id],
        row_to_message,
    )
    .optional()
    .map_err(Into::into)
}

fn insert_message_on(conn: &Connection, input: NewMessage) -> Result<Message> {
    let now = now_string();
    conn.execute(
        "INSERT INTO messages (direction, phone_number, body, timestamp, status, source, modem_sms_path, read_at, error, created_at, updated_at, inbound_dedupe_key)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            direction_to_str(input.direction),
            input.phone_number,
            input.body,
            input.timestamp,
            status_to_str(input.status),
            source_to_str(input.source),
            input.modem_sms_path,
            input.read_at,
            input.error,
            now,
            now,
            input.inbound_dedupe_key,
        ],
    )?;
    map_get(conn, conn.last_insert_rowid())
}

pub(super) enum ResolvedMessageCursor {
    Timeline { sort_key: f64, id: i64 },
}

pub(super) fn resolve_message_cursor(
    conn: &Connection,
    cursor: Option<&MessageCursor>,
) -> Result<Option<ResolvedMessageCursor>> {
    let Some(cursor) = cursor else {
        return Ok(None);
    };
    let (provided_timestamp, id) = match cursor {
        MessageCursor::Timeline { timestamp, id } => (Some(timestamp.as_str()), *id),
        MessageCursor::LegacyId(id) => (None, *id),
    };
    let sort_key: Option<f64> = conn.query_row(
        "SELECT COALESCE(julianday(?1), (SELECT COALESCE(julianday(timestamp), julianday(created_at)) FROM messages WHERE id = ?2))",
        params![provided_timestamp, id],
        |row| row.get(0),
    )?;
    let sort_key = sort_key.ok_or(InvalidMessageCursor)?;
    Ok(Some(ResolvedMessageCursor::Timeline { sort_key, id }))
}

pub(super) fn build_message_query(
    conn: &Connection,
    filter: &MessageFilter,
    cursor: Option<&ResolvedMessageCursor>,
    apply_limit: bool,
) -> Result<(String, Vec<Value>)> {
    let limit = filter.limit.unwrap_or(10).min(500);
    let mut sql = "SELECT * FROM messages WHERE 1=1".to_string();
    let mut values = Vec::new();
    match cursor {
        Some(ResolvedMessageCursor::Timeline { sort_key, id }) => {
            sql.push_str(
                " AND COALESCE(julianday(timestamp), julianday(created_at)) <= ? AND (COALESCE(julianday(timestamp), julianday(created_at)) < ? OR (COALESCE(julianday(timestamp), julianday(created_at)) = ? AND id < ?))",
            );
            values.push(Value::Real(*sort_key));
            values.push(Value::Real(*sort_key));
            values.push(Value::Real(*sort_key));
            values.push(Value::Integer(*id));
        }
        None => {}
    }
    if let Some(phone) = &filter.phone_number {
        sql.push_str(" AND phone_number = ?");
        values.push(Value::Text(phone.clone()));
    }
    if let Some(from) = &filter.from {
        sql.push_str(" AND COALESCE(julianday(timestamp), julianday(created_at)) >= ?");
        values.push(julian_day_value(conn, from)?);
    }
    if let Some(to) = &filter.to {
        sql.push_str(" AND COALESCE(julianday(timestamp), julianday(created_at)) <= ?");
        values.push(julian_day_value(conn, to)?);
    }
    if let Some(q) = &filter.q {
        sql.push_str(" AND (phone_number LIKE ? OR body LIKE ?)");
        let pattern = format!("%{}%", q);
        values.push(Value::Text(pattern.clone()));
        values.push(Value::Text(pattern));
    }
    if let Some(direction) = filter.direction {
        sql.push_str(" AND direction = ?");
        values.push(Value::Text(direction_to_str(direction).to_string()));
    }
    if let Some(status) = filter.status {
        sql.push_str(" AND status = ?");
        values.push(Value::Text(status_to_str(status).to_string()));
    }
    if let Some(unread) = filter.unread {
        sql.push_str(if unread {
            " AND read_at IS NULL"
        } else {
            " AND read_at IS NOT NULL"
        });
    }
    sql.push_str(" ORDER BY COALESCE(julianday(timestamp), julianday(created_at)) DESC, id DESC");
    if apply_limit {
        sql.push_str(" LIMIT ");
        sql.push_str(&limit.to_string());
    }
    Ok((sql, values))
}

fn julian_day_value(conn: &Connection, timestamp: &str) -> Result<Value> {
    let value: Option<f64> =
        conn.query_row("SELECT julianday(?1)", params![timestamp], |row| row.get(0))?;
    Ok(value.map(Value::Real).unwrap_or(Value::Null))
}

fn for_each_export_on<F>(conn: &Connection, filter: &MessageFilter, visit: &mut F) -> Result<()>
where
    F: FnMut(Message) -> Result<bool>,
{
    let cursor = resolve_message_cursor(conn, filter.before.as_ref())?;
    let (sql, values) = build_message_query(conn, filter, cursor.as_ref(), false)?;
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map(params_from_iter(values.iter()), row_to_message)?;
    for row in rows {
        if !visit(row?)? {
            break;
        }
    }
    Ok(())
}
