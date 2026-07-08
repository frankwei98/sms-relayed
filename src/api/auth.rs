use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use super::{ApiError, ApiState};

pub const SESSION_COOKIE: &str = "sms-relayed-session";
const SESSION_DAYS: i64 = 7;

#[derive(Clone, Default)]
pub struct SessionStore {
    inner: Arc<Mutex<HashMap<String, OffsetDateTime>>>,
}

impl SessionStore {
    pub fn create_session(&self) -> String {
        let token = Uuid::new_v4().to_string();
        let expires = OffsetDateTime::now_utc() + Duration::days(SESSION_DAYS);
        self.inner.lock().unwrap().insert(token.clone(), expires);
        token
    }

    pub fn is_valid(&self, token: &str) -> bool {
        let guard = self.inner.lock().unwrap();
        match guard.get(token) {
            Some(expires) => *expires > OffsetDateTime::now_utc(),
            None => false,
        }
    }

    pub fn remove(&self, token: &str) {
        self.inner.lock().unwrap().remove(token);
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

    pub fn expire_for_test(&self, token: &str) {
        if let Some(expires) = self.inner.lock().unwrap().get_mut(token) {
            *expires = OffsetDateTime::UNIX_EPOCH;
        }
    }
}

fn forwarded_https(headers: &HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.eq_ignore_ascii_case("https"))
        .unwrap_or(false)
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
    headers: HeaderMap,
    Json(req): Json<LoginRequest>,
) -> Result<(StatusCode, HeaderMap, Json<AuthResponse>), ApiError> {
    if req.password != state.config.api.password {
        return Err(ApiError::unauthorized("invalid password"));
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
