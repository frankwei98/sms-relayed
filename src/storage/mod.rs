use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
#[cfg(test)]
use rusqlite::params_from_iter;
use rusqlite::{params, Connection};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::message::{Message, MessageDirection, MessageSource, MessageStatus};
#[cfg(test)]
use crate::message::{MessageCursor, MessageFilter};

mod attempts;
mod codecs;
mod deliveries;
mod messages;
mod metadata;
mod migrations;
mod retention;

use codecs::now_string;
#[cfg(test)]
use messages::{build_message_query, resolve_message_cursor};

const CONVERSATION_SUMMARIES_BACKFILL_META_KEY: &str = "conversation_summaries_backfilled";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryRow {
    pub id: i64,
    pub message_id: i64,
    pub profile_key: String,
    pub state: DeliveryState,
    pub attempt_count: i64,
    pub next_attempt_at: Option<String>,
    pub lease_at: Option<String>,
    pub lease_token: Option<String>,
    pub last_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryState {
    Pending,
    InFlight,
    RetryWait,
    Succeeded,
    PermanentFailed,
}

impl DeliveryState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InFlight => "in_flight",
            Self::RetryWait => "retry_wait",
            Self::Succeeded => "succeeded",
            Self::PermanentFailed => "permanent_failed",
        }
    }
}

#[derive(Debug)]
pub struct InvalidMessageCursor;

impl std::fmt::Display for InvalidMessageCursor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("message cursor no longer references an existing message")
    }
}

impl std::error::Error for InvalidMessageCursor {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ForwardAttemptOutcome {
    Success,
    TransientFailure,
    PermanentFailure,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForwardAttemptSample {
    pub id: i64,
    pub profile_key: String,
    pub delivery_id: Option<i64>,
    pub attempt_number: i32,
    pub started_at: String,
    pub completed_at: String,
    pub latency_ms: i64,
    pub dispatch_delay_ms: Option<i64>,
    pub outcome: ForwardAttemptOutcome,
    pub error_code: Option<String>,
}

impl ForwardAttemptSample {
    pub fn is_retry(&self) -> bool {
        self.attempt_number > 1
    }
}

#[derive(Debug, Clone)]
pub struct NewForwardAttemptSample {
    pub profile_key: String,
    pub delivery_id: Option<i64>,
    pub attempt_number: i32,
    pub started_at: String,
    pub completed_at: String,
    pub latency_ms: i64,
    pub dispatch_delay_ms: i64,
    pub outcome: ForwardAttemptOutcome,
    pub error_code: Option<String>,
}

pub struct DeliveryCompletion<'a> {
    pub id: i64,
    pub state: DeliveryState,
    pub error: Option<&'a str>,
    pub attempt_count: i64,
    pub next_attempt_at: Option<&'a str>,
    pub lease_token: &'a str,
    pub sample: NewForwardAttemptSample,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InboundInsertResult {
    Inserted(Message),
    Duplicate(Message),
}

#[derive(Clone)]
pub struct MessageStore {
    conn: Arc<Mutex<Connection>>,
    path: Option<Arc<PathBuf>>,
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
    pub inbound_dedupe_key: Option<String>,
}

impl NewMessage {
    #[cfg(test)]
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
            inbound_dedupe_key: None,
        }
    }

    pub fn modem_inbound(
        phone_number: &str,
        body: &str,
        timestamp: &str,
        modem_sms_path: &str,
        modem_fingerprint: &str,
    ) -> Self {
        let dedup_key =
            compute_inbound_dedupe_key(modem_fingerprint, timestamp, phone_number, body);
        Self {
            direction: MessageDirection::Inbound,
            phone_number: phone_number.to_string(),
            body: body.to_string(),
            timestamp: timestamp.to_string(),
            status: MessageStatus::Received,
            source: MessageSource::Modem,
            modem_sms_path: Some(modem_sms_path.to_string()),
            read_at: None,
            error: None,
            inbound_dedupe_key: Some(dedup_key),
        }
    }
}

