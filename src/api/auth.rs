use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration as StdDuration, Instant};

use axum::extract::{ConnectInfo, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::persistence::Store;

use super::{ApiError, ApiState};

pub const SESSION_COOKIE: &str = "sms-relayed-session";
const SESSION_DAYS: i64 = 7;
const MAX_SESSIONS: usize = 256;
const MAX_LOGIN_FAILURES: u32 = 5;
const LOGIN_FAILURE_WINDOW: StdDuration = StdDuration::from_secs(5 * 60);
const CREDENTIAL_SECRET_BYTES: usize = 32;

#[derive(Clone, Copy)]
struct LoginFailures {
    window_started: Instant,
    count: u32,
    locked_until: Option<Instant>,
}

enum LoginResult {
    Authenticated,
    Rejected,
    RateLimited,
}

#[derive(Clone)]
pub struct SessionStore {
    store: Store,
    password: Arc<str>,
    login_failures: Arc<Mutex<HashMap<IpAddr, LoginFailures>>>,
    #[cfg(test)]
    invalidate_all_failure: Arc<AtomicBool>,
}

impl SessionStore {
    pub async fn open(store: Store, password: &str, database_path: &Path) -> anyhow::Result<Self> {
        let secret_path = credential_secret_path(database_path);
        let credential_secret =
            tokio::task::spawn_blocking(move || load_or_create_credential_secret(&secret_path))
                .await??;
        store
            .synchronize_auth_password(password.to_string(), credential_secret.to_vec())
            .await?;
        Ok(Self::new(store, password))
    }

    fn new(store: Store, password: &str) -> Self {
        Self {
            store,
            password: Arc::from(password),
            login_failures: Arc::new(Mutex::new(HashMap::new())),
            #[cfg(test)]
            invalidate_all_failure: Arc::new(AtomicBool::new(false)),
        }
    }

    pub async fn create_session(&self) -> anyhow::Result<String> {
        let token = Uuid::new_v4().to_string();
        let expires = OffsetDateTime::now_utc() + Duration::days(SESSION_DAYS);
        self.store
            .create_auth_session(
                token_hash(&token).to_vec(),
                credential_proof(&token, &self.password).to_vec(),
                expires.unix_timestamp(),
                OffsetDateTime::now_utc().unix_timestamp(),
                MAX_SESSIONS,
            )
            .await?;
        Ok(token)
    }

    pub async fn is_valid(&self, token: &str) -> anyhow::Result<bool> {
        self.store
            .auth_session_is_valid(
                token_hash(token).to_vec(),
                credential_proof(token, &self.password).to_vec(),
                OffsetDateTime::now_utc().unix_timestamp(),
            )
            .await
    }

    pub async fn remove(&self, token: &str) -> anyhow::Result<()> {
        self.store
            .delete_auth_session(token_hash(token).to_vec())
            .await
    }

    pub async fn invalidate_all(&self) -> anyhow::Result<()> {
        #[cfg(test)]
        if self.invalidate_all_failure.swap(false, Ordering::SeqCst) {
            anyhow::bail!("injected session invalidation failure");
        }
        self.store.delete_all_auth_sessions().await
    }

    #[cfg(test)]
    pub(crate) fn fail_next_invalidate_all(&self) {
        self.invalidate_all_failure.store(true, Ordering::SeqCst);
    }

    fn authenticate(&self, peer: IpAddr, password: &str, expected_password: &str) -> LoginResult {
        let now = Instant::now();
        let mut failures = self.login_failures.lock().unwrap();
        failures.retain(|_, failure| {
            failure.locked_until.is_some_and(|until| until > now)
                || now.duration_since(failure.window_started) < LOGIN_FAILURE_WINDOW
        });

        if failures
            .get(&peer)
            .and_then(|failure| failure.locked_until)
            .is_some_and(|until| until > now)
        {
            return LoginResult::RateLimited;
        }

        if password_matches(password, expected_password) {
            failures.remove(&peer);
            return LoginResult::Authenticated;
        }

        let failure = failures.entry(peer).or_insert(LoginFailures {
            window_started: now,
            count: 0,
            locked_until: None,
        });
        if now.duration_since(failure.window_started) >= LOGIN_FAILURE_WINDOW {
            *failure = LoginFailures {
                window_started: now,
                count: 0,
                locked_until: None,
            };
        }
        failure.count += 1;
        if failure.count >= MAX_LOGIN_FAILURES {
            failure.locked_until = Some(now + LOGIN_FAILURE_WINDOW);
        }
        LoginResult::Rejected
    }

    pub async fn login_cookie(&self, is_https: bool) -> anyhow::Result<String> {
        let token = self.create_session().await?;
        Ok(self.cookie_string(&token, is_https))
    }

    pub fn clear_cookie(&self, is_https: bool) -> String {
        let mut cookie = format!(
            "{}=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0",
            SESSION_COOKIE
        );
        if is_https {
            cookie.push_str("; Secure");
        }
        cookie
    }

    fn cookie_string(&self, token: &str, is_https: bool) -> String {
        let mut cookie = format!(
            "{}={}; HttpOnly; SameSite=Lax; Path=/; Max-Age=604800",
            SESSION_COOKIE, token
        );
        if is_https {
            cookie.push_str("; Secure");
        }
        cookie
    }

    #[cfg(test)]
    pub async fn expire_for_test(&self, token: &str) -> anyhow::Result<()> {
        self.store
            .expire_auth_session(token_hash(token).to_vec())
            .await
    }

    #[cfg(test)]
    pub async fn len(&self) -> anyhow::Result<usize> {
        self.store.auth_session_count().await
    }
}

fn credential_secret_path(database_path: &Path) -> PathBuf {
    let mut path = database_path.as_os_str().to_os_string();
    path.push(".auth-key");
    path.into()
}

fn load_or_create_credential_secret(path: &Path) -> anyhow::Result<[u8; CREDENTIAL_SECRET_BYTES]> {
    match fs::read(path) {
        Ok(secret) => return parse_credential_secret(secret),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }

    let mut secret = [0_u8; CREDENTIAL_SECRET_BYTES];
    secret[..16].copy_from_slice(Uuid::new_v4().as_bytes());
    secret[16..].copy_from_slice(Uuid::new_v4().as_bytes());
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(path) {
        Ok(mut file) => {
            file.write_all(&secret)?;
            file.sync_all()?;
            Ok(secret)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            parse_credential_secret(fs::read(path)?)
        }
        Err(error) => Err(error.into()),
    }
}

fn parse_credential_secret(secret: Vec<u8>) -> anyhow::Result<[u8; CREDENTIAL_SECRET_BYTES]> {
    secret.try_into().map_err(|secret: Vec<u8>| {
        anyhow::anyhow!(
            "authentication key must be {CREDENTIAL_SECRET_BYTES} bytes, got {}",
            secret.len()
        )
    })
}

fn token_hash(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

fn credential_proof(token: &str, password: &str) -> [u8; 32] {
    let mut mac =
        <Hmac<Sha256> as Mac>::new_from_slice(token.as_bytes()).expect("HMAC accepts any key size");
    mac.update(password.as_bytes());
    mac.finalize().into_bytes().into()
}

#[cfg(test)]
impl Default for SessionStore {
    fn default() -> Self {
        Self::new(Store::open_in_memory().unwrap(), "test-password")
    }
}

fn forwarded_https(headers: &HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.eq_ignore_ascii_case("https"))
        .unwrap_or(false)
}

fn password_matches(password: &str, expected_password: &str) -> bool {
    let password_digest: [u8; 32] = Sha256::digest(password.as_bytes()).into();
    let expected_digest: [u8; 32] = Sha256::digest(expected_password.as_bytes()).into();
    password_digest.ct_eq(&expected_digest).into()
}

pub fn session_token(headers: &HeaderMap) -> String {
    let Some(cookie) = headers.get(header::COOKIE).and_then(|v| v.to_str().ok()) else {
        return String::new();
    };
    for part in cookie.split(';') {
        let part = part.trim();
        if let Some(rest) = part.strip_prefix(&format!("{}=", SESSION_COOKIE)) {
            return rest.to_string();
        }
    }
    String::new()
}

#[derive(Deserialize)]
pub struct LoginRequest {
    password: String,
}

#[derive(Serialize)]
pub struct AuthResponse {
    authenticated: bool,
}

pub fn routes() -> Router<ApiState> {
    Router::new()
        .route("/api/auth/login", post(login))
        .route("/api/auth/logout", post(logout))
        .route("/api/auth/me", get(me))
}

async fn login(
    State(state): State<ApiState>,
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<LoginRequest>,
) -> Result<(StatusCode, HeaderMap, Json<AuthResponse>), ApiError> {
    match state
        .sessions
        .authenticate(peer.ip(), &req.password, &state.config.api.password)
    {
        LoginResult::Authenticated => {}
        LoginResult::Rejected => return Err(ApiError::unauthorized("invalid password")),
        LoginResult::RateLimited => {
            return Err(ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "login_rate_limited",
                "too many login attempts",
            ));
        }
    }
    let mut hdrs = HeaderMap::new();
    let cookie = state
        .sessions
        .login_cookie(forwarded_https(&headers))
        .await
        .map_err(session_storage_error)?;
    hdrs.insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie).unwrap_or_else(|_| HeaderValue::from_static("")),
    );
    Ok((
        StatusCode::OK,
        hdrs,
        Json(AuthResponse {
            authenticated: true,
        }),
    ))
}

