use std::path::{Path, PathBuf};

use axum::extract::{Query, State};
use axum::http::header::{CACHE_CONTROL, ETAG, IF_MATCH};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use similar::TextDiff;

use crate::config::{config_revision, AppConfig};
use crate::events::AppEvent;

use super::{ApiError, ApiResult, ApiState};

const RESTART_REQUIRED_HEADER: HeaderName = HeaderName::from_static("x-config-restart-required");
const CANDIDATE_REVISION_HEADER: HeaderName =
    HeaderName::from_static("x-config-candidate-revision");

#[derive(Deserialize)]
#[serde(untagged)]
pub enum CheckConfigPayload {
    Json(Box<AppConfig>),
    Toml { toml: String },
}

#[derive(Serialize)]
struct ConfigSaveResponse {
    revision: String,
    requires_restart: bool,
    restart_scheduled: bool,
    session_invalidated: bool,
}

#[derive(Serialize)]
struct ConfigPreviewResponse {
    base_revision: String,
    candidate_revision: String,
    has_changes: bool,
    diff: String,
    check: ConfigCheckResult,
    requires_restart: bool,
    password_change_pending: bool,
    warnings: Vec<&'static str>,
}

#[derive(Serialize)]
struct ConfigCheckResult {
    passed: bool,
    message: Option<String>,
}

#[derive(Default, Deserialize)]
struct SaveOptions {
    #[serde(default)]
    restart_after_save: bool,
}

struct ConfigDocument {
    content: String,
    config: AppConfig,
    revision: String,
}

pub fn check_config_payload(payload: CheckConfigPayload) -> anyhow::Result<()> {
    match payload {
        CheckConfigPayload::Json(cfg) => cfg.validate(),
        CheckConfigPayload::Toml { toml } => {
            let cfg: AppConfig = toml::from_str(&toml)?;
            cfg.validate()
        }
    }
}

pub fn routes() -> Router<ApiState> {
    Router::new()
        .route("/api/config", get(get_config).put(save_config))
        .route("/api/config/check", post(check_config))
        .route("/api/config/preview", post(preview_config))
}

async fn get_config(State(state): State<ApiState>) -> ApiResult<Response> {
    let document = load_config_document(state.config_path.clone()).await?;
    let requires_restart = document.config != *state.config;
    let mut response = Json(document.config).into_response();
    apply_document_headers(
        response.headers_mut(),
        &document.revision,
        Some(requires_restart),
    )?;
    Ok(response)
}