fn compute_inbound_dedupe_key(
    fingerprint: &str,
    timestamp: &str,
    phone: &str,
    body: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update([0u8]);
    hasher.update((fingerprint.len() as u32).to_be_bytes());
    hasher.update(fingerprint.as_bytes());
    hasher.update((timestamp.len() as u32).to_be_bytes());
    hasher.update(timestamp.as_bytes());
    hasher.update((phone.len() as u32).to_be_bytes());
    hasher.update(phone.as_bytes());
    hasher.update((body.len() as u32).to_be_bytes());
    hasher.update(body.as_bytes());
    hex_encode(hasher.finalize().as_slice())
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX_CHARS: &[u8] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX_CHARS[(b >> 4) as usize] as char);
        out.push(HEX_CHARS[(b & 0x0f) as usize] as char);
    }
    out
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
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
            path: Some(Arc::new(path.to_path_buf())),
        };
        store.migrate()?;
        Ok(store)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
            path: None,
        };
        store.migrate()?;
        Ok(store)
    }

    pub fn migrate(&self) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                direction TEXT NOT NULL CHECK (direction IN ('inbound', 'outbound')),
                phone_number TEXT NOT NULL,
                body TEXT NOT NULL,
                timestamp TEXT NOT NULL,
                status TEXT NOT NULL CHECK (status IN ('received', 'sending', 'sent', 'failed')),
                source TEXT NOT NULL CHECK (source IN ('modem', 'web', 'cli')),
                modem_sms_path TEXT NULL,
                read_at TEXT NULL,
                error TEXT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                inbound_dedupe_key TEXT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_messages_phone_number ON messages(phone_number);
            CREATE INDEX IF NOT EXISTS idx_messages_timestamp ON messages(timestamp);
            DROP INDEX IF EXISTS idx_messages_timeline;
            DROP INDEX IF EXISTS idx_messages_phone_timeline;
            CREATE INDEX IF NOT EXISTS idx_messages_timeline_v2 ON messages(COALESCE(julianday(timestamp), julianday(created_at)) DESC, id DESC);
            CREATE INDEX IF NOT EXISTS idx_messages_phone_timeline_v2 ON messages(phone_number, COALESCE(julianday(timestamp), julianday(created_at)) DESC, id DESC);
            CREATE INDEX IF NOT EXISTS idx_messages_direction ON messages(direction);
            CREATE INDEX IF NOT EXISTS idx_messages_status ON messages(status);
            CREATE INDEX IF NOT EXISTS idx_messages_read_at ON messages(read_at);
            CREATE TABLE IF NOT EXISTS conversation_summaries (
                phone_number TEXT PRIMARY KEY,
                total_count INTEGER NOT NULL,
                unread_count INTEGER NOT NULL,
                last_message_id INTEGER
            );
            CREATE TRIGGER IF NOT EXISTS messages_insert_conversation_summary
            AFTER INSERT ON messages
            BEGIN
                INSERT INTO conversation_summaries (
                    phone_number, total_count, unread_count, last_message_id
                ) VALUES (
                    NEW.phone_number,
                    1,
                    CASE WHEN NEW.direction = 'inbound' AND NEW.read_at IS NULL THEN 1 ELSE 0 END,
                    NEW.id
                )
                ON CONFLICT(phone_number) DO UPDATE SET
                    total_count = total_count + 1,
                    unread_count = unread_count +
                        CASE WHEN NEW.direction = 'inbound' AND NEW.read_at IS NULL THEN 1 ELSE 0 END,
                    last_message_id = CASE
                        WHEN COALESCE(julianday(NEW.timestamp), julianday(NEW.created_at)) >
                            (SELECT COALESCE(julianday(timestamp), julianday(created_at))
                             FROM messages
                             WHERE id = conversation_summaries.last_message_id)
                          OR (
                            COALESCE(julianday(NEW.timestamp), julianday(NEW.created_at)) =
                                (SELECT COALESCE(julianday(timestamp), julianday(created_at))
                                 FROM messages
                                 WHERE id = conversation_summaries.last_message_id)
                            AND NEW.id > conversation_summaries.last_message_id
                          )
                        THEN NEW.id
                        ELSE conversation_summaries.last_message_id
                    END;
            END;
            CREATE TRIGGER IF NOT EXISTS messages_update_read_conversation_summary
            AFTER UPDATE OF read_at ON messages
            WHEN OLD.read_at IS NOT NEW.read_at
            BEGIN
                UPDATE conversation_summaries
                SET unread_count = unread_count
                    - CASE WHEN OLD.direction = 'inbound' AND OLD.read_at IS NULL THEN 1 ELSE 0 END
                    + CASE WHEN NEW.direction = 'inbound' AND NEW.read_at IS NULL THEN 1 ELSE 0 END
                WHERE phone_number = NEW.phone_number;
            END;
            CREATE TRIGGER IF NOT EXISTS messages_delete_conversation_summary
            AFTER DELETE ON messages
            BEGIN
                UPDATE conversation_summaries
                SET total_count = total_count - 1,
                    unread_count = unread_count
                        - CASE WHEN OLD.direction = 'inbound' AND OLD.read_at IS NULL THEN 1 ELSE 0 END,
                    last_message_id = CASE
                        WHEN last_message_id = OLD.id THEN (
                            SELECT id FROM messages
                            WHERE phone_number = OLD.phone_number
                            ORDER BY COALESCE(julianday(timestamp), julianday(created_at)) DESC, id DESC
                            LIMIT 1
                        )
                        ELSE last_message_id
                    END
                WHERE phone_number = OLD.phone_number;
                DELETE FROM conversation_summaries
                WHERE phone_number = OLD.phone_number AND total_count = 0;
            END;
            CREATE TRIGGER IF NOT EXISTS messages_update_timeline_conversation_summary
            AFTER UPDATE OF timestamp, created_at ON messages
            BEGIN
                UPDATE conversation_summaries
                SET last_message_id = (
                    SELECT id FROM messages
                    WHERE phone_number = NEW.phone_number
                    ORDER BY COALESCE(julianday(timestamp), julianday(created_at)) DESC, id DESC
                    LIMIT 1
                )
                WHERE phone_number = NEW.phone_number;
            END;
            CREATE TABLE IF NOT EXISTS meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS forward_deliveries (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
                profile_key TEXT NOT NULL,
                state TEXT NOT NULL DEFAULT 'pending'
                    CHECK (state IN ('pending', 'in_flight', 'retry_wait', 'succeeded', 'permanent_failed')),
                attempt_count INTEGER NOT NULL DEFAULT 0,
                next_attempt_at TEXT,
                lease_at TEXT,
                lease_token TEXT,
                last_error TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                UNIQUE(message_id, profile_key)
            );
            CREATE INDEX IF NOT EXISTS idx_deliveries_state ON forward_deliveries(state);
            CREATE INDEX IF NOT EXISTS idx_deliveries_next_attempt ON forward_deliveries(next_attempt_at);
            CREATE INDEX IF NOT EXISTS idx_deliveries_due
                ON forward_deliveries(julianday(COALESCE(next_attempt_at, created_at)), id)
                WHERE state IN ('pending', 'retry_wait');
            CREATE TABLE IF NOT EXISTS forward_attempt_samples (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                profile_key TEXT NOT NULL,
                delivery_id INTEGER NULL,
                attempt_number INTEGER NOT NULL,
                started_at TEXT NOT NULL,
                completed_at TEXT NOT NULL,
                latency_ms INTEGER NOT NULL,
                dispatch_delay_ms INTEGER NULL,
                outcome TEXT NOT NULL
                    CHECK (outcome IN ('success', 'transient_failure', 'permanent_failure')),
                error_code TEXT NULL,
                created_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_attempts_profile ON forward_attempt_samples(profile_key, completed_at DESC, id DESC);
            CREATE TRIGGER IF NOT EXISTS messages_validate_domain_insert
            BEFORE INSERT ON messages
            WHEN NEW.direction NOT IN ('inbound', 'outbound')
              OR NEW.status NOT IN ('received', 'sending', 'sent', 'failed')
              OR NEW.source NOT IN ('modem', 'web', 'cli')
            BEGIN
                SELECT RAISE(ABORT, 'invalid message domain value');
            END;
            CREATE TRIGGER IF NOT EXISTS messages_validate_domain_update
            BEFORE UPDATE ON messages
            WHEN NEW.direction NOT IN ('inbound', 'outbound')
              OR NEW.status NOT IN ('received', 'sending', 'sent', 'failed')
              OR NEW.source NOT IN ('modem', 'web', 'cli')
            BEGIN
                SELECT RAISE(ABORT, 'invalid message domain value');
            END;
            CREATE TRIGGER IF NOT EXISTS deliveries_validate_state_insert
            BEFORE INSERT ON forward_deliveries
            WHEN NEW.state NOT IN ('pending', 'in_flight', 'retry_wait', 'succeeded', 'permanent_failed')
            BEGIN
                SELECT RAISE(ABORT, 'invalid delivery state');
            END;
            CREATE TRIGGER IF NOT EXISTS deliveries_validate_state_update
            BEFORE UPDATE ON forward_deliveries
            WHEN NEW.state NOT IN ('pending', 'in_flight', 'retry_wait', 'succeeded', 'permanent_failed')
            BEGIN
                SELECT RAISE(ABORT, 'invalid delivery state');
            END;
            CREATE TRIGGER IF NOT EXISTS attempt_samples_validate_outcome_insert
            BEFORE INSERT ON forward_attempt_samples
            WHEN NEW.outcome NOT IN ('success', 'transient_failure', 'permanent_failure')
            BEGIN
                SELECT RAISE(ABORT, 'invalid forward attempt outcome');
            END;
            CREATE TRIGGER IF NOT EXISTS attempt_samples_validate_outcome_update
            BEFORE UPDATE ON forward_attempt_samples
            WHEN NEW.outcome NOT IN ('success', 'transient_failure', 'permanent_failure')
            BEGIN
                SELECT RAISE(ABORT, 'invalid forward attempt outcome');
            END;",
        )?;

        migrations::migrate_existing_schema(&tx)?;

        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::OffsetDateTime;

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
                inbound_dedupe_key: None,
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
    fn lists_latest_messages_by_sms_timestamp_not_insert_order() {
        let store = memory_store();
        store
            .insert_message(NewMessage {
                direction: MessageDirection::Inbound,
                phone_number: "+15550000000".to_string(),
                body: "yesterday-current".to_string(),
                timestamp: "2026-07-19T12:00:00Z".to_string(),
                status: MessageStatus::Received,
                source: MessageSource::Modem,
                modem_sms_path: None,
                read_at: None,
                error: None,
                inbound_dedupe_key: None,
            })
            .unwrap();

        for day in 1..=12 {
            store
                .insert_message(NewMessage {
                    direction: MessageDirection::Inbound,
                    phone_number: "+15550000000".to_string(),
                    body: format!("historical-{day}"),
                    timestamp: format!("2025-01-{day:02}T12:00:00Z"),
                    status: MessageStatus::Received,
                    source: MessageSource::Modem,
                    modem_sms_path: None,
                    read_at: None,
                    error: None,
                    inbound_dedupe_key: None,
                })
                .unwrap();
        }

        let rows = store.list_messages(&MessageFilter::default()).unwrap();

        assert_eq!(rows.len(), 10);
        assert_eq!(rows[0].body, "yesterday-current");
        assert_eq!(rows[1].body, "historical-12");
    }

    #[test]
    fn timeline_cursor_pages_by_timestamp_and_id_without_duplicates() {
        let store = memory_store();
        for (body, timestamp) in [
            ("newest", "2026-03-01T00:00:00Z"),
            ("same-time-first", "2026-02-01T00:00:00Z"),
            ("same-time-second", "2026-02-01T00:00:00Z"),
            ("oldest-imported-last", "2026-01-01T00:00:00Z"),
        ] {
            store
                .insert_message(NewMessage {
                    direction: MessageDirection::Inbound,
                    phone_number: "+1".to_string(),
                    body: body.to_string(),
                    timestamp: timestamp.to_string(),
                    status: MessageStatus::Received,
                    source: MessageSource::Modem,
                    modem_sms_path: None,
                    read_at: None,
                    error: None,
                    inbound_dedupe_key: None,
                })
                .unwrap();
        }

        let first_page = store
            .list_messages(&MessageFilter {
                limit: Some(2),
                phone_number: Some("+1".to_string()),
                ..MessageFilter::default()
            })
            .unwrap();
        let cursor = first_page.last().unwrap();
        let second_page = store
            .list_messages(&MessageFilter {
                limit: Some(2),
                before: Some(MessageCursor::Timeline {
                    timestamp: cursor.timestamp.clone(),
                    id: cursor.id,
                }),
                phone_number: Some("+1".to_string()),
                ..MessageFilter::default()
            })
            .unwrap();

        assert_eq!(
            first_page
                .iter()
                .chain(&second_page)
                .map(|message| message.body.as_str())
                .collect::<Vec<_>>(),
            vec![
                "newest",
                "same-time-second",
                "same-time-first",
                "oldest-imported-last",
            ]
        );

        let legacy_second_page = store
            .list_messages(&MessageFilter {
                limit: Some(2),
                before: Some(MessageCursor::LegacyId(cursor.id)),
                phone_number: Some("+1".to_string()),
                ..MessageFilter::default()
            })
            .unwrap();
        assert_eq!(legacy_second_page, second_page);
    }

    #[test]
    fn timeline_cursor_uses_the_phone_timeline_index_range() {
        let store = memory_store();
        let cursor_message = store
            .insert_message(NewMessage {
                direction: MessageDirection::Inbound,
                phone_number: "+1".to_string(),
                body: "cursor".to_string(),
                timestamp: "2026-02-01T00:00:00Z".to_string(),
                status: MessageStatus::Received,
                source: MessageSource::Modem,
                modem_sms_path: None,
                read_at: None,
                error: None,
                inbound_dedupe_key: None,
            })
            .unwrap();
        let filter = MessageFilter {
            before: Some(MessageCursor::Timeline {
                timestamp: cursor_message.timestamp,
                id: cursor_message.id,
            }),
            phone_number: Some("+1".to_string()),
            ..MessageFilter::default()
        };
        let conn = store.conn.lock().unwrap();
        let cursor = resolve_message_cursor(&conn, filter.before.as_ref()).unwrap();
        let (sql, values) = build_message_query(&conn, &filter, cursor.as_ref(), true).unwrap();
        let mut statement = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
        let details = statement
            .query_map(params_from_iter(values.iter()), |row| {
                row.get::<_, String>(3)
            })
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();

        assert!(
            details.iter().any(|detail| {
                detail.contains("idx_messages_phone_timeline_v2") && detail.contains("<expr><?")
            }),
            "query plan did not use a timeline range: {details:?}"
        );
    }

    #[test]
    fn malformed_sms_timestamps_fall_back_to_ingestion_time_across_pages() {
        let store = memory_store();
        let newest_valid = store
            .insert_message(NewMessage {
                direction: MessageDirection::Inbound,
                phone_number: "+1".to_string(),
                body: "newest valid".to_string(),
                timestamp: "2026-03-01T00:00:00Z".to_string(),
                status: MessageStatus::Received,
                source: MessageSource::Modem,
                modem_sms_path: None,
                read_at: None,
                error: None,
                inbound_dedupe_key: None,
            })
            .unwrap();
        let malformed = store
            .insert_message(NewMessage {
                direction: MessageDirection::Inbound,
                phone_number: "+1".to_string(),
                body: "malformed timestamp".to_string(),
                timestamp: "not-a-timestamp".to_string(),
                status: MessageStatus::Received,
                source: MessageSource::Modem,
                modem_sms_path: None,
                read_at: None,
                error: None,
                inbound_dedupe_key: None,
            })
            .unwrap();
        let oldest = store
            .insert_message(NewMessage {
                direction: MessageDirection::Inbound,
                phone_number: "+1".to_string(),
                body: "oldest".to_string(),
                timestamp: "2026-01-01T00:00:00Z".to_string(),
                status: MessageStatus::Received,
                source: MessageSource::Modem,
                modem_sms_path: None,
                read_at: None,
                error: None,
                inbound_dedupe_key: None,
            })
            .unwrap();
        store
            .conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE messages SET created_at = '2026-04-01T00:00:00Z' WHERE id = ?1",
                params![malformed.id],
            )
            .unwrap();

        let first_page = store
            .list_messages(&MessageFilter {
                limit: Some(1),
                phone_number: Some("+1".to_string()),
                ..MessageFilter::default()
            })
            .unwrap();
        assert_eq!(
            first_page
                .iter()
                .map(|message| message.id)
                .collect::<Vec<_>>(),
            vec![malformed.id]
        );

        let conversations = store.list_conversations().unwrap();
        assert_eq!(conversations[0].last_message.id, malformed.id);

        let second_page = store
            .list_messages(&MessageFilter {
                limit: Some(2),
                before: Some(MessageCursor::Timeline {
                    timestamp: malformed.timestamp,
                    id: malformed.id,
                }),
                phone_number: Some("+1".to_string()),
                ..MessageFilter::default()
            })
            .unwrap();
        assert_eq!(
            second_page
                .iter()
                .map(|message| message.id)
                .collect::<Vec<_>>(),
            vec![newest_valid.id, oldest.id]
        );
    }

    #[test]
    fn optional_message_lookup_distinguishes_absence_from_query_failure() {
        let store = memory_store();
        assert_eq!(store.get_message_optional(42).unwrap(), None);

        {
            let conn = store.conn.lock().unwrap();
            conn.execute("DROP TABLE messages", []).unwrap();
        }

        assert!(store.get_message_optional(42).is_err());
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
                inbound_dedupe_key: None,
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
    fn conversations_use_latest_sms_timestamp_not_insert_order() {
        let store = memory_store();
        store
            .insert_message(NewMessage {
                direction: MessageDirection::Inbound,
                phone_number: "+1".to_string(),
                body: "current".to_string(),
                timestamp: "2026-07-19T12:00:00Z".to_string(),
                status: MessageStatus::Received,
                source: MessageSource::Modem,
                modem_sms_path: None,
                read_at: None,
                error: None,
                inbound_dedupe_key: None,
            })
            .unwrap();
        store
            .insert_message(NewMessage {
                direction: MessageDirection::Inbound,
                phone_number: "+1".to_string(),
                body: "imported history".to_string(),
                timestamp: "2025-01-01T12:00:00Z".to_string(),
                status: MessageStatus::Received,
                source: MessageSource::Modem,
                modem_sms_path: None,
                read_at: None,
                error: None,
                inbound_dedupe_key: None,
            })
            .unwrap();

        let conversations = store.list_conversations().unwrap();

        assert_eq!(conversations[0].last_message.body, "current");
        assert_eq!(conversations[0].total_count, 2);
    }

    #[test]
    fn conversation_summary_tracks_read_state_and_last_message_after_delete() {
        let store = memory_store();
        let earlier = store
            .insert_message(NewMessage::inbound("+15550000001", "earlier"))
            .unwrap();
        let latest = store
            .insert_message(NewMessage::inbound("+15550000001", "latest"))
            .unwrap();

        store.mark_read(earlier.id).unwrap();
        store.delete_messages(&[latest.id]).unwrap();

        let conversations = store.list_conversations().unwrap();
        assert_eq!(conversations.len(), 1);
        assert_eq!(conversations[0].last_message.id, earlier.id);
        assert_eq!(conversations[0].total_count, 1);
        assert_eq!(conversations[0].unread_count, 0);

        let conn = store.conn.lock().unwrap();
        let summary: (i64, i64, i64) = conn
            .query_row(
                "SELECT total_count, unread_count, last_message_id
                 FROM conversation_summaries WHERE phone_number = ?1",
                params!["+15550000001"],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(summary, (1, 0, earlier.id));
    }

    #[test]
    fn filters_by_timestamp_range() {
        let store = memory_store();
        let lower_bound = store
            .insert_message(NewMessage {
                direction: MessageDirection::Inbound,
                phone_number: "+1".to_string(),
                body: "lower bound".to_string(),
                timestamp: "not-a-timestamp".to_string(),
                status: MessageStatus::Received,
                source: MessageSource::Modem,
                modem_sms_path: None,
                read_at: None,
                error: None,
                inbound_dedupe_key: None,
            })
            .unwrap();
        store
            .conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE messages SET created_at = '2026-06-01T00:00:00Z' WHERE id = ?1",
                params![lower_bound.id],
            )
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
                inbound_dedupe_key: None,
            })
            .unwrap();
        store
            .insert_message(NewMessage {
                direction: MessageDirection::Inbound,
                phone_number: "+3".to_string(),
                body: "upper bound".to_string(),
                timestamp: "2026-07-01T00:00:00Z".to_string(),
                status: MessageStatus::Received,
                source: MessageSource::Modem,
                modem_sms_path: None,
                read_at: None,
                error: None,
                inbound_dedupe_key: None,
            })
            .unwrap();
        store
            .insert_message(NewMessage {
                direction: MessageDirection::Inbound,
                phone_number: "+4".to_string(),
                body: "late".to_string(),
                timestamp: "2026-12-31T23:59:59Z".to_string(),
                status: MessageStatus::Received,
                source: MessageSource::Modem,
                modem_sms_path: None,
                read_at: None,
                error: None,
                inbound_dedupe_key: None,
            })
            .unwrap();

        let rows = store
            .list_messages(&MessageFilter {
                from: Some("2026-06-01T04:00:00+04:00".to_string()),
                to: Some("2026-06-30T20:00:00-04:00".to_string()),
                ..MessageFilter::default()
            })
            .unwrap();
        assert_eq!(
            rows.iter()
                .map(|message| message.body.as_str())
                .collect::<Vec<_>>(),
            vec!["upper bound", "middle", "lower bound"]
        );
    }

    #[test]
    fn export_iteration_ignores_page_limit_without_collecting_in_store() {
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
        let mut rows = Vec::new();
        store
            .for_each_export_message(&filter, |message| {
                rows.push(message);
                Ok(true)
            })
            .unwrap();
        assert_eq!(rows.len(), 2);
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
                    inbound_dedupe_key: None,
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
        assert_eq!(store.get_meta("test_key").unwrap(), None);
        store.set_meta("test_key", "hello").unwrap();
        assert_eq!(
            store.get_meta("test_key").unwrap().as_deref(),
            Some("hello")
        );
        store.set_meta("test_key", "updated").unwrap();
        assert_eq!(
            store.get_meta("test_key").unwrap().as_deref(),
            Some("updated")
        );
    }

    #[test]
    fn message_reads_reject_unknown_persisted_enum_values() {
        let store = memory_store();
        {
            let conn = store.conn.lock().unwrap();
            conn.pragma_update(None, "ignore_check_constraints", "ON")
                .unwrap();
            conn.execute("DROP TRIGGER messages_validate_domain_insert", [])
                .unwrap();
            conn.execute(
                "INSERT INTO messages (
                    direction, phone_number, body, timestamp, status, source,
                    created_at, updated_at
                 ) VALUES ('sideways', '+1', 'invalid', '2026-01-01T00:00:00Z',
                           'received', 'modem', '2026-01-01T00:00:00Z',
                           '2026-01-01T00:00:00Z')",
                [],
            )
            .unwrap();
        }

        assert!(store.get_message(1).is_err());
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
                inbound_dedupe_key: None,
            })
            .unwrap();
        let deleted = store.run_retention(1, 100).unwrap();
        assert_eq!(deleted, 1);
    }

    #[test]
    fn retention_compares_rfc3339_timestamps_by_instant() {
        let store = memory_store();
        let cutoff = OffsetDateTime::now_utc() - time::Duration::days(1);
        let recent = (cutoff + time::Duration::minutes(30))
            .to_offset(time::UtcOffset::from_hms(-12, 0, 0).unwrap())
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap();
        store
            .insert_message(NewMessage {
                direction: MessageDirection::Inbound,
                phone_number: "+1".to_string(),
                body: "recent with negative offset".to_string(),
                timestamp: recent,
                status: MessageStatus::Received,
                source: MessageSource::Modem,
                modem_sms_path: None,
                read_at: None,
                error: None,
                inbound_dedupe_key: None,
            })
            .unwrap();

        let deleted = store.run_retention(1, 100).unwrap();

        assert_eq!(deleted, 0);
        assert_eq!(store.count_messages().unwrap(), 1);
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
                inbound_dedupe_key: None,
            })
            .unwrap();
        store
            .insert_deliveries(msg.id, &["bark.test".to_string()])
            .unwrap();
        let deleted = store.run_retention(1, 100).unwrap();
        assert_eq!(deleted, 0);
    }

    #[test]
    fn inbound_message_and_deliveries_commit_atomically() {
        let store = memory_store();
        let duplicate_profiles = ["bark.primary".to_string(), "bark.primary".to_string()];

        let result = store.insert_message_with_deliveries(
            NewMessage::inbound("+1", "atomic"),
            &duplicate_profiles,
        );

        assert!(result.is_err());
        assert_eq!(store.count_messages().unwrap(), 0);
        assert_eq!(store.count_deliveries().unwrap(), 0);
    }

    #[test]
    fn repeated_inbound_dedupe_key_creates_one_message_and_delivery_set() {
        let store = memory_store();
        let profiles = ["bark.primary".to_string(), "telegram.backup".to_string()];
        let input = || {
            NewMessage::modem_inbound(
                "+15550000000",
                "same sms",
                "2026-07-12T16:57:00+08:00",
                "/org/freedesktop/ModemManager1/SMS/42",
                "modem-fingerprint",
            )
        };

        let first = store
            .insert_inbound_message_with_deliveries(input(), &profiles)
            .unwrap();
        let second = store
            .insert_inbound_message_with_deliveries(input(), &profiles)
            .unwrap();

        let InboundInsertResult::Inserted(inserted) = first else {
            panic!("first reception must insert the message");
        };
        let InboundInsertResult::Duplicate(existing) = second else {
            panic!("replayed reception must be reported as a duplicate");
        };
        assert_eq!(existing.id, inserted.id);
        assert_eq!(store.count_messages().unwrap(), 1);
        assert_eq!(store.count_deliveries().unwrap(), 2);
    }

    #[test]
    fn inbound_dedupe_survives_path_changes_but_not_provider_timestamp_changes() {
        let store = memory_store();
        let profiles = ["bark.primary".to_string()];

        let first = NewMessage::modem_inbound(
            "+15550000000",
            "same sms",
            "2026-07-12T16:57:00+08:00",
            "/org/freedesktop/ModemManager1/SMS/42",
            "modem-fingerprint",
        );
        let replayed_at_new_path = NewMessage::modem_inbound(
            "+15550000000",
            "same sms",
            "2026-07-12T16:57:00+08:00",
            "/org/freedesktop/ModemManager1/SMS/99",
            "modem-fingerprint",
        );
        let later_message = NewMessage::modem_inbound(
            "+15550000000",
            "same sms",
            "2026-07-12T16:58:00+08:00",
            "/org/freedesktop/ModemManager1/SMS/100",
            "modem-fingerprint",
        );

        assert!(matches!(
            store
                .insert_inbound_message_with_deliveries(first, &profiles)
                .unwrap(),
            InboundInsertResult::Inserted(_)
        ));
        assert!(matches!(
            store
                .insert_inbound_message_with_deliveries(replayed_at_new_path, &profiles)
                .unwrap(),
            InboundInsertResult::Duplicate(_)
        ));
        assert!(matches!(
            store
                .insert_inbound_message_with_deliveries(later_message, &profiles)
                .unwrap(),
            InboundInsertResult::Inserted(_)
        ));
        assert_eq!(store.count_messages().unwrap(), 2);
        assert_eq!(store.count_deliveries().unwrap(), 2);
    }

    #[test]
    fn forwarding_attempt_history_keeps_latest_five_completed_per_profile() {
        let store = memory_store();

        for attempt_number in 1..=6 {
            store
                .record_forward_attempt(NewForwardAttemptSample {
                    profile_key: "bark.primary".to_string(),
                    delivery_id: None,
                    attempt_number,
                    started_at: format!("2026-07-12T16:57:0{}Z", attempt_number - 1),
                    completed_at: format!("2026-07-12T16:57:0{attempt_number}Z"),
                    latency_ms: attempt_number as i64 * 100,
                    dispatch_delay_ms: 0,
                    outcome: ForwardAttemptOutcome::Success,
                    error_code: None,
                })
                .unwrap();
        }
        store
            .record_forward_attempt(NewForwardAttemptSample {
                profile_key: "telegram.backup".to_string(),
                delivery_id: None,
                attempt_number: 1,
                started_at: "2026-07-12T16:58:00Z".to_string(),
                completed_at: "2026-07-12T16:58:01Z".to_string(),
                latency_ms: 1_000,
                dispatch_delay_ms: 0,
                outcome: ForwardAttemptOutcome::TransientFailure,
                error_code: Some("http_timeout".to_string()),
            })
            .unwrap();

        let bark = store.list_forward_attempts("bark.primary", 5).unwrap();
        let telegram = store.list_forward_attempts("telegram.backup", 5).unwrap();
        assert_eq!(bark.len(), 5);
        assert_eq!(
            bark.iter()
                .map(|sample| sample.attempt_number)
                .collect::<Vec<_>>(),
            vec![6, 5, 4, 3, 2]
        );
        assert!(bark[0].is_retry());
        assert_eq!(telegram.len(), 1);
        assert_eq!(telegram[0].error_code.as_deref(), Some("http_timeout"));
    }

    #[test]
    fn migrates_legacy_attempt_samples_with_unknown_dispatch_delay() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE forward_attempt_samples (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                profile_key TEXT NOT NULL,
                delivery_id INTEGER NULL,
                attempt_number INTEGER NOT NULL,
                started_at TEXT NOT NULL,
                completed_at TEXT NOT NULL,
                latency_ms INTEGER NOT NULL,
                outcome TEXT NOT NULL,
                error_code TEXT NULL,
                created_at TEXT NOT NULL
            );
            INSERT INTO forward_attempt_samples
                (profile_key, delivery_id, attempt_number, started_at, completed_at,
                 latency_ms, outcome, error_code, created_at)
            VALUES
                ('bark.primary', NULL, 1, '2026-07-12T16:57:00Z',
                 '2026-07-12T16:57:01Z', 1000, 'success', NULL,
                 '2026-07-12T16:57:01Z');",
        )
        .unwrap();
        let store = MessageStore {
            conn: Arc::new(Mutex::new(conn)),
            path: None,
        };

        store.migrate().unwrap();

        let samples = store.list_forward_attempts("bark.primary", 5).unwrap();
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].dispatch_delay_ms, None);
    }

    #[test]
    fn claims_deliveries_by_due_time_instead_of_creation_id() {
        let store = memory_store();
        store
            .insert_message_with_deliveries(
                NewMessage::inbound("+1", "older retry"),
                &["bark.primary".to_string()],
            )
            .unwrap();
        store
            .insert_message_with_deliveries(
                NewMessage::inbound("+1", "earlier due pending"),
                &["telegram.primary".to_string()],
            )
            .unwrap();
        {
            let conn = store.conn.lock().unwrap();
            conn.execute(
                "UPDATE forward_deliveries
                 SET state = 'retry_wait', next_attempt_at = '2026-01-01T00:00:20Z'
                 WHERE id = 1",
                [],
            )
            .unwrap();
            conn.execute(
                "UPDATE forward_deliveries SET created_at = '2026-01-01T00:00:10Z'
                 WHERE id = 2",
                [],
            )
            .unwrap();
        }

        let claimed = store
            .claim_due_deliveries(1, "2099-01-01T00:00:00Z")
            .unwrap();

        assert_eq!(claimed[0].id, 2);
    }

    #[test]
    fn claims_due_delivery_when_rfc3339_precision_differs_from_now() {
        let store = memory_store();
        store
            .insert_message_with_deliveries(
                NewMessage::inbound("+1", "due at whole second"),
                &["bark.primary".to_string()],
            )
            .unwrap();
        let due_at = OffsetDateTime::now_utc()
            .replace_nanosecond(0)
            .unwrap()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap();
        {
            let conn = store.conn.lock().unwrap();
            conn.execute(
                "UPDATE forward_deliveries
                 SET state = 'retry_wait', next_attempt_at = ?1",
                params![due_at],
            )
            .unwrap();
        }

        let claimed = store
            .claim_due_deliveries(1, "2099-01-01T00:00:00Z")
            .unwrap();

        assert_eq!(claimed.len(), 1);
    }

    #[test]
    fn pending_delivery_is_due_even_when_created_at_is_in_the_future() {
        let store = memory_store();
        store
            .insert_message_with_deliveries(
                NewMessage::inbound("+1", "clock moved backwards"),
                &["bark.primary".to_string()],
            )
            .unwrap();
        {
            let conn = store.conn.lock().unwrap();
            conn.execute(
                "UPDATE forward_deliveries
                 SET created_at = '2099-01-01T00:00:00Z'
                 WHERE next_attempt_at IS NULL",
                [],
            )
            .unwrap();
        }

        let claimed = store
            .claim_due_deliveries(1, "2099-01-01T00:00:00Z")
            .unwrap();

        assert_eq!(claimed.len(), 1);
    }

    #[test]
    fn reports_the_earliest_pending_or_retry_deadline() {
        let store = memory_store();
        assert_eq!(store.next_delivery_due_at().unwrap(), None);
        store
            .insert_message_with_deliveries(
                NewMessage::inbound("+1", "deadline"),
                &["bark.primary".to_string()],
            )
            .unwrap();
        let delivery = store
            .claim_due_deliveries(1, "2099-01-01T00:00:00Z")
            .unwrap()
            .pop()
            .unwrap();
        store
            .complete_delivery(
                delivery.id,
                DeliveryState::RetryWait,
                Some("http_timeout"),
                1,
                Some("2099-01-02T00:00:00Z"),
                delivery.lease_token.as_deref().unwrap(),
            )
            .unwrap();

        assert_eq!(
            store.next_delivery_due_at().unwrap().as_deref(),
            Some("2099-01-02T00:00:00Z")
        );
    }

    #[test]
    fn forwarding_attempt_is_retained_when_delivery_lease_is_lost() {
        let store = memory_store();
        let message = store
            .insert_message_with_deliveries(
                NewMessage::inbound("+1", "lease sample"),
                &["bark.primary".to_string()],
            )
            .unwrap();
        let delivery = store
            .claim_due_deliveries(1, "2099-01-01T00:00:00Z")
            .unwrap()
            .pop()
            .unwrap();

        let completed = store
            .complete_delivery_with_attempt(DeliveryCompletion {
                id: delivery.id,
                state: DeliveryState::Succeeded,
                error: None,
                attempt_count: 1,
                next_attempt_at: None,
                lease_token: "wrong-lease-token",
                sample: NewForwardAttemptSample {
                    profile_key: delivery.profile_key,
                    delivery_id: Some(delivery.id),
                    attempt_number: 1,
                    started_at: "2026-07-12T16:57:00Z".to_string(),
                    completed_at: "2026-07-12T16:57:01Z".to_string(),
                    latency_ms: 1_000,
                    dispatch_delay_ms: 0,
                    outcome: ForwardAttemptOutcome::Success,
                    error_code: None,
                },
            })
            .unwrap();

        assert!(!completed);
        assert_eq!(
            store.get_delivery(delivery.id).unwrap().state,
            DeliveryState::InFlight
        );
        assert_eq!(store.get_message(message.id).unwrap().body, "lease sample");
        assert_eq!(
            store
                .list_forward_attempts("bark.primary", 5)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn migrates_a_populated_pre_delivery_database() {
        let path = std::env::temp_dir().join(format!(
            "sms-relayed-legacy-{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let legacy = Connection::open(&path).unwrap();
        legacy
            .execute_batch(
                "CREATE TABLE messages (
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
                INSERT INTO messages
                    (direction, phone_number, body, timestamp, status, source, created_at, updated_at)
                VALUES
                    ('inbound', '+1', 'legacy', '2026-01-01T00:00:00Z', 'received',
                     'modem', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
                CREATE TABLE forward_deliveries (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    message_id INTEGER NOT NULL,
                    profile_key TEXT NOT NULL,
                    state TEXT NOT NULL DEFAULT 'pending',
                    attempt_count INTEGER NOT NULL DEFAULT 0,
                    next_attempt_at TEXT,
                    lease_at TEXT,
                    lease_token TEXT,
                    last_error TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    UNIQUE(message_id, profile_key)
                );",
            )
            .unwrap();
        drop(legacy);

        let store = MessageStore::open(&path).unwrap();
        assert_eq!(store.count_messages().unwrap(), 1);
        store
            .insert_deliveries(1, &["bark.primary".to_string()])
            .unwrap();
        assert_eq!(store.count_deliveries().unwrap(), 1);
        {
            let conn = store.conn.lock().unwrap();
            assert!(conn
                .execute(
                    "INSERT INTO messages
                        (direction, phone_number, body, timestamp, status, source, created_at, updated_at)
                     VALUES ('sideways', '+2', 'invalid', '2026-01-01T00:00:00Z',
                             'received', 'modem', '2026-01-01T00:00:00Z',
                             '2026-01-01T00:00:00Z')",
                    [],
                )
                .is_err());
            assert!(conn
                .execute(
                    "INSERT INTO forward_deliveries
                        (message_id, profile_key, state, created_at, updated_at)
                     VALUES (1, 'invalid.state', 'unknown', '2026-01-01T00:00:00Z',
                             '2026-01-01T00:00:00Z')",
                    [],
                )
                .is_err());
            assert!(conn
                .execute(
                    "INSERT INTO forward_attempt_samples
                        (profile_key, attempt_number, started_at, completed_at, latency_ms,
                         outcome, created_at)
                     VALUES ('invalid.outcome', 1, '2026-01-01T00:00:00Z',
                             '2026-01-01T00:00:01Z', 1000, 'unknown',
                             '2026-01-01T00:00:01Z')",
                    [],
                )
                .is_err());
        }
        drop(store);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }

    #[test]
    fn legacy_migration_backfills_dedupe_keys_with_fingerprint() {
        let path = std::env::temp_dir().join(format!(
            "sms-relayed-legacy-backfill-{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        {
            let legacy = Connection::open(&path).unwrap();
            legacy.execute_batch(
                "CREATE TABLE messages (
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
                INSERT INTO messages
                    (direction, phone_number, body, timestamp, status, source, created_at, updated_at)
                VALUES
                    ('inbound', '+1', 'original', '2026-01-01T00:00:00Z', 'received',
                     'modem', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
                INSERT INTO messages
                    (direction, phone_number, body, timestamp, status, source, created_at, updated_at)
                VALUES
                    ('inbound', '+1', 'original', '2026-01-01T00:00:00Z', 'received',
                     'modem', '2026-01-01T00:00:01Z', '2026-01-01T00:00:01Z');",
            ).unwrap();
            // Pre-migration deliveries are terminal
            legacy.execute_batch(
                "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 INSERT INTO meta (key, value) VALUES ('modem_fingerprint', 'test-fingerprint');
                 CREATE TABLE forward_deliveries (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    message_id INTEGER NOT NULL,
                    profile_key TEXT NOT NULL,
                    state TEXT NOT NULL DEFAULT 'pending',
                    attempt_count INTEGER NOT NULL DEFAULT 0,
                    next_attempt_at TEXT,
                    lease_at TEXT,
                    lease_token TEXT,
                    last_error TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                 );
                 INSERT INTO forward_deliveries
                    (message_id, profile_key, state, attempt_count, created_at, updated_at)
                 VALUES (1, 'bark.primary', 'succeeded', 2, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');",
            ).unwrap();
        }

        let store = MessageStore::open(&path).unwrap();
        // First row (id=1) gets dedupe key – verify by inserting a duplicate
        let probe = NewMessage::modem_inbound(
            "+1",
            "original",
            "2026-01-01T00:00:00Z",
            "/org/.../SMS/99",
            "test-fingerprint",
        );
        let result = store
            .insert_inbound_message_with_deliveries(probe, &[])
            .unwrap();
        assert!(
            matches!(result, InboundInsertResult::Duplicate(_)),
            "first legacy row must get dedupe key for duplicate detection"
        );

        // Second row (duplicate content) stays NULL, but dedup still catches it
        let probe2 = NewMessage::modem_inbound(
            "+1",
            "original",
            "2026-01-01T00:00:00Z",
            "/org/.../SMS/100",
            "test-fingerprint",
        );
        let result2 = store
            .insert_inbound_message_with_deliveries(probe2, &[])
            .unwrap();
        assert!(
            matches!(result2, InboundInsertResult::Duplicate(_)),
            "second row also deduped via the existing key"
        );

        // Only 2 original messages exist, no new ones from probes
        assert_eq!(store.count_messages().unwrap(), 2);

        // Delivery state preserved (delivery is id=1 from legacy setup)
        let delivery = store.get_delivery(1).unwrap();
        assert_eq!(delivery.state, DeliveryState::Succeeded);
        assert_eq!(delivery.attempt_count, 2);

        // Idempotent re-open
        drop(store);
        let store = MessageStore::open(&path).unwrap();
        assert_eq!(store.count_messages().unwrap(), 2);

        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{}", path.display(), suffix));
        }
    }

    #[test]
    fn legacy_migration_defers_backfill_then_completes_after_enrollment() {
        let path = std::env::temp_dir().join(format!(
            "sms-relayed-deferred-backfill-{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        {
            let legacy = Connection::open(&path).unwrap();
            legacy.execute_batch(
                "CREATE TABLE messages (
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
                INSERT INTO messages
                    (direction, phone_number, body, timestamp, status, source, created_at, updated_at)
                VALUES
                    ('inbound', '+1', 'original', '2026-01-01T00:00:00Z', 'received',
                     'modem', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
                INSERT INTO messages
                    (direction, phone_number, body, timestamp, status, source, created_at, updated_at)
                VALUES
                    ('inbound', '+2', 'other', '2026-01-02T00:00:00Z', 'received',
                     'modem', '2026-01-02T00:00:00Z', '2026-01-02T00:00:00Z');",
            ).unwrap();
        }

        // Open without fingerprint - backfill skipped
        let store = MessageStore::open(&path).unwrap();
        assert_eq!(store.count_messages().unwrap(), 2);
        // Verify via insert that first message does not dedupe (no key set)
        // Use a different fingerprint than the eventual enrollment to avoid key collision
        let probe_before = NewMessage::modem_inbound(
            "+1",
            "original",
            "2026-01-01T00:00:00Z",
            "/org/.../SMS/99",
            "temp-fp",
        );
        let result_before = store
            .insert_inbound_message_with_deliveries(probe_before, &[])
            .unwrap();
        // No matching key → the probe creates a new message (Inserted), not Duplicate
        assert!(
            matches!(result_before, InboundInsertResult::Inserted(_)),
            "without backfill, duplicate must insert a new message"
        );
        assert_eq!(store.count_messages().unwrap(), 3);

        // Enroll fingerprint now and backfill
        store.set_meta("modem_fingerprint", "enrolled-fp").unwrap();
        let count = store.backfill_dedupe_keys().unwrap();
        // Two legacy rows get backfilled with key computed from "enrolled-fp"
        assert_eq!(count, 2, "both legacy rows should be backfilled");

        // Replay with same content is suppressed
        let input = NewMessage::modem_inbound(
            "+1",
            "original",
            "2026-01-01T00:00:00Z",
            "/org/.../SMS/99",
            "enrolled-fp",
        );
        let result = store
            .insert_inbound_message_with_deliveries(input, &[])
            .unwrap();
        assert!(
            matches!(result, InboundInsertResult::Duplicate(_)),
            "replay must be suppressed"
        );

        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{}", path.display(), suffix));
        }
    }

    #[test]
    fn delivery_claims_use_owner_tokens_and_recover_expired_leases() {
        let store = memory_store();
        let message = store
            .insert_message_with_deliveries(
                NewMessage::inbound("+1", "lease"),
                &["bark.primary".to_string()],
            )
            .unwrap();
        assert_eq!(store.count_deliveries().unwrap(), 1);

        let first = store
            .claim_due_deliveries(1, "2000-01-01T00:00:00Z")
            .unwrap()
            .pop()
            .unwrap();
        let first_token = first.lease_token.unwrap();
        let second = store
            .claim_due_deliveries(1, "2099-01-01T00:00:00Z")
            .unwrap()
            .pop()
            .unwrap();
        let second_token = second.lease_token.clone().unwrap();
        assert_ne!(first_token, second_token);
        assert!(!store
            .complete_delivery(
                second.id,
                DeliveryState::Succeeded,
                None,
                1,
                None,
                &first_token,
            )
            .unwrap());
        assert!(store
            .complete_delivery(
                second.id,
                DeliveryState::Succeeded,
                None,
                1,
                None,
                &second_token,
            )
            .unwrap());
        assert_eq!(
            store.get_delivery(second.id).unwrap().state,
            DeliveryState::Succeeded
        );
        assert_eq!(store.get_message(message.id).unwrap().body, "lease");
    }

    #[test]
    fn deleting_message_cascades_delivery_rows() {
        let store = memory_store();
        let message = store
            .insert_message_with_deliveries(
                NewMessage::inbound("+1", "delete"),
                &["bark.primary".to_string()],
            )
            .unwrap();

        store.delete_messages(&[message.id]).unwrap();

        assert_eq!(store.count_messages().unwrap(), 0);
        assert_eq!(store.count_deliveries().unwrap(), 0);
    }

    #[test]
    fn file_export_does_not_hold_the_writer_connection() {
        let path = std::env::temp_dir().join(format!(
            "sms-relayed-export-{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = MessageStore::open(&path).unwrap();
        for i in 0..50 {
            store
                .insert_message(NewMessage::inbound("+1", &format!("row-{i}")))
                .unwrap();
        }
        let export_store = store.clone();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let mut first = true;
            export_store
                .for_each_export_message(&MessageFilter::default(), |_| {
                    if first {
                        first = false;
                        started_tx.send(()).unwrap();
                    }
                    std::thread::sleep(std::time::Duration::from_millis(2));
                    Ok(true)
                })
                .unwrap();
        });
        started_rx.recv().unwrap();

        store
            .insert_message(NewMessage::inbound("+2", "written-during-export"))
            .unwrap();
        worker.join().unwrap();
        assert_eq!(store.count_messages().unwrap(), 51);

        drop(store);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{}", path.display(), suffix));
        }
    }

    #[test]
    fn complete_delivery_with_attempt_preserves_last_error_for_failures() {
        let store = memory_store();
        // Create three separate messages with deliveries
        for i in 1..=3 {
            let _ = store
                .insert_message_with_deliveries(
                    NewMessage::inbound(&format!("+{i}"), &format!("last-error-{i}")),
                    &["bark.primary".to_string()],
                )
                .unwrap();
        }

        let all = store
            .claim_due_deliveries(3, "2099-01-01T00:00:00Z")
            .unwrap();
        assert_eq!(all.len(), 3);
        let d1 = &all[0];
        let d2 = &all[1];
        let d3 = &all[2];

        // Success: last_error stays NULL
        store
            .complete_delivery_with_attempt(DeliveryCompletion {
                id: d1.id,
                state: DeliveryState::Succeeded,
                error: None,
                attempt_count: 1,
                next_attempt_at: None,
                lease_token: d1.lease_token.as_ref().unwrap(),
                sample: NewForwardAttemptSample {
                    profile_key: d1.profile_key.clone(),
                    delivery_id: Some(d1.id),
                    attempt_number: 1,
                    started_at: "2026-01-01T00:00:00Z".to_string(),
                    completed_at: "2026-01-01T00:00:01Z".to_string(),
                    latency_ms: 100,
                    dispatch_delay_ms: 0,
                    outcome: ForwardAttemptOutcome::Success,
                    error_code: None,
                },
            })
            .unwrap();

        // Permanent failure: last_error gets the error_code
        store
            .complete_delivery_with_attempt(DeliveryCompletion {
                id: d2.id,
                state: DeliveryState::PermanentFailed,
                error: Some("shell_exit_nonzero"),
                attempt_count: 1,
                next_attempt_at: None,
                lease_token: d2.lease_token.as_ref().unwrap(),
                sample: NewForwardAttemptSample {
                    profile_key: d2.profile_key.clone(),
                    delivery_id: Some(d2.id),
                    attempt_number: 1,
                    started_at: "2026-01-01T00:00:00Z".to_string(),
                    completed_at: "2026-01-01T00:00:01Z".to_string(),
                    latency_ms: 100,
                    dispatch_delay_ms: 0,
                    outcome: ForwardAttemptOutcome::PermanentFailure,
                    error_code: Some("shell_exit_nonzero".to_string()),
                },
            })
            .unwrap();
        assert_eq!(
            store.get_delivery(d2.id).unwrap().last_error.as_deref(),
            Some("shell_exit_nonzero")
        );
        assert_eq!(
            store.get_delivery(d2.id).unwrap().state,
            DeliveryState::PermanentFailed
        );

        // Transient failure: last_error gets the error_code
        store
            .complete_delivery_with_attempt(DeliveryCompletion {
                id: d3.id,
                state: DeliveryState::RetryWait,
                error: Some("http_timeout"),
                attempt_count: 1,
                next_attempt_at: Some("2099-01-02T00:00:00Z"),
                lease_token: d3.lease_token.as_ref().unwrap(),
                sample: NewForwardAttemptSample {
                    profile_key: d3.profile_key.clone(),
                    delivery_id: Some(d3.id),
                    attempt_number: 1,
                    started_at: "2026-01-01T00:00:00Z".to_string(),
                    completed_at: "2026-01-01T00:00:01Z".to_string(),
                    latency_ms: 100,
                    dispatch_delay_ms: 0,
                    outcome: ForwardAttemptOutcome::TransientFailure,
                    error_code: Some("http_timeout".to_string()),
                },
            })
            .unwrap();
        assert_eq!(
            store.get_delivery(d3.id).unwrap().last_error.as_deref(),
            Some("http_timeout")
        );
        assert_eq!(
            store.get_delivery(d3.id).unwrap().state,
            DeliveryState::RetryWait
        );
    }

    #[test]
    fn concurrent_dedupe_inserts_one_message_with_immediate_transactions() {
        let store = memory_store();
        let profiles = ["bark.primary".to_string()];

        let store1 = store.clone();
        let store2 = store.clone();
        let profiles1 = profiles.clone();
        let h1 = std::thread::spawn(move || {
            let input = NewMessage::modem_inbound(
                "+15550000000",
                "concurrent",
                "2026-07-12T17:00:00Z",
                "/org/freedesktop/ModemManager1/SMS/1",
                "modem-fingerprint",
            );
            store1.insert_inbound_message_with_deliveries(input, &profiles1)
        });
        let profiles2 = profiles.clone();
        let h2 = std::thread::spawn(move || {
            let input = NewMessage::modem_inbound(
                "+15550000000",
                "concurrent",
                "2026-07-12T17:00:00Z",
                "/org/freedesktop/ModemManager1/SMS/2",
                "modem-fingerprint",
            );
            store2.insert_inbound_message_with_deliveries(input, &profiles2)
        });

        let r1 = h1.join().unwrap();
        let r2 = h2.join().unwrap();
        assert!(r1.is_ok());
        assert!(r2.is_ok());

        let insert_count = [&r1, &r2]
            .iter()
            .filter(|r| matches!(r.as_ref().unwrap(), InboundInsertResult::Inserted(_)))
            .count();
        let duplicate_count = [&r1, &r2]
            .iter()
            .filter(|r| matches!(r.as_ref().unwrap(), InboundInsertResult::Duplicate(_)))
            .count();

        assert_eq!(insert_count, 1, "exactly one thread must Inserted");
        assert_eq!(duplicate_count, 1, "exactly one thread must get Duplicate");
        assert_eq!(store.count_messages().unwrap(), 1);
        assert_eq!(store.count_deliveries().unwrap(), 1);
    }
}
