use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::config::AppConfig;
use crate::events::AppEvent;

use super::{ApiError, ApiResult, ApiState};

#[derive(Deserialize)]
#[serde(untagged)]
pub enum CheckConfigPayload {
    Json(Box<AppConfig>),
    Toml { toml: String },
}

#[derive(Serialize)]
pub struct ConfigSaveResponse {
    requires_restart: bool,
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
}

async fn get_config(State(state): State<ApiState>) -> ApiResult<Json<AppConfig>> {
    Ok(Json((*state.config).clone()))
}

async fn check_config(
    State(_state): State<ApiState>,
    Json(payload): Json<CheckConfigPayload>,
) -> ApiResult<Json<serde_json::Value>> {
    check_config_payload(payload).map_err(|e| ApiError::bad_request(e.to_string()))?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn save_config(
    State(state): State<ApiState>,
    Json(payload): Json<AppConfig>,
) -> ApiResult<Json<ConfigSaveResponse>> {
    payload
        .validate()
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    let config_path = state.config_path.clone();
    tokio::task::spawn_blocking(move || payload.save_secure(&config_path))
        .await
        .map_err(|error| ApiError::internal(error.to_string()))?
        .map_err(|error| ApiError::internal(error.to_string()))?;
    state.events.send(AppEvent::ConfigSaved);
    Ok(Json(ConfigSaveResponse {
        requires_restart: true,
    }))
}
