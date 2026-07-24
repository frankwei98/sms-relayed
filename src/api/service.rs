use std::process::Command;
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;

use crate::events::AppEvent;

use super::{ApiResult, ApiState};

#[derive(Serialize)]
struct StatusResponse {
    version: &'static str,
    uptime_seconds: u64,
    api_bind: String,
    api_port: u16,
    database_path: String,
}

pub fn routes() -> Router<ApiState> {
    Router::new()
        .route("/api/status", get(status))
        .route("/api/service/restart", post(restart))
}

async fn status(State(state): State<ApiState>) -> ApiResult<Json<StatusResponse>> {
    Ok(Json(StatusResponse {
        version: env!("CARGO_PKG_VERSION"),
        uptime_seconds: state.started_at.elapsed().as_secs(),
        api_bind: state.config.api.bind.clone(),
        api_port: state.config.api.port,
        database_path: state.config.api.database_path.clone(),
    }))
}

async fn restart(State(state): State<ApiState>) -> ApiResult<StatusCode> {
    state.events.send(AppEvent::ServiceRestartScheduled);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if let Err(error) = tokio::task::spawn_blocking(restart_service).await {
            log::warn!("service restart task failed: {}", error);
        }
    });
    Ok(StatusCode::ACCEPTED)
}

fn restart_service() {
    let initd = "/etc/init.d/sms-relayed";
    let result = if std::path::Path::new(initd).exists() {
        Command::new(initd).arg("restart").status()
    } else {
        Command::new("systemctl")
            .args(["restart", "sms-relayed"])
            .status()
    };
    match result {
        Ok(status) if status.success() => {
            log::info!("service restart scheduled");
        }
        Ok(status) => {
            log::warn!("service restart command exited with status {}", status);
        }
        Err(e) => {
            log::warn!("failed to run service restart command: {}", e);
        }
    }
}
