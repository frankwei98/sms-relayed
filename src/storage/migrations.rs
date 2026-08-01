use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

use super::{metadata::backfill_dedupe_keys_on, CONVERSATION_SUMMARIES_BACKFILL_META_KEY};

pub(super) fn migrate_existing_schema(conn: &Connection) -> Result<()> {
    let has_favorite_at: bool = conn
        .prepare(
            "SELECT COUNT(*) FROM pragma_table_info('messages')
             WHERE name = 'favorite_at'",
        )?
        .query_row([], |row| row.get::<_, i64>(0))
        .map(|count| count > 0)?;
    if !has_favorite_at {
        conn.execute("ALTER TABLE messages ADD COLUMN favorite_at TEXT NULL", [])?;
    }
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_messages_favorite_at
         ON messages(julianday(favorite_at) DESC, id DESC)
         WHERE favorite_at IS NOT NULL",
        [],
    )?;

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

    // Existing databases created `messages_delete_conversation_summary` via
    // `CREATE TRIGGER IF NOT EXISTS`, so the updated body in the schema block
    // (which also clears the conversation's pin when the last message is
    // deleted) will not replace it. Drop and recreate so those databases get
    // the fix; this is idempotent for databases that already have the new body.
    conn.execute(
        "DROP TRIGGER IF EXISTS messages_delete_conversation_summary",
        [],
    )?;
    conn.execute(
        "CREATE TRIGGER messages_delete_conversation_summary
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
             DELETE FROM conversation_pins
             WHERE phone_number = OLD.phone_number
               AND NOT EXISTS (SELECT 1 FROM messages WHERE phone_number = OLD.phone_number);
         END",
        [],
    )?;

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
