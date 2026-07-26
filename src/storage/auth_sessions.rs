use anyhow::Result;
use rusqlite::{params, OptionalExtension};

use super::MessageStore;

impl MessageStore {
    pub fn create_auth_session(
        &self,
        token_hash: &[u8],
        credential_proof: &[u8],
        expires_at: i64,
        now: i64,
        max_sessions: usize,
    ) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM auth_sessions WHERE expires_at <= ?1",
            params![now],
        )?;
        let session_count: i64 =
            tx.query_row("SELECT COUNT(*) FROM auth_sessions", [], |row| row.get(0))?;
        let sessions_to_evict = session_count - max_sessions as i64 + 1;
        if sessions_to_evict > 0 {
            tx.execute(
                "DELETE FROM auth_sessions
                 WHERE token_hash IN (
                     SELECT token_hash FROM auth_sessions
                     ORDER BY expires_at ASC, rowid ASC
                     LIMIT ?1
                 )",
                params![sessions_to_evict],
            )?;
        }
        tx.execute(
            "INSERT INTO auth_sessions (token_hash, credential_proof, expires_at)
             VALUES (?1, ?2, ?3)",
            params![token_hash, credential_proof, expires_at],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn auth_session_is_valid(
        &self,
        token_hash: &[u8],
        credential_proof: &[u8],
        now: i64,
    ) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT 1 FROM auth_sessions
             WHERE token_hash = ?1 AND credential_proof = ?2 AND expires_at > ?3",
            params![token_hash, credential_proof, now],
            |_| Ok(()),
        )
        .optional()
        .map(|session| session.is_some())
        .map_err(Into::into)
    }

    pub fn delete_auth_session(&self, token_hash: &[u8]) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM auth_sessions WHERE token_hash = ?1",
            params![token_hash],
        )?;
        Ok(())
    }

    pub fn delete_all_auth_sessions(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM auth_sessions", [])?;
        Ok(())
    }

    #[cfg(test)]
    pub fn expire_auth_session(&self, token_hash: &[u8]) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE auth_sessions SET expires_at = 0 WHERE token_hash = ?1",
            params![token_hash],
        )?;
        Ok(())
    }

    #[cfg(test)]
    pub fn auth_session_count(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM auth_sessions", [], |row| row.get(0))?;
        Ok(count as usize)
    }
}
