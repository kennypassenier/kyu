//! K3 and K6 · acknowledging and negatively acknowledging a delivery.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;

use crate::engine::Settled;

use crate::http::AppState;
use crate::http::error::ApiError;
use crate::http::handlers::engine_call::spawn_engine_write;

#[derive(Debug, Deserialize)]
pub struct AckQuery {
    #[serde(rename = "as")]
    pub as_: String,
}

/// K3 · `POST /t/{topic}/ack/{id}?as={subscription}`
pub async fn ack(
    State(state): State<AppState>,
    Path((topic, id)): Path<(String, String)>,
    Query(query): Query<AckQuery>,
) -> Result<Response, ApiError> {
    let engine = state.engine.clone();
    let topic_for_engine = topic.clone();
    let subscription = query.as_.clone();
    let id_for_engine = id.clone();

    spawn_engine_write(move || engine.ack(&topic_for_engine, &subscription, &id_for_engine))
        .await?;

    tracing::info!(topic = %topic, subscription = %query.as_, id = %id, "message acknowledged");

    Ok((StatusCode::OK, axum::Json(json!({ "acked": id }))).into_response())
}

#[derive(Debug, Deserialize)]
pub struct NackQuery {
    #[serde(rename = "as")]
    pub as_: Option<String>,
    /// `true` sends the message straight to the dead-letter list instead of
    /// spending its remaining attempts on a payload that cannot work.
    pub dead: Option<bool>,
}

/// The word the nack answer uses for what happened to the message.
fn outcome_name(settled: &Settled) -> &'static str {
    match settled {
        Settled::Redelivered => "redelivered",
        Settled::DeadLettered => "dead_lettered",
        Settled::Expired => "expired",
        Settled::Unchanged => "unchanged",
    }
}

/// W5 · `POST /t/{topic}/nack/{id}?as={subscription}[&dead=true]`
pub async fn nack(
    State(state): State<AppState>,
    Path((topic, id)): Path<(String, String)>,
    Query(query): Query<NackQuery>,
) -> Result<Response, ApiError> {
    let subscription = query
        .as_
        .clone()
        .ok_or_else(ApiError::missing_subscription)?;
    let dead = query.dead.unwrap_or(false);

    let engine = state.engine.clone();
    let topic_for_engine = topic.clone();
    let subscription_for_engine = subscription.clone();
    let id_for_engine = id.clone();
    let settled = spawn_engine_write(move || {
        engine.nack(
            &topic_for_engine,
            &subscription_for_engine,
            &id_for_engine,
            dead,
        )
    })
    .await?;

    let outcome = outcome_name(&settled);

    // A redelivered message may be waiting behind a backoff, so waking the
    // subscription is a hint, not a promise that it is available now.
    if matches!(settled, Settled::Redelivered) {
        state
            .notifiers
            .wake(&topic, std::slice::from_ref(&subscription));
    }

    tracing::info!(topic = %topic, subscription = %subscription, id = %id, outcome, "message nacked");

    Ok((
        StatusCode::OK,
        axum::Json(json!({ "nacked": id, "outcome": outcome })),
    )
        .into_response())
}
