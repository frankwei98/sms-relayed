use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

use super::{compute_inbound_dedupe_key, MessageStore};

impl MessageStore {
    pub fn backfill_dedupe_keys(&self) -> Result<usize> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let count = backfill_dedupe_keys_on(&tx)?;
        tx.commit()?;
        Ok(count)
    }

    pub fn get_meta(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT value FROM meta WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
        Ok(())
    }
}

pub(super) fn backfill_dedupe_keys_on(conn: &Connection) -> Result<usize> {
    let fingerprint: Option<String> = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'modem_fingerprint'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let Some(fingerprint) = fingerprint else {
        return Ok(0);
    };
    let mut statement = conn.prepare(
        "SELECT id, phone_number, body, timestamp FROM messages
         WHERE direction = 'inbound' AND source = 'modem' AND inbound_dedupe_key IS NULL
         ORDER BY id ASC",
    )?;
    let rows: Vec<(i64, String, String, String)> = statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);

    let mut exists_statement =
        conn.prepare("SELECT COUNT(*) > 0 FROM messages WHERE inbound_dedupe_key = ?1")?;
    let mut count = 0;
    for (id, phone, body, timestamp) in &rows {
        let dedupe_key = compute_inbound_dedupe_key(&fingerprint, timestamp, phone, body);
        let exists: bool = exists_statement.query_row(params![dedupe_key], |row| row.get(0))?;
        if !exists {
            conn.execute(
                "UPDATE messages SET inbound_dedupe_key = ?1 WHERE id = ?2",
                params![dedupe_key, id],
            )?;
            count += 1;
        }
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backfill_without_modem_fingerprint_is_a_no_op() {
        let store = MessageStore::open_in_memory().unwrap();

        assert_eq!(store.backfill_dedupe_keys().unwrap(), 0);
    }

    #[test]
    fn backfill_propagates_modem_fingerprint_query_errors() {
        let store = MessageStore::open_in_memory().unwrap();
        {
            let conn = store.conn.lock().unwrap();
            conn.execute("DROP TABLE meta", []).unwrap();
        }

        let error = store.backfill_dedupe_keys().unwrap_err();

        assert!(
            error.to_string().contains("no such table: meta"),
            "expected the fingerprint query error, got: {error:#}"
        );
    }
}
