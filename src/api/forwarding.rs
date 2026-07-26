use std::collections::HashSet;

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use time::OffsetDateTime;

use crate::storage::ForwardAttemptOutcome;

use super::{ApiResult, ApiState};

const SAMPLE_LIMIT: u32 = 5;

#[derive(Serialize)]
struct ForwardingResponse {
    generated_at: String,
    sample_limit: u32,
    profiles: Vec<ProfileStatus>,
}

#[derive(Serialize)]
struct ProfileStatus {
    profile_key: String,
    configured: bool,
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
    let configured_keys = state.config.configured_profile_keys();
    let configured: HashSet<&str> = configured_keys.iter().map(String::as_str).collect();
    let enabled: HashSet<&str> = state
        .config
        .forward
        .enabled
        .iter()
        .map(String::as_str)
        .collect();

    let mut profiles = Vec::new();
    let mut seen_keys = HashSet::new();

    for key in &state.config.forward.enabled {
        if configured.contains(key.as_str()) && seen_keys.insert(key.clone()) {
            profiles.push(profile_status(&state, key.clone(), true, true).await?);
        }
    }

    for key in configured_keys {
        if seen_keys.insert(key.clone()) {
            profiles.push(
                profile_status(&state, key.clone(), true, enabled.contains(key.as_str())).await?,
            );
        }
    }

    let stored_keys = state
        .store
        .forwarding_profiles()
        .await
        .map_err(|error| super::ApiError::internal(error.to_string()))?;
    for key in stored_keys {
        if seen_keys.insert(key.clone()) {
            profiles.push(profile_status(&state, key, false, false).await?);
        }
    }

    let generated_at = OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();

    Ok(Json(ForwardingResponse {
        generated_at,
        sample_limit: SAMPLE_LIMIT,
        profiles,
    }))
}

async fn profile_status(
    state: &ApiState,
    profile_key: String,
    configured: bool,
    enabled: bool,
) -> ApiResult<ProfileStatus> {
    let samples = load_samples(state, &profile_key).await?;
    Ok(ProfileStatus {
        profile_key,
        configured,
        enabled,
        samples,
    })
}

async fn load_samples(state: &ApiState, profile_key: &str) -> ApiResult<Vec<SampleView>> {
    let profile_key = profile_key.to_string();
    let samples = state
        .store
        .forwarding_attempts(profile_key, SAMPLE_LIMIT)
        .await
        .map_err(|error| super::ApiError::internal(error.to_string()))?;
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
