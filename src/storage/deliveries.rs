use anyhow::Result;
use rusqlite::{params, OptionalExtension, TransactionBehavior};

use super::codecs::{now_string, row_to_delivery};
use super::{DeliveryRow, DeliveryState, MessageStore};

impl MessageStore {
    #[cfg(test)]
    pub fn count_deliveries(&self) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        Ok(
            conn.query_row("SELECT COUNT(*) FROM forward_deliveries", [], |row| {
                row.get(0)
            })?,
        )
    }

    #[cfg(test)]
    pub fn get_delivery(&self, id: i64) -> Result<DeliveryRow> {
        let conn = self.conn.lock().unwrap();
        Ok(conn.query_row(
            "SELECT id, message_id, profile_key, state, attempt_count, next_attempt_at, lease_at, lease_token, last_error, created_at, updated_at
             FROM forward_deliveries WHERE id = ?1",
            params![id],
            row_to_delivery,
        )?)
    }

    #[cfg(test)]
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
        let lease_token = uuid::Uuid::new_v4().to_string();
        let mut conn = self.conn.lock().unwrap();
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "UPDATE forward_deliveries
             SET state = 'retry_wait', lease_at = NULL, lease_token = NULL,
                 next_attempt_at = ?1, updated_at = ?1
             WHERE state = 'in_flight' AND lease_at IS NOT NULL
               AND julianday(lease_at) <= julianday(?1)",
            params![now],
        )?;
        let mut statement = transaction.prepare(
            "SELECT id, message_id, profile_key, state, attempt_count, next_attempt_at, lease_at, lease_token, last_error, created_at, updated_at
             FROM forward_deliveries
             WHERE state IN ('pending', 'retry_wait')
               AND (next_attempt_at IS NULL
                    OR julianday(next_attempt_at) <= julianday(?1))
             ORDER BY julianday(COALESCE(next_attempt_at, created_at)) ASC, id ASC
             LIMIT ?2",
        )?;
        let rows: Vec<DeliveryRow> = statement
            .query_map(params![now, batch_size], row_to_delivery)?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        for row in &rows {
            transaction.execute(
                "UPDATE forward_deliveries
                 SET state = 'in_flight', lease_at = ?1, lease_token = ?2, updated_at = ?3
                 WHERE id = ?4 AND state IN ('pending', 'retry_wait')",
                params![lease_until, lease_token, now, row.id],
            )?;
        }
        drop(statement);
        let mut claimed_statement = transaction.prepare(
            "SELECT id, message_id, profile_key, state, attempt_count, next_attempt_at, lease_at, lease_token, last_error, created_at, updated_at
             FROM forward_deliveries WHERE lease_token = ?1 ORDER BY id ASC",
        )?;
        let claimed = claimed_statement
            .query_map(params![lease_token], row_to_delivery)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(claimed_statement);
        transaction.commit()?;
        Ok(claimed)
    }

    pub fn next_delivery_due_at(&self) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT COALESCE(next_attempt_at, created_at)
             FROM forward_deliveries
             WHERE state IN ('pending', 'retry_wait')
             ORDER BY julianday(COALESCE(next_attempt_at, created_at)) ASC, id ASC
             LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn complete_delivery(
        &self,
        id: i64,
        state: DeliveryState,
        error: Option<&str>,
        attempt_count: i64,
        next_attempt_at: Option<&str>,
        lease_token: &str,
    ) -> Result<bool> {
        let now = now_string();
        let conn = self.conn.lock().unwrap();
        let changed = conn.execute(
            "UPDATE forward_deliveries
             SET state = ?1, last_error = ?2, attempt_count = ?3, next_attempt_at = ?4,
                 lease_at = NULL, lease_token = NULL, updated_at = ?5
             WHERE id = ?6 AND state = 'in_flight' AND lease_token = ?7",
            params![
                state.as_str(),
                error,
                attempt_count,
                next_attempt_at,
                now,
                id,
                lease_token
            ],
        )?;
        Ok(changed == 1)
    }

    pub fn recover_expired_leases(&self, before: &str) -> Result<usize> {
        let now = now_string();
        let conn = self.conn.lock().unwrap();
        let count = conn.execute(
            "UPDATE forward_deliveries SET state = 'retry_wait', lease_at = NULL, lease_token = NULL, next_attempt_at = ?1, updated_at = ?1
             WHERE state = 'in_flight' AND lease_at IS NOT NULL
               AND julianday(lease_at) < julianday(?2)",
            params![now, before],
        )?;
        Ok(count)
    }
}
