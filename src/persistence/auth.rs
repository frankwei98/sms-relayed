use anyhow::Result;

use super::Store;

impl Store {
    pub async fn synchronize_auth_password(
        &self,
        password: String,
        credential_secret: Vec<u8>,
    ) -> Result<()> {
        self.run(move |sqlite| sqlite.synchronize_auth_password(&password, &credential_secret))
            .await
    }

    pub async fn create_auth_session(
        &self,
        token_hash: Vec<u8>,
        credential_proof: Vec<u8>,
        expires_at: i64,
        now: i64,
        max_sessions: usize,
    ) -> Result<()> {
        self.run(move |sqlite| {
            sqlite.create_auth_session(
                &token_hash,
                &credential_proof,
                expires_at,
                now,
                max_sessions,
            )
        })
        .await
    }

    pub async fn auth_session_is_valid(
        &self,
        token_hash: Vec<u8>,
        credential_proof: Vec<u8>,
        now: i64,
    ) -> Result<bool> {
        self.run(move |sqlite| sqlite.auth_session_is_valid(&token_hash, &credential_proof, now))
            .await
    }

    pub async fn delete_auth_session(&self, token_hash: Vec<u8>) -> Result<()> {
        self.run(move |sqlite| sqlite.delete_auth_session(&token_hash))
            .await
    }

    pub async fn delete_all_auth_sessions(&self) -> Result<()> {
        self.run(|sqlite| sqlite.delete_all_auth_sessions()).await
    }

    #[cfg(test)]
    pub async fn expire_auth_session(&self, token_hash: Vec<u8>) -> Result<()> {
        self.run(move |sqlite| sqlite.expire_auth_session(&token_hash))
            .await
    }

    #[cfg(test)]
    pub async fn auth_session_count(&self) -> Result<usize> {
        self.run(|sqlite| sqlite.auth_session_count()).await
    }
}
