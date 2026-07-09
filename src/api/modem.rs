use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use crate::modem::{ActionResponse, ModemAction, ModemStatus};

use super::auth;
use super::{ApiError, ApiResult, ApiState};

#[derive(Deserialize)]
struct ResetRequest {
    confirm: Option<bool>,
}

pub fn routes() -> Router<ApiState> {
    Router::new()
        .route("/api/modem/status", get(status))
        .route("/api/modem/enable", post(enable))
        .route("/api/modem/disable", post(disable))
        .route("/api/modem/reset", post(reset))
}

async fn status(State(state): State<ApiState>) -> ApiResult<Json<ModemStatus>> {
    Ok(Json(state.modem.status(&state.config.app.modem_path).await))
}

async fn enable(State(state): State<ApiState>, headers: HeaderMap) -> ApiResult<Json<ActionResponse>> {
    harden_action(&headers)?;
    run_action(state, headers, ModemAction::Enable).await
}

async fn disable(State(state): State<ApiState>, headers: HeaderMap) -> ApiResult<Json<ActionResponse>> {
    harden_action(&headers)?;
    run_action(state, headers, ModemAction::Disable).await
}

async fn reset(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(req): Json<ResetRequest>,
) -> ApiResult<Json<ActionResponse>> {
    harden_action(&headers)?;
    if req.confirm != Some(true) {
        return Err(ApiError::bad_request("reset requires confirm=true"));
    }
    run_action(state, headers, ModemAction::Reset).await
}

async fn run_action(
    state: ApiState,
    headers: HeaderMap,
    action: ModemAction,
) -> ApiResult<Json<ActionResponse>> {
    let token = auth::session_token(&headers);
    state
        .modem
        .run_action(&state.config.app.modem_path, &token, action)
        .await
        .map(Json)
        .map_err(map_modem_error)
}

fn harden_action(headers: &HeaderMap) -> ApiResult<()> {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !content_type.to_ascii_lowercase().starts_with("application/json") {
        return Err(ApiError::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_media_type",
            "modem actions require application/json",
        ));
    }
    if !same_origin(headers) {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "cross_origin_rejected",
            "cross-origin modem action rejected",
        ));
    }
    Ok(())
}

fn same_origin(headers: &HeaderMap) -> bool {
    let Some(expected_host) = headers
        .get("x-forwarded-host")
        .or_else(|| headers.get(header::HOST))
        .and_then(|v| v.to_str().ok())
        .map(strip_port)
    else {
        return true;
    };

    for name in [header::ORIGIN, header::REFERER] {
        let Some(value) = headers.get(name).and_then(|v| v.to_str().ok()) else {
            continue;
        };
        let Some(host) = origin_host(value) else {
            return false;
        };
        if host != expected_host {
            return false;
        }
    }
    true
}

fn origin_host(value: &str) -> Option<String> {
    let without_scheme = value.split("://").nth(1).unwrap_or(value);
    let host = without_scheme.split('/').next()?;
    Some(strip_port(host))
}

fn strip_port(host: &str) -> String {
    host.split(':').next().unwrap_or(host).to_ascii_lowercase()
}

fn map_modem_error(err: crate::modem::ModemError) -> ApiError {
    match err.code() {
        "action_in_progress" => ApiError::new(StatusCode::CONFLICT, "action_in_progress", err.to_string()),
        "reset_rate_limited" => ApiError::new(StatusCode::TOO_MANY_REQUESTS, "reset_rate_limited", err.to_string()),
        "modem_path_unresolved" => ApiError::new(StatusCode::CONFLICT, "modem_path_unresolved", err.to_string()),
        _ => ApiError::internal(err.to_string()),
    }
}
