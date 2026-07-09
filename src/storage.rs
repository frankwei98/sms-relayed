use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use rusqlite::{params, params_from_iter, Connection, Row};
use time::OffsetDateTime;

use crate::message::{
    ConversationSummary, Message, MessageDirection, MessageFilter, MessageSource, MessageStatus,
};

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
            CREATE INDEX IF NOT EXISTS idx_messages_read_at ON messages(read_at);",
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
        let messages = self.list_messages(&MessageFilter {
            limit: Some(500),
            ..MessageFilter::default()
        })?;
        let mut by_phone: std::collections::BTreeMap<String, Vec<Message>> =
            std::collections::BTreeMap::new();
        for msg in messages {
            by_phone
                .entry(msg.phone_number.clone())
                .or_default()
                .push(msg);
        }
        let mut out: Vec<ConversationSummary> = by_phone
            .into_iter()
            .filter_map(|(phone_number, rows)| {
                let last_message = rows.first()?.clone();
                let unread_count = rows
                    .iter()
                    .filter(|m| m.direction == MessageDirection::Inbound && m.read_at.is_none())
                    .count() as i64;
                let total_count = rows.len() as i64;
                Some(ConversationSummary {
                    phone_number,
                    last_message,
                    unread_count,
                    total_count,
                })
            })
            .collect();
        out.sort_by(|a, b| {
            b.last_message
                .timestamp
                .cmp(&a.last_message.timestamp)
                .then(b.last_message.id.cmp(&a.last_message.id))
        });
        Ok(out)
    }

    pub fn health_check(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.query_row("SELECT 1", [], |_| Ok(()))?;
        Ok(())
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
}
