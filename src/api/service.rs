use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
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
    restart_status: RestartStatus,
}

pub trait ServiceRestarter: Send + Sync {
    fn restart(&self) -> anyhow::Result<()>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[repr(u8)]
#[serde(rename_all = "snake_case")]
pub enum RestartStatus {
    Idle,
    Scheduled,
    CommandCompleted,
    CommandFailed,
}

#[derive(Clone)]
pub struct ServiceControl {
    restarter: Arc<dyn ServiceRestarter>,
    restart_pending: Arc<AtomicBool>,
    restart_status: Arc<AtomicU8>,
}

impl ServiceControl {
    pub fn new(restarter: impl ServiceRestarter + 'static) -> Self {
        Self {
            restarter: Arc::new(restarter),
            restart_pending: Arc::new(AtomicBool::new(false)),
            restart_status: Arc::new(AtomicU8::new(RestartStatus::Idle as u8)),
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
        let restart_status = self.restart_status.clone();
        restart_status.store(RestartStatus::Scheduled as u8, Ordering::Release);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let status = match tokio::task::spawn_blocking(move || restarter.restart()).await {
                Ok(Ok(())) => RestartStatus::CommandCompleted,
                Ok(Err(error)) => {
                    log::warn!("service restart command failed: {}", error);
                    RestartStatus::CommandFailed
                }
                Err(error) => {
                    log::warn!("service restart task failed: {}", error);
                    RestartStatus::CommandFailed
                }
            };
            restart_status.store(status as u8, Ordering::Release);
            restart_pending.store(false, Ordering::Release);
        });
        true
    }

    pub fn restart_status(&self) -> RestartStatus {
        match self.restart_status.load(Ordering::Acquire) {
            value if value == RestartStatus::Scheduled as u8 => RestartStatus::Scheduled,
            value if value == RestartStatus::CommandCompleted as u8 => {
                RestartStatus::CommandCompleted
            }
            value if value == RestartStatus::CommandFailed as u8 => RestartStatus::CommandFailed,
            _ => RestartStatus::Idle,
        }
    }

    #[cfg(test)]
    pub(crate) fn restart_pending(&self) -> bool {
        self.restart_pending.load(Ordering::Acquire)
    }
}

impl Default for ServiceControl {
    fn default() -> Self {
        Self::new(SystemServiceRestarter)
    }
}

struct SystemServiceRestarter;

impl ServiceRestarter for SystemServiceRestarter {
    fn restart(&self) -> anyhow::Result<()> {
        let initd = "/etc/init.d/sms-relayed";
        let status = if std::path::Path::new(initd).exists() {
            Command::new(initd).arg("restart").status()
        } else {
            Command::new("systemctl")
                .args(["restart", "sms-relayed"])
                .status()
        }?;
        if !status.success() {
            anyhow::bail!("service restart command exited with status {status}");
        }
        log::info!("service restart command completed successfully");
        Ok(())
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
        restart_status: state.service_control.restart_status(),
    }))
}

async fn restart(State(state): State<ApiState>) -> ApiResult<StatusCode> {
    schedule_restart(&state);
    Ok(StatusCode::ACCEPTED)
}
