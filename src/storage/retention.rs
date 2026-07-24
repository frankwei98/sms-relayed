use anyhow::Result;
use rusqlite::params;
use time::OffsetDateTime;

use super::MessageStore;

impl MessageStore {
    #[allow(dead_code)]
    pub fn run_retention(&self, max_age_days: u64, batch_size: u32) -> Result<usize> {
        let cutoff = (OffsetDateTime::now_utc() - time::Duration::days(max_age_days as i64))
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default();
        let mut conn = self.conn.lock().unwrap();
        let transaction = conn.transaction()?;
        let mut statement = transaction.prepare(
            "SELECT m.id FROM messages m
             WHERE julianday(m.timestamp) < julianday(?1)
               AND m.status IN ('received', 'sent', 'failed')
               AND NOT EXISTS (
                   SELECT 1 FROM forward_deliveries d
                   WHERE d.message_id = m.id
                     AND d.state IN ('pending', 'in_flight', 'retry_wait')
               )
             LIMIT ?2",
        )?;
        let ids: Vec<i64> = statement
            .query_map(params![cutoff, batch_size], |row| row.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let count = ids.len();
        drop(statement);
        for id in &ids {
            transaction.execute("DELETE FROM messages WHERE id = ?1", params![id])?;
        }
        transaction.commit()?;
        Ok(count)
    }
}
