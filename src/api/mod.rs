pub mod auth;
pub mod config;
pub mod messages;
pub mod service;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use axum::http::StatusCode;
use axum::middleware;
use axum::response::{IntoResponse, Response};
use axum::routing::Router;
use axum::Json;
use serde::Serialize;

use crate::config::AppConfig;
use crate::events::EventBus;
use crate::storage::MessageStore;

#[derive(Clone)]
pub struct ApiState {
    pub config: Arc<AppConfig>,
    pub config_path: PathBuf,
    pub store: MessageStore,
    pub events: EventBus,
    pub started_at: Instant,
    pub sessions: auth::SessionStore,
}

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    error: ErrorDetail<'a>,
}

#[derive(Serialize)]
struct ErrorDetail<'a> {
    code: &'a str,
    message: &'a str,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized", message)
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad_request", message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = ErrorBody {
            error: ErrorDetail {
                code: self.code,
                message: &self.message,
            },
        };
        (self.status, Json(body)).into_response()
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

pub fn router(state: ApiState) -> Router {
    let sessions = state.sessions.clone();
    let protected = Router::new()
        .merge(messages::routes())
        .merge(config::routes())
        .merge(service::routes())
        .layer(middleware::from_fn(
            move |req: axum::extract::Request, next: middleware::Next| {
                let sessions = sessions.clone();
                async move {
                    let token = auth::session_token(req.headers());
                    if !sessions.is_valid(&token) {
                        return ApiError::unauthorized("authentication required").into_response();
                    }
                    next.run(req).await
                }
            },
        ));

    let auth_routes = auth::routes();

    Router::new()
        .merge(auth_routes)
        .merge(protected)
        .with_state(state)
        .fallback(crate::assets::serve)
}

pub async fn serve(state: ApiState) -> anyhow::Result<()> {
    let addr = format!("{}:{}", state.config.api.bind, state.config.api.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    log::info!("web api listening on {}", addr);
    axum::serve(listener, router(state)).await?;
    Ok(())
}

impl From<anyhow::Error> for ApiError {
    fn from(err: anyhow::Error) -> Self {
        ApiError::internal(err.to_string())
    }
}

impl From<rusqlite::Error> for ApiError {
    fn from(err: rusqlite::Error) -> Self {
        ApiError::internal(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::auth::SessionStore;
    use super::config::{check_config_payload, CheckConfigPayload};
    use crate::config::AppConfig;

    #[test]
    fn login_cookie_uses_p2_session_contract() {
        let sessions = SessionStore::default();
        let cookie = sessions.login_cookie(false);

        assert!(cookie.starts_with("sms-relayed-session="));
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Lax"));
        assert!(cookie.contains("Path=/"));
        assert!(cookie.contains("Max-Age=604800"));
        assert!(!cookie.contains("Secure"));
    }

    #[test]
    fn login_cookie_is_secure_when_request_is_https() {
        let sessions = SessionStore::default();
        let cookie = sessions.login_cookie(true);
        assert!(cookie.contains("Secure"));
    }

    #[test]
    fn session_tokens_expire_after_seven_days() {
        let sessions = SessionStore::default();
        let token = sessions.create_session();
        assert!(sessions.is_valid(&token));

        sessions.expire_for_test(&token);
        assert!(!sessions.is_valid(&token));
    }

    #[test]
    fn config_check_accepts_json_or_toml_and_rejects_bad_config() {
        let mut cfg = AppConfig::default();
        cfg.api.password = "secret".to_string();

        assert!(check_config_payload(CheckConfigPayload::Json(cfg.clone())).is_ok());

        let toml = toml::to_string_pretty(&cfg).unwrap();
        assert!(check_config_payload(CheckConfigPayload::Toml { toml }).is_ok());

        cfg.api.password.clear();
        let err = check_config_payload(CheckConfigPayload::Json(cfg))
            .unwrap_err()
            .to_string();
        assert!(err.contains("api.password"));
    }
}
