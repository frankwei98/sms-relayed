use anyhow::Result;

use super::Store;

impl Store {
    pub async fn create_auth_session(
        &self,
        token: String,
        credential_hash: Vec<u8>,
        expires_at: i64,
        now: i64,
        max_sessions: usize,
    ) -> Result<()> {
        self.run(move |sqlite| {
            sqlite.create_auth_session(&token, &credential_hash, expires_at, now, max_sessions)
        })
        .await
    }

    pub async fn auth_session_is_valid(
        &self,
        token: String,
        credential_hash: Vec<u8>,
        now: i64,
    ) -> Result<bool> {
        self.run(move |sqlite| sqlite.auth_session_is_valid(&token, &credential_hash, now))
            .await
    }

    pub async fn delete_auth_session(&self, token: String) -> Result<()> {
        self.run(move |sqlite| sqlite.delete_auth_session(&token))
            .await
    }

    #[cfg(test)]
    pub async fn expire_auth_session(&self, token: String) -> Result<()> {
        self.run(move |sqlite| sqlite.expire_auth_session(&token))
            .await
    }

    #[cfg(test)]
    pub async fn auth_session_count(&self) -> Result<usize> {
        self.run(|sqlite| sqlite.auth_session_count()).await
    }
}