async fn logout(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<(StatusCode, HeaderMap, Json<AuthResponse>), ApiError> {
    let token = session_token(&headers);
    state
        .sessions
        .remove(&token)
        .await
        .map_err(session_storage_error)?;
    let mut hdrs = HeaderMap::new();
    let cookie = state.sessions.clear_cookie(forwarded_https(&headers));
    hdrs.insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie).unwrap_or_else(|_| HeaderValue::from_static("")),
    );
    Ok((
        StatusCode::OK,
        hdrs,
        Json(AuthResponse {
            authenticated: false,
        }),
    ))
}

async fn me(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Json<AuthResponse>, ApiError> {
    let authenticated = state
        .sessions
        .is_valid(&session_token(&headers))
        .await
        .map_err(session_storage_error)?;
    Ok(Json(AuthResponse { authenticated }))
}

pub(super) fn session_storage_error(error: anyhow::Error) -> ApiError {
    log::error!("session storage operation failed: {error:#}");
    ApiError::internal("session storage unavailable")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn session_store_prunes_expired_and_enforces_capacity() {
        let store = SessionStore::default();
        // Create more than MAX_SESSIONS tokens
        for i in 0..MAX_SESSIONS + 10 {
            let token = store.create_session().await.unwrap();
            if i < MAX_SESSIONS {
                assert!(store.is_valid(&token).await.unwrap());
            }
        }
        let len = store.len().await.unwrap();
        assert!(len <= MAX_SESSIONS, "len {} > max {}", len, MAX_SESSIONS);
    }

    #[test]
    fn password_comparison_requires_an_exact_match() {
        assert!(password_matches(
            "correct horse battery staple",
            "correct horse battery staple"
        ));
        assert!(!password_matches(
            "correct horse",
            "correct horse battery staple"
        ));
        assert!(!password_matches(
            "correct horse battery stapler",
            "correct horse battery staple"
        ));
    }

    #[tokio::test]
    async fn session_remains_valid_after_store_is_reopened() {
        let database_path = std::env::temp_dir().join(format!(
            "sms-relayed-session-restart-{}.sqlite",
            Uuid::new_v4()
        ));
        let sessions = SessionStore::open(
            crate::persistence::Store::open(&database_path)
                .await
                .unwrap(),
            "same-password",
            &database_path,
        )
        .await
        .unwrap();
        let token = sessions.create_session().await.unwrap();
        drop(sessions);

        let restarted_sessions = SessionStore::open(
            crate::persistence::Store::open(&database_path)
                .await
                .unwrap(),
            "same-password",
            &database_path,
        )
        .await
        .unwrap();

        assert!(restarted_sessions.is_valid(&token).await.unwrap());

        drop(restarted_sessions);
        for path in [
            database_path.clone(),
            database_path.with_extension("sqlite-shm"),
            database_path.with_extension("sqlite-wal"),
            credential_secret_path(&database_path),
        ] {
            let _ = std::fs::remove_file(path);
        }
    }

    #[tokio::test]
    async fn changing_the_api_password_invalidates_existing_sessions() {
        let store = crate::persistence::Store::open_in_memory().unwrap();
        let config_path = std::env::temp_dir().join(format!(
            "sms-relayed-password-restart-{}.toml",
            Uuid::new_v4()
        ));
        let sessions = SessionStore::open(store.clone(), "old-password", &config_path)
            .await
            .unwrap();
        let token = sessions.create_session().await.unwrap();

        let sessions_after_password_change =
            SessionStore::open(store.clone(), "new-password", &config_path)
                .await
                .unwrap();

        assert!(!sessions_after_password_change
            .is_valid(&token)
            .await
            .unwrap());

        let sessions_after_password_reuse = SessionStore::open(store, "old-password", &config_path)
            .await
            .unwrap();
        assert!(!sessions_after_password_reuse
            .is_valid(&token)
            .await
            .unwrap());
        let _ = fs::remove_file(credential_secret_path(&config_path));
    }

    #[tokio::test]
    async fn invalidated_sessions_do_not_return_when_the_password_is_reused() {
        let store = crate::persistence::Store::open_in_memory().unwrap();
        let sessions = SessionStore::new(store.clone(), "reused-password");
        let token = sessions.create_session().await.unwrap();

        sessions.invalidate_all().await.unwrap();
        let sessions_after_password_reuse = SessionStore::new(store, "reused-password");

        assert!(!sessions_after_password_reuse
            .is_valid(&token)
            .await
            .unwrap());
    }
}
