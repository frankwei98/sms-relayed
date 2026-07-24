use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

use super::{metadata::backfill_dedupe_keys_on, CONVERSATION_SUMMARIES_BACKFILL_META_KEY};

pub(super) fn migrate_existing_schema(conn: &Connection) -> Result<()> {
    let has_dedupe: bool = conn
        .prepare(
            "SELECT COUNT(*) FROM pragma_table_info('messages')
             WHERE name = 'inbound_dedupe_key'",
        )?
        .query_row([], |row| row.get::<_, i64>(0))
        .map(|count| count > 0)?;
    if !has_dedupe {
        conn.execute(
            "ALTER TABLE messages ADD COLUMN inbound_dedupe_key TEXT NULL",
            [],
        )?;
    }

    for (name, definition) in [
        (
            "outbound_phase",
            "TEXT NULL CHECK (
                outbound_phase IN (
                    'created', 'prepared', 'send_started', 'uncertain', 'unknown', 'complete'
                )
            )",
        ),
        ("outbound_owner", "TEXT NULL"),
        ("outbound_lease_until", "TEXT NULL"),
        ("outbound_next_attempt_at", "TEXT NULL"),
    ] {
        let exists: bool = conn
            .prepare(
                "SELECT COUNT(*) FROM pragma_table_info('messages')
                 WHERE name = ?1",
            )?
            .query_row(params![name], |row| row.get::<_, i64>(0))
            .map(|count| count > 0)?;
        if !exists {
            conn.execute(
                &format!("ALTER TABLE messages ADD COLUMN {name} {definition}"),
                [],
            )?;
        }
    }

    let has_dispatch_delay: bool = conn
        .prepare(
            "SELECT COUNT(*) FROM pragma_table_info('forward_attempt_samples')
             WHERE name = 'dispatch_delay_ms'",
        )?
        .query_row([], |row| row.get::<_, i64>(0))
        .map(|count| count > 0)?;
    if !has_dispatch_delay {
        conn.execute(
            "ALTER TABLE forward_attempt_samples
             ADD COLUMN dispatch_delay_ms INTEGER NULL",
            [],
        )?;
    }

    conn.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_messages_dedupe
         ON messages(inbound_dedupe_key)
         WHERE inbound_dedupe_key IS NOT NULL",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_messages_outbound_due
         ON messages(julianday(COALESCE(outbound_next_attempt_at, created_at)), id)
         WHERE direction = 'outbound' AND status = 'sending'",
        [],
    )?;
    conn.execute(
        "UPDATE messages
         SET outbound_phase = CASE
             WHEN modem_sms_path IS NULL THEN 'created'
             ELSE 'uncertain'
         END
         WHERE direction = 'outbound'
           AND status = 'sending'
           AND outbound_phase IS NULL",
        [],
    )?;
    let event_outbox_initialized = conn
        .query_row(
            "SELECT 1 FROM meta WHERE key = 'outbound_event_outbox_v1_initialized'",
            [],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !event_outbox_initialized {
        conn.execute(
            "UPDATE messages
             SET outbound_phase = NULL,
                 outbound_owner = NULL,
                 outbound_lease_until = NULL
             WHERE direction = 'outbound'
               AND status IN ('sent', 'failed')
               AND outbound_phase = 'complete'",
            [],
        )?;
        conn.execute(
            "INSERT INTO meta (key, value)
             VALUES ('outbound_event_outbox_v1_initialized', '1')",
            [],
        )?;
    }

    backfill_dedupe_keys_on(conn)?;

    let summaries_backfilled = conn
        .query_row(
            "SELECT 1 FROM meta WHERE key = ?1",
            params![CONVERSATION_SUMMARIES_BACKFILL_META_KEY],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !summaries_backfilled {
        conn.execute(
            "INSERT OR IGNORE INTO conversation_summaries (
                phone_number, total_count, unread_count, last_message_id
             )
             SELECT phone_number,
                    COUNT(*),
                    SUM(CASE WHEN direction = 'inbound' AND read_at IS NULL THEN 1 ELSE 0 END),
                    (
                        SELECT latest.id FROM messages AS latest
                        WHERE latest.phone_number = messages.phone_number
                        ORDER BY COALESCE(julianday(latest.timestamp), julianday(latest.created_at)) DESC,
                                 latest.id DESC
                        LIMIT 1
                    )
             FROM messages
             GROUP BY phone_number",
            [],
        )?;
        conn.execute(
            "INSERT INTO meta (key, value) VALUES (?1, '1')",
            params![CONVERSATION_SUMMARIES_BACKFILL_META_KEY],
        )?;
    }
    Ok(())
}
