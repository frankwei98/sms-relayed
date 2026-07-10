use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use rusqlite::{params, params_from_iter, Connection, Row};
use time::OffsetDateTime;

use crate::message::{
    ConversationSummary, Message, MessageDirection, MessageFilter, MessageSource, MessageStatus,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryRow {
    pub id: i64,
    pub message_id: i64,
    pub profile_key: String,
    pub state: String,
    pub attempt_count: i64,
    pub next_attempt_at: Option<String>,
    pub lease_at: Option<String>,
    pub last_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone)]
pub struct MessageStore {
    conn: Arc<Mutex<Connection>>,
}

pub struct NewMessage {
    pub direction: MessageDirection,
    pub phone_number: String,
    pub body: String,
    pub timestamp: String,
    pub status: MessageStatus,
    pub source: MessageSource,
    pub modem_sms_path: Option<String>,
    pub read_at: Option<String>,
    pub error: Option<String>,
}

impl NewMessage {
    pub fn inbound(phone_number: &str, body: &str) -> Self {
        Self {
            direction: MessageDirection::Inbound,
            phone_number: phone_number.to_string(),
            body: body.to_string(),
            timestamp: now_string(),
            status: MessageStatus::Received,
            source: MessageSource::Modem,
            modem_sms_path: None,
            read_at: None,
            error: None,
        }
    }
}

