use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use time::OffsetDateTime;

use crate::modem::PublicModemHealth;

use super::ApiState;

#[derive(Serialize)]
struct ServiceHealth {
    status: &'static str,
    #[serde(with = "time::serde::rfc3339")]
    checked_at: OffsetDateTime,
}

#[derive(Serialize)]
struct HealthResponse {
    service: ServiceHealth,
    modem: PublicModemHealth,
}

pub fn routes() -> Router<ApiState> {
    Router::new().route("/api/health", get(health))
}

async fn health(State(state): State<ApiState>) -> Json<HealthResponse> {
    let checked_at = OffsetDateTime::now_utc();
    let service = ServiceHealth {
        status: if state.store.health_check().is_ok() {
            "ok"
        } else {
            "error"
        },
        checked_at,
    };
    let modem = state
        .modem
        .public_health(&state.config.app.modem_path)
        .await;
    Json(HealthResponse { service, modem })
}
