use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
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

pub trait ServiceRestarter: Send + Sync {
    fn restart(&self);
}

#[derive(Clone)]
pub struct ServiceControl {
    restarter: Arc<dyn ServiceRestarter>,
    restart_pending: Arc<AtomicBool>,
}

impl ServiceControl {
    pub fn new(restarter: impl ServiceRestarter + 'static) -> Self {
        Self {
            restarter: Arc::new(restarter),
            restart_pending: Arc::new(AtomicBool::new(false)),
        }
    }

    fn schedule(&self) -> bool {
        if self
            .restart_pending
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return false;
        }

        let restarter = self.restarter.clone();
        let restart_pending = self.restart_pending.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            if let Err(error) = tokio::task::spawn_blocking(move || restarter.restart()).await {
                log::warn!("service restart task failed: {}", error);
            }
            restart_pending.store(false, Ordering::Release);
        });
        true
    }
}

impl Default for ServiceControl {
    fn default() -> Self {
        Self::new(SystemServiceRestarter)
    }
}

struct SystemServiceRestarter;

impl ServiceRestarter for SystemServiceRestarter {
    fn restart(&self) {
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
                log::info!("service restart command completed successfully");
            }
            Ok(status) => {
                log::warn!("service restart command exited with status {}", status);
            }
            Err(error) => {
                log::warn!("failed to run service restart command: {}", error);
            }
        }
    }
}

pub fn routes() -> Router<ApiState> {
    Router::new()
        .route("/api/status", get(status))
        .route("/api/service/restart", post(restart))
}

pub fn schedule_restart(state: &ApiState) -> bool {
    let scheduled = state.service_control.schedule();
    if scheduled {
        state.events.send(AppEvent::ServiceRestartScheduled);
    }
    scheduled
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
    schedule_restart(&state);
    Ok(StatusCode::ACCEPTED)
}
