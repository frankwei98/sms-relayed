use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use time::OffsetDateTime;

use crate::storage::ForwardAttemptOutcome;

use super::{ApiResult, ApiState};

#[derive(Serialize)]
struct ForwardingResponse {
    generated_at: String,
    profiles: Vec<ProfileStatus>,
}

#[derive(Serialize)]
struct ProfileStatus {
    profile_key: String,
    enabled: bool,
    samples: Vec<SampleView>,
}

#[derive(Serialize)]
struct SampleView {
    attempt_number: i32,
    is_retry: bool,
    started_at: String,
    completed_at: String,
    latency_ms: i64,
    dispatch_delay_ms: Option<i64>,
    outcome: ForwardAttemptOutcome,
    error_code: Option<String>,
}

pub fn routes() -> Router<ApiState> {
    Router::new().route("/api/forwarding/attempts", get(forwarding_attempts))
}

async fn forwarding_attempts(State(state): State<ApiState>) -> ApiResult<Json<ForwardingResponse>> {
    let config_profiles = state.config.enabled_profiles()?;

    let mut profiles: Vec<ProfileStatus> = Vec::new();
    let mut seen_keys = std::collections::HashSet::new();

    for profile in &config_profiles {
        let key = profile.key();
        seen_keys.insert(key.clone());
        let samples = load_samples(&state, &key)?;
        profiles.push(ProfileStatus {
            profile_key: key,
            enabled: true,
            samples,
        });
    }

    let all_keys = state.store.list_forward_attempt_profiles()?;
    for key in all_keys {
        if seen_keys.insert(key.clone()) {
            let samples = load_samples(&state, &key)?;
            profiles.push(ProfileStatus {
                profile_key: key,
                enabled: false,
                samples,
            });
        }
    }

    let generated_at = OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();

    Ok(Json(ForwardingResponse {
        generated_at,
        profiles,
    }))
}

fn load_samples(state: &ApiState, profile_key: &str) -> ApiResult<Vec<SampleView>> {
    let samples = state.store.list_forward_attempts(profile_key, 5)?;
    Ok(samples
        .into_iter()
        .map(|s| SampleView {
            attempt_number: s.attempt_number,
            is_retry: s.is_retry(),
            started_at: s.started_at,
            completed_at: s.completed_at,
            latency_ms: s.latency_ms,
            dispatch_delay_ms: s.dispatch_delay_ms,
            outcome: s.outcome,
            error_code: s.error_code,
        })
        .collect())
}