impl MessageStore {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
        }
        let conn = Connection::open(path)
            .with_context(|| format!("failed to open sqlite database {}", path.display()))?;
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        store.migrate()?;
        Ok(store)
    }

    pub fn open_in_memory() -> Result<Self> {
        let store = Self {
            conn: Arc::new(Mutex::new(Connection::open_in_memory()?)),
        };
        store.migrate()?;
        Ok(store)
    }

    pub fn migrate(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                direction TEXT NOT NULL,
                phone_number TEXT NOT NULL,
                body TEXT NOT NULL,
                timestamp TEXT NOT NULL,
                status TEXT NOT NULL,
                source TEXT NOT NULL,
                modem_sms_path TEXT NULL,
                read_at TEXT NULL,
                error TEXT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_messages_phone_number ON messages(phone_number);
            CREATE INDEX IF NOT EXISTS idx_messages_timestamp ON messages(timestamp);
            CREATE INDEX IF NOT EXISTS idx_messages_direction ON messages(direction);
            CREATE INDEX IF NOT EXISTS idx_messages_status ON messages(status);
            CREATE INDEX IF NOT EXISTS idx_messages_read_at ON messages(read_at);
            CREATE TABLE IF NOT EXISTS meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS forward_deliveries (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                message_id INTEGER NOT NULL REFERENCES messages(id),
                profile_key TEXT NOT NULL,
                state TEXT NOT NULL DEFAULT 'pending',
                attempt_count INTEGER NOT NULL DEFAULT 0,
                next_attempt_at TEXT,
                lease_at TEXT,
                last_error TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                UNIQUE(message_id, profile_key)
            );
            CREATE INDEX IF NOT EXISTS idx_deliveries_state ON forward_deliveries(state);
            CREATE INDEX IF NOT EXISTS idx_deliveries_next_attempt ON forward_deliveries(next_attempt_at);",
        )?;
        Ok(())
    }

    pub fn get_meta(&self, key: &str) -> Option<String> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT value FROM meta WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .ok()
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn insert_message(&self, input: NewMessage) -> Result<Message> {
        let now = now_string();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO messages (direction, phone_number, body, timestamp, status, source, modem_sms_path, read_at, error, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
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
                now
            ],
        )?;
        let id = conn.last_insert_rowid();
        map_get(&conn, id)
    }

    pub fn get_message(&self, id: i64) -> Result<Message> {
        let conn = self.conn.lock().unwrap();
        map_get(&conn, id)
    }

    pub fn list_messages(&self, filter: &MessageFilter) -> Result<Vec<Message>> {
        self.query_messages(filter, true)
    }

    pub fn export_messages(&self, filter: &MessageFilter) -> Result<Vec<Message>> {
        self.query_messages(filter, false)
    }

    fn query_messages(&self, filter: &MessageFilter, apply_limit: bool) -> Result<Vec<Message>> {
        let conn = self.conn.lock().unwrap();
        let limit = filter.limit.unwrap_or(50).min(500);
        let mut sql = "SELECT * FROM messages WHERE 1=1".to_string();
        let mut values: Vec<String> = Vec::new();
        if let Some(before_id) = filter.before_id {
            sql.push_str(" AND id < ");
            sql.push_str(&before_id.to_string());
        }
        if let Some(phone) = &filter.phone_number {
            sql.push_str(" AND phone_number = ?");
            values.push(phone.clone());
        }
        if let Some(from) = &filter.from {
            sql.push_str(" AND timestamp >= ?");
            values.push(from.clone());
        }
        if let Some(to) = &filter.to {
            sql.push_str(" AND timestamp <= ?");
            values.push(to.clone());
        }
        if let Some(q) = &filter.q {
            sql.push_str(" AND (phone_number LIKE ? OR body LIKE ?)");
            values.push(format!("%{}%", q));
            values.push(format!("%{}%", q));
        }
        if let Some(direction) = filter.direction {
            sql.push_str(" AND direction = ?");
            values.push(direction_to_str(direction).to_string());
        }
        if let Some(status) = filter.status {
            sql.push_str(" AND status = ?");
            values.push(status_to_str(status).to_string());
        }
        if let Some(unread) = filter.unread {
            if unread {
                sql.push_str(" AND read_at IS NULL");
            } else {
                sql.push_str(" AND read_at IS NOT NULL");
            }
        }
        sql.push_str(" ORDER BY id DESC");
        if apply_limit {
            sql.push_str(" LIMIT ");
            sql.push_str(&limit.to_string());
        }
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(values.iter()), row_to_message)?;
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
        let conn = self.conn.lock().unwrap();
        for id in ids {
            conn.execute("DELETE FROM messages WHERE id = ?1", params![id])?;
        }
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

    pub fn list_conversations(&self) -> Result<Vec<ConversationSummary>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "WITH agg AS (
                SELECT phone_number,
                       COUNT(*) as total_count,
                       SUM(CASE WHEN direction = 'inbound' AND read_at IS NULL THEN 1 ELSE 0 END) as unread_count,
                       MAX(id) as last_id
                FROM messages
                GROUP BY phone_number
            )
            SELECT agg.phone_number, agg.total_count, agg.unread_count,
                   m.id, m.direction, m.phone_number, m.body, m.timestamp,
                   m.status, m.source, m.modem_sms_path, m.read_at, m.error,
                   m.created_at, m.updated_at
            FROM agg
            JOIN messages m ON m.id = agg.last_id
            ORDER BY m.id DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            let phone_number: String = row.get(0)?;
            let total_count: i64 = row.get(1)?;
            let unread_count: i64 = row.get(2)?;
            let last_message = Message {
                id: row.get(3)?,
                direction: str_to_direction(&row.get::<_, String>(4)?),
                phone_number: row.get(5)?,
                body: row.get(6)?,
                timestamp: row.get(7)?,
                status: str_to_status(&row.get::<_, String>(8)?),
                source: str_to_source(&row.get::<_, String>(9)?),
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

    pub fn insert_deliveries(&self, message_id: i64, profile_keys: &[String]) -> Result<()> {
        let now = now_string();
        let conn = self.conn.lock().unwrap();
        for key in profile_keys {
            conn.execute(
                "INSERT OR IGNORE INTO forward_deliveries (message_id, profile_key, state, created_at, updated_at)
                 VALUES (?1, ?2, 'pending', ?3, ?3)",
                params![message_id, key, now],
            )?;
        }
        Ok(())
    }

    pub fn claim_due_deliveries(
        &self,
        batch_size: u32,
        lease_until: &str,
    ) -> Result<Vec<DeliveryRow>> {
        let now = now_string();
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, message_id, profile_key, state, attempt_count, next_attempt_at, lease_at, last_error, created_at, updated_at
             FROM forward_deliveries
             WHERE state IN ('pending', 'retry_wait')
               AND (next_attempt_at IS NULL OR next_attempt_at <= ?1)
             ORDER BY id ASC
             LIMIT ?2",
        )?;
        let rows: Vec<DeliveryRow> = stmt
            .query_map(params![now, batch_size], row_to_delivery)?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        for row in &rows {
            conn.execute(
                "UPDATE forward_deliveries SET state = 'in_flight', lease_at = ?1, updated_at = ?1 WHERE id = ?2 AND state IN ('pending', 'retry_wait')",
                params![lease_until, row.id],
            )?;
        }

        // Re-read rows that were successfully claimed
        let ids: Vec<String> = rows.iter().map(|r| r.id.to_string()).collect();
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders: String = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT id, message_id, profile_key, state, attempt_count, next_attempt_at, lease_at, last_error, created_at, updated_at
             FROM forward_deliveries WHERE id IN ({}) AND state = 'in_flight'",
            placeholders
        );
        let mut stmt2 = conn.prepare(&sql)?;
        let params2: Vec<&dyn rusqlite::types::ToSql> = ids
            .iter()
            .map(|s| s as &dyn rusqlite::types::ToSql)
            .collect();
        let claimed = stmt2
            .query_map(params2.as_slice(), row_to_delivery)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(claimed)
    }

    pub fn complete_delivery(
        &self,
        id: i64,
        state: &str,
        error: Option<&str>,
        attempt_count: i64,
        next_attempt_at: Option<&str>,
    ) -> Result<()> {
        let now = now_string();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE forward_deliveries SET state = ?1, last_error = ?2, attempt_count = ?3, next_attempt_at = ?4, lease_at = NULL, updated_at = ?5 WHERE id = ?6",
            params![state, error, attempt_count, next_attempt_at, now, id],
        )?;
        Ok(())
    }

    pub fn recover_expired_leases(&self, before: &str) -> Result<usize> {
        let now = now_string();
        let conn = self.conn.lock().unwrap();
        let count = conn.execute(
            "UPDATE forward_deliveries SET state = 'retry_wait', lease_at = NULL, next_attempt_at = ?1, updated_at = ?1
             WHERE state = 'in_flight' AND lease_at IS NOT NULL AND lease_at < ?2",
            params![now, before],
        )?;
        Ok(count)
    }

    #[allow(dead_code)]
    pub fn run_retention(&self, max_age_days: u64, batch_size: u32) -> Result<usize> {
        let cutoff = (OffsetDateTime::now_utc() - time::Duration::days(max_age_days as i64))
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default();
        let conn = self.conn.lock().unwrap();
        // Find message IDs eligible for deletion: terminal status, old, and no non-terminal deliveries
        let mut stmt = conn.prepare(
            "SELECT m.id FROM messages m
             WHERE m.timestamp < ?1
               AND m.status IN ('received', 'sent', 'failed')
               AND NOT EXISTS (
                   SELECT 1 FROM forward_deliveries d
                   WHERE d.message_id = m.id
                     AND d.state IN ('pending', 'in_flight', 'retry_wait')
               )
             LIMIT ?2",
        )?;
        let ids: Vec<i64> = stmt
            .query_map(params![cutoff, batch_size], |row| row.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let count = ids.len();
        for id in &ids {
            conn.execute("DELETE FROM messages WHERE id = ?1", params![id])?;
        }
        Ok(count)
    }

    pub fn get_delivery_count_for_message(
        &self,
        message_id: i64,
        non_terminal_states: &[&str],
    ) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        let placeholders: Vec<String> = non_terminal_states
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 2))
            .collect();
        let sql = format!(
            "SELECT COUNT(*) FROM forward_deliveries
             WHERE message_id = ?1 AND state IN ({})",
            placeholders.join(",")
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(message_id)];
        for s in non_terminal_states {
            params_vec.push(Box::new(s.to_string()));
        }
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params_vec.iter().map(|p| p.as_ref()).collect();
        let count: i64 = stmt.query_row(param_refs.as_slice(), |row| row.get(0))?;
        Ok(count)
    }

    pub fn export_messages_csv(&self, filter: &MessageFilter) -> Result<String> {
        let messages = self.export_messages(filter)?;
        let mut writer = csv::WriterBuilder::new()
            .terminator(csv::Terminator::Any(b'\n'))
            .from_writer(Vec::new());
        writer.write_record([
            "id",
            "direction",
            "phone_number",
            "body",
            "timestamp",
            "status",
            "source",
            "read_at",
            "error",
            "created_at",
            "updated_at",
        ])?;
        for msg in messages {
            writer.write_record([
                msg.id.to_string(),
                direction_to_str(msg.direction).to_string(),
                msg.phone_number,
                msg.body,
                msg.timestamp,
                status_to_str(msg.status).to_string(),
                source_to_str(msg.source).to_string(),
                msg.read_at.unwrap_or_default(),
                msg.error.unwrap_or_default(),
                msg.created_at,
                msg.updated_at,
            ])?;
        }
        let bytes = writer.into_inner()?;
        Ok(String::from_utf8(bytes).unwrap_or_default())
    }
}