async fn check_config(
    State(_state): State<ApiState>,
    Json(payload): Json<CheckConfigPayload>,
) -> ApiResult<Response> {
    check_config_payload(payload).map_err(|error| ApiError::bad_request(error.to_string()))?;
    let mut response = Json(serde_json::json!({ "ok": true })).into_response();
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

async fn preview_config(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(candidate): Json<AppConfig>,
) -> ApiResult<Response> {
    let expected_revision = required_revision(&headers, IF_MATCH, "config_revision_required")?;
    let document = load_config_document(state.config_path.clone()).await?;
    ensure_revision_matches(&expected_revision, &document.revision, "config_changed")?;

    let candidate_toml = candidate
        .canonical_toml()
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let candidate_revision = config_revision(&candidate_toml);
    let check = match candidate.validate() {
        Ok(()) => ConfigCheckResult {
            passed: true,
            message: None,
        },
        Err(error) => ConfigCheckResult {
            passed: false,
            message: Some(error.to_string()),
        },
    };
    let password_change_pending = candidate.api.password != state.config.api.password;
    let requires_restart = candidate != *state.config;
    let has_changes = document.content != candidate_toml;
    let diff = if has_changes {
        TextDiff::from_lines(&document.content, &candidate_toml)
            .unified_diff()
            .context_radius(3)
            .header("current/config.toml", "candidate/config.toml")
            .to_string()
    } else {
        String::new()
    };
    let warnings = config_warnings(&state.config, &candidate);

    let body = ConfigPreviewResponse {
        base_revision: document.revision,
        candidate_revision,
        has_changes,
        diff,
        check,
        requires_restart,
        password_change_pending,
        warnings,
    };
    let mut response = Json(body).into_response();
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

async fn save_config(
    State(state): State<ApiState>,
    Query(options): Query<SaveOptions>,
    headers: HeaderMap,
    Json(candidate): Json<AppConfig>,
) -> ApiResult<Response> {
    let expected_revision = required_revision(&headers, IF_MATCH, "config_revision_required")?;
    let expected_candidate_revision = required_revision(
        &headers,
        CANDIDATE_REVISION_HEADER,
        "config_preview_required",
    )?;

    candidate
        .validate()
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let candidate_toml = candidate
        .canonical_toml()
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let candidate_revision = config_revision(&candidate_toml);
    ensure_revision_matches(
        &expected_candidate_revision,
        &candidate_revision,
        "config_preview_changed",
    )?;

    let password_changed = candidate.api.password != state.config.api.password;
    if password_changed && !options.restart_after_save {
        return Err(ApiError::new(
            StatusCode::PRECONDITION_REQUIRED,
            "password_restart_required",
            "changing api.password requires save and restart in one request",
        ));
    }

    let requires_restart = candidate != *state.config;
    let config_path = state.config_path.clone();
    {
        let _save_guard = state.config_save_lock.lock().await;
        let document = load_config_document(config_path.clone()).await?;
        ensure_revision_matches(&expected_revision, &document.revision, "config_changed")?;
        let prepared =
            tokio::task::spawn_blocking(move || candidate.prepare_secure_write(&config_path))
                .await
                .map_err(|error| ApiError::internal(error.to_string()))?
                .map_err(|error| ApiError::internal(error.to_string()))?;
        tokio::task::spawn_blocking(move || prepared.commit())
            .await
            .map_err(|error| ApiError::internal(error.to_string()))?
            .map_err(|error| ApiError::internal(error.to_string()))?;
    }

    let restart_scheduled = options.restart_after_save && super::service::schedule_restart(&state);
    state.events.send(AppEvent::ConfigSaved);

    let body = ConfigSaveResponse {
        revision: candidate_revision.clone(),
        requires_restart,
        restart_scheduled,
        session_invalidated: false,
    };
    let mut response = Json(body).into_response();
    apply_document_headers(
        response.headers_mut(),
        &candidate_revision,
        Some(requires_restart),
    )?;
    Ok(response)
}

fn load_config_document_sync(path: &Path) -> anyhow::Result<ConfigDocument> {
    let content = std::fs::read_to_string(path)?;
    let config = toml::from_str(&content)?;
    let revision = config_revision(&content);
    Ok(ConfigDocument {
        content,
        config,
        revision,
    })
}

async fn load_config_document(path: PathBuf) -> ApiResult<ConfigDocument> {
    tokio::task::spawn_blocking(move || load_config_document_sync(&path))
        .await
        .map_err(|error| ApiError::internal(error.to_string()))?
        .map_err(|error| ApiError::internal(error.to_string()))
}

fn required_revision(
    headers: &HeaderMap,
    name: HeaderName,
    code: &'static str,
) -> ApiResult<String> {
    let value = headers.get(name).ok_or_else(|| {
        ApiError::new(
            StatusCode::PRECONDITION_REQUIRED,
            code,
            "configuration revision header is required",
        )
    })?;
    let value = value
        .to_str()
        .map_err(|_| ApiError::bad_request("configuration revision header is invalid"))?;
    let normalized = value
        .strip_prefix("W/")
        .unwrap_or(value)
        .trim_matches('"')
        .trim();
    if normalized.is_empty() {
        return Err(ApiError::bad_request(
            "configuration revision header is empty",
        ));
    }
    Ok(normalized.to_string())
}

fn ensure_revision_matches(expected: &str, actual: &str, code: &'static str) -> ApiResult<()> {
    if expected == actual {
        Ok(())
    } else {
        Err(ApiError::new(
            StatusCode::PRECONDITION_FAILED,
            code,
            "configuration changed; reload before saving",
        ))
    }
}

fn apply_document_headers(
    headers: &mut HeaderMap,
    revision: &str,
    requires_restart: Option<bool>,
) -> ApiResult<()> {
    let etag = HeaderValue::from_str(&format!("\"{revision}\""))
        .map_err(|error| ApiError::internal(error.to_string()))?;
    headers.insert(ETAG, etag);
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if let Some(requires_restart) = requires_restart {
        headers.insert(
            RESTART_REQUIRED_HEADER,
            HeaderValue::from_static(if requires_restart { "true" } else { "false" }),
        );
    }
    Ok(())
}

fn config_warnings(running: &AppConfig, candidate: &AppConfig) -> Vec<&'static str> {
    let mut warnings = Vec::new();
    if candidate.api.password != running.api.password {
        warnings.push("password_change");
    }
    if running.api.enabled && !candidate.api.enabled {
        warnings.push("api_disable");
    }
    if candidate.api.bind != running.api.bind
        || candidate.api.port != running.api.port
        || candidate.api.enable_ipv6 != running.api.enable_ipv6
    {
        warnings.push("api_endpoint_change");
    }
    if candidate.api.trusted_proxies != running.api.trusted_proxies {
        warnings.push("trusted_proxies_change");
    }
    if candidate.api.database_path != running.api.database_path {
        warnings.push("database_path_change");
    }
    if candidate
        .channels
        .webhook
        .values()
        .any(|profile| profile.method == crate::config::WebhookMethod::Get)
    {
        warnings.push("webhook_get");
    }
    warnings
}
