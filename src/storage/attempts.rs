use anyhow::Result;
use rusqlite::{params, Connection};

use super::codecs::{now_string, outcome_to_str, row_to_attempt_sample};
use super::{DeliveryCompletion, ForwardAttemptSample, MessageStore, NewForwardAttemptSample};

impl MessageStore {
    #[cfg(test)]
    pub fn record_forward_attempt(
        &self,
        sample: NewForwardAttemptSample,
    ) -> Result<ForwardAttemptSample> {
        let now = now_string();
        let mut conn = self.conn.lock().unwrap();
        let transaction = conn.transaction()?;
        let id = insert_forward_attempt_on(&transaction, &sample, &now)?;
        prune_forward_attempts_on(&transaction, &sample.profile_key)?;
        transaction.commit()?;
        Ok(ForwardAttemptSample {
            id,
            profile_key: sample.profile_key,
            delivery_id: sample.delivery_id,
            attempt_number: sample.attempt_number,
            started_at: sample.started_at,
            completed_at: sample.completed_at,
            latency_ms: sample.latency_ms,
            dispatch_delay_ms: Some(sample.dispatch_delay_ms),
            outcome: sample.outcome,
            error_code: sample.error_code,
        })
    }

    pub fn list_forward_attempt_profiles(&self) -> Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let mut statement = conn.prepare(
            "SELECT DISTINCT profile_key FROM forward_attempt_samples ORDER BY profile_key",
        )?;
        let rows = statement.query_map([], |row| row.get(0))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn list_forward_attempts(
        &self,
        profile_key: &str,
        limit: u32,
    ) -> Result<Vec<ForwardAttemptSample>> {
        let conn = self.conn.lock().unwrap();
        let mut statement = conn.prepare(
            "SELECT id, profile_key, delivery_id, attempt_number, started_at, completed_at,
                    latency_ms, dispatch_delay_ms, outcome, error_code
             FROM forward_attempt_samples
             WHERE profile_key = ?1
             ORDER BY completed_at DESC, id DESC
             LIMIT ?2",
        )?;
        let rows = statement.query_map(params![profile_key, limit], row_to_attempt_sample)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn complete_delivery_with_attempt(
        &self,
        completion: DeliveryCompletion<'_>,
    ) -> Result<bool> {
        let DeliveryCompletion {
            id,
            state,
            error,
            attempt_count,
            next_attempt_at,
            lease_token,
            sample,
        } = completion;
        let now = now_string();
        let mut conn = self.conn.lock().unwrap();
        let transaction = conn.transaction()?;
        insert_forward_attempt_on(&transaction, &sample, &now)?;
        prune_forward_attempts_on(&transaction, &sample.profile_key)?;
        let changed = transaction.execute(
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
        transaction.commit()?;
        Ok(changed == 1)
    }
}

fn prune_forward_attempts_on(conn: &Connection, profile_key: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM forward_attempt_samples WHERE id IN (
            SELECT id FROM forward_attempt_samples
            WHERE profile_key = ?1
            ORDER BY completed_at DESC, id DESC
            LIMIT -1 OFFSET 5
        )",
        params![profile_key],
    )?;
    Ok(())
}

fn insert_forward_attempt_on(
    conn: &Connection,
    sample: &NewForwardAttemptSample,
    created_at: &str,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO forward_attempt_samples
         (profile_key, delivery_id, attempt_number, started_at, completed_at,
          latency_ms, dispatch_delay_ms, outcome, error_code, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            &sample.profile_key,
            sample.delivery_id,
            sample.attempt_number,
            &sample.started_at,
            &sample.completed_at,
            sample.latency_ms,
            sample.dispatch_delay_ms,
            outcome_to_str(&sample.outcome),
            &sample.error_code,
            created_at,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}