fn map_get(conn: &Connection, id: i64) -> Result<Message> {
    let mut stmt = conn.prepare("SELECT * FROM messages WHERE id = ?1")?;
    let mut rows = stmt.query_map(params![id], row_to_message)?;
    match rows.next() {
        Some(row) => row.map_err(Into::into),
        None => anyhow::bail!("message {} not found", id),
    }
}

fn row_to_message(row: &Row) -> rusqlite::Result<Message> {
    Ok(Message {
        id: row.get(0)?,
        direction: str_to_direction(&row.get::<_, String>(1)?),
        phone_number: row.get(2)?,
        body: row.get(3)?,
        timestamp: row.get(4)?,
        status: str_to_status(&row.get::<_, String>(5)?),
        source: str_to_source(&row.get::<_, String>(6)?),
        modem_sms_path: row.get(7)?,
        read_at: row.get(8)?,
        error: row.get(9)?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
    })
}

fn direction_to_str(direction: MessageDirection) -> &'static str {
    match direction {
        MessageDirection::Inbound => "inbound",
        MessageDirection::Outbound => "outbound",
    }
}

fn status_to_str(status: MessageStatus) -> &'static str {
    match status {
        MessageStatus::Received => "received",
        MessageStatus::Sending => "sending",
        MessageStatus::Sent => "sent",
        MessageStatus::Failed => "failed",
    }
}

