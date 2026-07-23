use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration as StdDuration, Instant};

use axum::extract::{ConnectInfo, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use super::{ApiError, ApiState};

pub const SESSION_COOKIE: &str = "sms-relayed-session";
const SESSION_DAYS: i64 = 7;
const MAX_SESSIONS: usize = 256;
const MAX_LOGIN_FAILURES: u32 = 5;
const LOGIN_FAILURE_WINDOW: StdDuration = StdDuration::from_secs(5 * 60);

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

#[derive(Clone, Default)]
pub struct SessionStore {
    inner: Arc<Mutex<HashMap<String, OffsetDateTime>>>,
    login_failures: Arc<Mutex<HashMap<IpAddr, LoginFailures>>>,
}

impl SessionStore {
    pub fn create_session(&self) -> String {
        let token = Uuid::new_v4().to_string();
        let expires = OffsetDateTime::now_utc() + Duration::days(SESSION_DAYS);
        let mut guard = self.inner.lock().unwrap();
        // Prune expired before insert
        prune_expired(&mut guard);
        // Enforce capacity
        if guard.len() >= MAX_SESSIONS {
            evict_oldest(&mut guard);
        }
        guard.insert(token.clone(), expires);
        token
    }

    pub fn is_valid(&self, token: &str) -> bool {
        let mut guard = self.inner.lock().unwrap();
        prune_expired(&mut guard);
        guard
            .get(token)
            .is_some_and(|expires| *expires > OffsetDateTime::now_utc())
    }

    pub fn remove(&self, token: &str) {
        self.inner.lock().unwrap().remove(token);
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

    pub fn login_cookie(&self, is_https: bool) -> String {
        let token = self.create_session();
        self.cookie_string(&token, is_https)
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
    pub fn expire_for_test(&self, token: &str) {
        if let Some(expires) = self.inner.lock().unwrap().get_mut(token) {
            *expires = OffsetDateTime::UNIX_EPOCH;
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().len()
    }
}

fn prune_expired(sessions: &mut HashMap<String, OffsetDateTime>) {
    let now = OffsetDateTime::now_utc();
    sessions.retain(|_, expires| *expires > now);
}

fn evict_oldest(sessions: &mut HashMap<String, OffsetDateTime>) {
    if let Some(oldest) = sessions
        .iter()
        .min_by_key(|(_, expires)| *expires)
        .map(|(k, _)| k.clone())
    {
        sessions.remove(&oldest);
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
    let cookie = state.sessions.login_cookie(forwarded_https(&headers));
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
) -> (StatusCode, HeaderMap, Json<AuthResponse>) {
    let token = session_token(&headers);
    state.sessions.remove(&token);
    let mut hdrs = HeaderMap::new();
    let cookie = state.sessions.clear_cookie(forwarded_https(&headers));
    hdrs.insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie).unwrap_or_else(|_| HeaderValue::from_static("")),
    );
    (
        StatusCode::OK,
        hdrs,
        Json(AuthResponse {
            authenticated: false,
        }),
    )
}

async fn me(State(state): State<ApiState>, headers: HeaderMap) -> Json<AuthResponse> {
    Json(AuthResponse {
        authenticated: state.sessions.is_valid(&session_token(&headers)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_store_prunes_expired_and_enforces_capacity() {
        let store = SessionStore::default();
        // Create more than MAX_SESSIONS tokens
        for i in 0..MAX_SESSIONS + 10 {
            let token = store.create_session();
            if i < MAX_SESSIONS {
                assert!(store.is_valid(&token));
            }
        }
        let len = store.len();
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
}
