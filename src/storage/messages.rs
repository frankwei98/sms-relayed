use anyhow::Result;
use rusqlite::{
    params, params_from_iter, types::Value, Connection, OptionalExtension, TransactionBehavior,
};

use crate::message::{ConversationSummary, Message, MessageCursor, MessageFilter, MessageStatus};

use super::codecs::{
    direction_to_str, now_string, row_to_message, source_to_str, status_to_str, str_to_direction,
    str_to_source, str_to_status,
};
use super::{InboundInsertResult, InvalidMessageCursor, MessageStore, NewMessage};

impl MessageStore {
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

    pub fn list_sending_outbound(&self) -> Result<Vec<Message>> {
        let conn = self.conn.lock().unwrap();
        let mut statement = conn.prepare(
            "SELECT * FROM messages
             WHERE direction = 'outbound' AND status = 'sending'
             ORDER BY id",
        )?;
        let rows = statement.query_map([], row_to_message)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
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
            let is_sending = transaction
                .query_row(
                    "SELECT status = 'sending' FROM messages WHERE id = ?1",
                    params![id],
                    |row| row.get::<_, bool>(0),
                )
                .optional()?
                .unwrap_or(false);
            if is_sending {
                anyhow::bail!("message {id} cannot be deleted while sending");
            }
        }
        for id in ids {
            transaction.execute("DELETE FROM messages WHERE id = ?1", params![id])?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn update_status(
        &self,
        id: i64,
        status: MessageStatus,
        error: Option<String>,
    ) -> Result<Message> {
        let now = now_string();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE messages SET status = ?1, error = ?2, updated_at = ?3 WHERE id = ?4",
            params![status_to_str(status), error, now, id],
        )?;
        map_get(&conn, id)
    }

    pub fn set_outbound_modem_sms_path(&self, id: i64, modem_sms_path: &str) -> Result<Message> {
        let now = now_string();
        let conn = self.conn.lock().unwrap();
        let changed = conn.execute(
            "UPDATE messages
             SET modem_sms_path = ?1, updated_at = ?2
             WHERE id = ?3 AND direction = 'outbound' AND status = 'sending'",
            params![modem_sms_path, now, id],
        )?;
        if changed != 1 {
            anyhow::bail!("sending outbound message {id} not found");
        }
        map_get(&conn, id)
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