fn source_to_str(source: MessageSource) -> &'static str {
    match source {
        MessageSource::Modem => "modem",
        MessageSource::Web => "web",
        MessageSource::Cli => "cli",
    }
}

fn str_to_direction(s: &str) -> MessageDirection {
    match s {
        "outbound" => MessageDirection::Outbound,
        _ => MessageDirection::Inbound,
    }
}

fn str_to_status(s: &str) -> MessageStatus {
    match s {
        "sending" => MessageStatus::Sending,
        "sent" => MessageStatus::Sent,
        "failed" => MessageStatus::Failed,
        _ => MessageStatus::Received,
    }
}

fn str_to_source(s: &str) -> MessageSource {
    match s {
        "web" => MessageSource::Web,
        "cli" => MessageSource::Cli,
        _ => MessageSource::Modem,
    }
}

fn row_to_delivery(row: &Row) -> rusqlite::Result<DeliveryRow> {
    Ok(DeliveryRow {
        id: row.get(0)?,
        message_id: row.get(1)?,
        profile_key: row.get(2)?,
        state: row.get(3)?,
        attempt_count: row.get(4)?,
        next_attempt_at: row.get(5)?,
        lease_at: row.get(6)?,
        last_error: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

fn now_string() -> String {
    OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory_store() -> MessageStore {
        MessageStore::open_in_memory().unwrap()
    }

    #[test]
    fn inserts_and_lists_messages_newest_first() {
        let store = memory_store();
        let first = store
            .insert_message(NewMessage {
                direction: MessageDirection::Inbound,
                phone_number: "+15551234567".to_string(),
                body: "hello".to_string(),
                timestamp: "2026-07-08T12:00:00Z".to_string(),
                status: MessageStatus::Received,
                source: MessageSource::Modem,
                modem_sms_path: Some("/org/freedesktop/ModemManager1/SMS/1".to_string()),
                read_at: None,
                error: None,
            })
            .unwrap();
        let second = store
            .insert_message(NewMessage::inbound("+15550000000", "later"))
            .unwrap();

        let rows = store.list_messages(&MessageFilter::default()).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, second.id);
        assert_eq!(rows[1].id, first.id);
        assert_eq!(rows[1].body, "hello");
        assert_eq!(
            rows[1].modem_sms_path.as_deref(),
            Some("/org/freedesktop/ModemManager1/SMS/1")
        );
    }

    #[test]
    fn filters_search_unread_direction_status_and_phone() {
        let store = memory_store();
        store
            .insert_message(NewMessage::inbound("+1", "alpha code"))
            .unwrap();
        store
            .insert_message(NewMessage {
                direction: MessageDirection::Outbound,
                phone_number: "+1".to_string(),
                body: "alpha reply".to_string(),
                timestamp: "2026-07-08T12:01:00Z".to_string(),
                status: MessageStatus::Sent,
                source: MessageSource::Web,
                modem_sms_path: None,
                read_at: Some("2026-07-08T12:01:00Z".to_string()),
                error: None,
            })
            .unwrap();
        store
            .insert_message(NewMessage::inbound("+2", "beta"))
            .unwrap();

        let rows = store
            .list_messages(&MessageFilter {
                phone_number: Some("+1".to_string()),
                q: Some("alpha".to_string()),
                direction: Some(MessageDirection::Inbound),
                status: Some(MessageStatus::Received),
                unread: Some(true),
                ..MessageFilter::default()
            })
            .unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].phone_number, "+1");
        assert_eq!(rows[0].direction, MessageDirection::Inbound);
        assert_eq!(rows[0].status, MessageStatus::Received);
        assert!(rows[0].read_at.is_none());
    }

    #[test]
    fn marks_single_message_and_conversation_read_unread() {
        let store = memory_store();
        let one = store
            .insert_message(NewMessage::inbound("+1", "one"))
            .unwrap();
        let two = store
            .insert_message(NewMessage::inbound("+1", "two"))
            .unwrap();
        store
            .insert_message(NewMessage::inbound("+2", "other"))
            .unwrap();

        store.mark_read(one.id).unwrap();
        assert!(store.get_message(one.id).unwrap().read_at.is_some());
        store.mark_unread(one.id).unwrap();
        assert!(store.get_message(one.id).unwrap().read_at.is_none());

        let changed = store.mark_conversation_read("+1").unwrap();
        assert_eq!(changed, 2);
        assert!(store.get_message(one.id).unwrap().read_at.is_some());
        assert!(store.get_message(two.id).unwrap().read_at.is_some());

        let unread = store
            .list_messages(&MessageFilter {
                unread: Some(true),
                ..MessageFilter::default()
            })
            .unwrap();
        assert_eq!(unread.len(), 1);
        assert_eq!(unread[0].phone_number, "+2");
    }

    #[test]
    fn deletes_multiple_messages() {
        let store = memory_store();
        let one = store
            .insert_message(NewMessage::inbound("+1", "one"))
            .unwrap();
        let two = store
            .insert_message(NewMessage::inbound("+2", "two"))
            .unwrap();
        store.delete_messages(&[one.id, two.id]).unwrap();
        assert!(store
            .list_messages(&MessageFilter::default())
            .unwrap()
            .is_empty());
    }

    #[test]
    fn conversations_include_last_message_and_unread_counts() {
        let store = memory_store();
        store
            .insert_message(NewMessage::inbound("+1", "old unread"))
            .unwrap();
        let latest = store
            .insert_message(NewMessage::inbound("+1", "new unread"))
            .unwrap();
        let read = store
            .insert_message(NewMessage::inbound("+2", "read"))
            .unwrap();
        store.mark_read(read.id).unwrap();

        let conversations = store.list_conversations().unwrap();
        assert_eq!(conversations.len(), 2);
        assert_eq!(conversations[0].phone_number, "+2");
        assert_eq!(conversations[0].unread_count, 0);
        assert_eq!(conversations[1].phone_number, "+1");
        assert_eq!(conversations[1].last_message.id, latest.id);
        assert_eq!(conversations[1].unread_count, 2);
        assert_eq!(conversations[1].total_count, 2);
    }

    #[test]
    fn filters_by_timestamp_range() {
        let store = memory_store();
        store
            .insert_message(NewMessage {
                direction: MessageDirection::Inbound,
                phone_number: "+1".to_string(),
                body: "early".to_string(),
                timestamp: "2026-01-01T00:00:00Z".to_string(),
                status: MessageStatus::Received,
                source: MessageSource::Modem,
                modem_sms_path: None,
                read_at: None,
                error: None,
            })
            .unwrap();
        store
            .insert_message(NewMessage {
                direction: MessageDirection::Inbound,
                phone_number: "+2".to_string(),
                body: "middle".to_string(),
                timestamp: "2026-06-15T12:00:00Z".to_string(),
                status: MessageStatus::Received,
                source: MessageSource::Modem,
                modem_sms_path: None,
                read_at: None,
                error: None,
            })
            .unwrap();
        store
            .insert_message(NewMessage {
                direction: MessageDirection::Inbound,
                phone_number: "+3".to_string(),
                body: "late".to_string(),
                timestamp: "2026-12-31T23:59:59Z".to_string(),
                status: MessageStatus::Received,
                source: MessageSource::Modem,
                modem_sms_path: None,
                read_at: None,
                error: None,
            })
            .unwrap();

        let rows = store
            .list_messages(&MessageFilter {
                from: Some("2026-06-01T00:00:00Z".to_string()),
                to: Some("2026-07-01T00:00:00Z".to_string()),
                ..MessageFilter::default()
            })
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].body, "middle");
    }

    #[test]
    fn export_ignores_page_limit_and_uses_stable_csv_columns() {
        let store = memory_store();
        store
            .insert_message(NewMessage::inbound("+1", "alpha"))
            .unwrap();
        store
            .insert_message(NewMessage::inbound("+1", "alpha second"))
            .unwrap();

        let filter = MessageFilter {
            limit: Some(1),
            q: Some("alpha".to_string()),
            ..MessageFilter::default()
        };
        let rows = store.export_messages(&filter).unwrap();
        assert_eq!(rows.len(), 2);

        let csv = store.export_messages_csv(&filter).unwrap();
        assert!(csv.starts_with("id,direction,phone_number,body,timestamp,status,source,read_at,error,created_at,updated_at\n"));
        assert!(csv.contains("alpha second"));
    }

    #[test]
    fn conversations_include_all_phones_beyond_500_messages() {
        let store = memory_store();
        store
            .insert_message(NewMessage::inbound("+15550000002", "oldest message"))
            .unwrap();
        for i in 0..500 {
            store
                .insert_message(NewMessage {
                    direction: MessageDirection::Inbound,
                    phone_number: "+15550000001".to_string(),
                    body: format!("newer msg {}", i),
                    timestamp: format!("2026-07-08T{:02}:{:02}:00Z", i / 60, i % 60),
                    status: MessageStatus::Received,
                    source: MessageSource::Modem,
                    modem_sms_path: None,
                    read_at: None,
                    error: None,
                })
                .unwrap();
        }

        let conversations = store.list_conversations().unwrap();
        assert_eq!(conversations.len(), 2);
        let a = conversations
            .iter()
            .find(|c| c.phone_number == "+15550000001")
            .unwrap();
        assert_eq!(a.total_count, 500);
        let b = conversations
            .iter()
            .find(|c| c.phone_number == "+15550000002")
            .unwrap();
        assert_eq!(b.total_count, 1);
    }

    #[test]
    fn meta_read_write_roundtrip() {
        let store = memory_store();
        assert_eq!(store.get_meta("test_key"), None);
        store.set_meta("test_key", "hello").unwrap();
        assert_eq!(store.get_meta("test_key").as_deref(), Some("hello"));
        store.set_meta("test_key", "updated").unwrap();
        assert_eq!(store.get_meta("test_key").as_deref(), Some("updated"));
    }

    #[test]
    fn retention_deletes_old_terminal_messages() {
        let store = memory_store();
        store
            .insert_message(NewMessage {
                direction: MessageDirection::Inbound,
                phone_number: "+1".to_string(),
                body: "old".to_string(),
                timestamp: "2020-01-01T00:00:00Z".to_string(),
                status: MessageStatus::Received,
                source: MessageSource::Modem,
                modem_sms_path: None,
                read_at: None,
                error: None,
            })
            .unwrap();
        let deleted = store.run_retention(1, 100).unwrap();
        assert_eq!(deleted, 1);
    }

    #[test]
    fn retention_skips_messages_with_non_terminal_deliveries() {
        let store = memory_store();
        let msg = store
            .insert_message(NewMessage {
                direction: MessageDirection::Inbound,
                phone_number: "+1".to_string(),
                body: "old".to_string(),
                timestamp: "2020-01-01T00:00:00Z".to_string(),
                status: MessageStatus::Received,
                source: MessageSource::Modem,
                modem_sms_path: None,
                read_at: None,
                error: None,
            })
            .unwrap();
        store
            .insert_deliveries(msg.id, &["bark.test".to_string()])
            .unwrap();
        let deleted = store.run_retention(1, 100).unwrap();
        assert_eq!(deleted, 0);
    }
}
