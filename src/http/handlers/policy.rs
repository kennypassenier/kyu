//! K7 · reading and changing a subscription's delivery policy.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;

use crate::engine::policy::Policy;
use crate::store::queries::StoredPolicy;

use crate::http::AppState;
use crate::http::error::ApiError;
use crate::http::handlers::engine_call::{spawn_engine, spawn_engine_write};

/// K7 · `GET /api/t/{topic}/subs/{sub}/policy`
pub async fn get_policy(
    State(state): State<AppState>,
    Path((topic, subscription)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let engine = state.engine.clone();
    let defaults = engine.defaults();
    let (effective, stored) = spawn_engine(move || engine.policy(&topic, &subscription)).await?;
    let idle = (
        stored.idle_flag_ms.unwrap_or(defaults.idle_flag_ms),
        stored.idle_archive_ms.unwrap_or(defaults.idle_archive_ms),
    );

    Ok((
        StatusCode::OK,
        axum::Json(policy_json(effective, stored, idle)),
    )
        .into_response())
}

#[derive(Debug, Deserialize)]
pub struct PolicyBody {
    pub lease_ms: Option<i64>,
    pub max_attempts: Option<i64>,
    pub backoff_ms: Option<i64>,
    pub ttl_ms: Option<i64>,
    /// K11 · this subscription's own idle thresholds, for a consumer whose
    /// normal rhythm is slower than the hub's default.
    pub idle_flag_ms: Option<i64>,
    pub idle_archive_ms: Option<i64>,
}

/// Parsed by hand rather than with the Json extractor, so a malformed body
/// still answers with a remedy (standing rule 11).
fn parse_policy(body: &[u8]) -> Result<StoredPolicy, ApiError> {
    let parsed: PolicyBody = serde_json::from_slice(body).map_err(|error| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("the policy body is not valid JSON: {error}"),
            "send an object with any of lease_ms, max_attempts, backoff_ms and ttl_ms, \
             for example {\"ttl_ms\": 600000}. Fields you leave out return to their \
             defaults, and the response tells you what is in force.",
        )
    })?;
    Ok(StoredPolicy {
        lease_ms: parsed.lease_ms,
        max_attempts: parsed.max_attempts,
        backoff_ms: parsed.backoff_ms,
        ttl_ms: parsed.ttl_ms,
        idle_flag_ms: parsed.idle_flag_ms,
        idle_archive_ms: parsed.idle_archive_ms,
    })
}

/// K7 · `PUT /api/t/{topic}/subs/{sub}/policy`
///
/// The body replaces the policy in full: a field left out goes back to its
/// default. One rule beats two, and the response says what is now in force.
pub async fn put_policy(
    State(state): State<AppState>,
    Path((topic, subscription)): Path<(String, String)>,
    body: axum::body::Bytes,
) -> Result<Response, ApiError> {
    let stored = parse_policy(&body)?;

    let engine = state.engine.clone();
    let defaults = engine.defaults();
    let topic_for_log = topic.clone();
    let subscription_for_log = subscription.clone();
    let effective =
        spawn_engine_write(move || engine.set_policy(&topic, &subscription, stored)).await?;
    let idle = (
        stored.idle_flag_ms.unwrap_or(defaults.idle_flag_ms),
        stored.idle_archive_ms.unwrap_or(defaults.idle_archive_ms),
    );

    tracing::info!(
        topic = %topic_for_log,
        subscription = %subscription_for_log,
        lease_ms = effective.lease_ms,
        max_attempts = effective.max_attempts,
        backoff_ms = effective.backoff_ms,
        ttl_ms = ?effective.ttl_ms,
        "subscription policy set"
    );

    Ok((
        StatusCode::OK,
        axum::Json(policy_json(effective, stored, idle)),
    )
        .into_response())
}

fn policy_json(effective: Policy, stored: StoredPolicy, idle: (i64, i64)) -> serde_json::Value {
    // Reporting both means the dashboard can show "30000 (default)" instead
    // of leaving you to guess where a number came from.
    json!({
        "effective": {
            "lease_ms": effective.lease_ms,
            "max_attempts": effective.max_attempts,
            "backoff_ms": effective.backoff_ms,
            "ttl_ms": effective.ttl_ms,
            "idle_flag_ms": idle.0,
            "idle_archive_ms": idle.1,
        },
        "explicit": {
            "lease_ms": stored.lease_ms,
            "max_attempts": stored.max_attempts,
            "backoff_ms": stored.backoff_ms,
            "ttl_ms": stored.ttl_ms,
            "idle_flag_ms": stored.idle_flag_ms,
            "idle_archive_ms": stored.idle_archive_ms,
        },
        "retry_schedule_ms": (1..effective.max_attempts.max(1))
            .map(|attempt| effective.retry_delay_ms(attempt))
            .collect::<Vec<_>>(),
    })
}
