use anyhow::Result;
use hmac::{Hmac, Mac};
use pbkdf2::pbkdf2_hmac;
use rusqlite::{params, OptionalExtension};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use uuid::Uuid;

use super::MessageStore;

const PASSWORD_HASH_ROUNDS: u32 = 100_000;

impl MessageStore {
    pub fn synchronize_auth_password(
        &self,
        password: &str,
        credential_secret: &[u8],
    ) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let existing: Option<(Vec<u8>, Vec<u8>)> = tx
            .query_row(
                "SELECT salt, verifier FROM auth_credential_state WHERE singleton = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let password_matches = existing.as_ref().is_some_and(|(salt, verifier)| {
            let candidate = password_verifier(password, salt, credential_secret);
            candidate.as_slice().ct_eq(verifier.as_slice()).into()
        });
        if !password_matches {
            let salt = Uuid::new_v4().into_bytes();
            let verifier = password_verifier(password, &salt, credential_secret);
            tx.execute(
                "INSERT INTO auth_credential_state (singleton, salt, verifier)
                 VALUES (1, ?1, ?2)
                 ON CONFLICT(singleton) DO UPDATE
                 SET salt = excluded.salt, verifier = excluded.verifier",
                params![salt.as_slice(), verifier.as_slice()],
            )?;
            tx.execute("DELETE FROM auth_sessions", [])?;
        }
        tx.commit()?;
        Ok(())
    }

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

fn password_verifier(password: &str, salt: &[u8], credential_secret: &[u8]) -> [u8; 32] {
    let mut derived = [0_u8; 32];
    pbkdf2_hmac::<Sha256>(
        password.as_bytes(),
        salt,
        PASSWORD_HASH_ROUNDS,
        &mut derived,
    );
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(credential_secret)
        .expect("HMAC accepts any key size");
    mac.update(&derived);
    mac.finalize().into_bytes().into()
}
