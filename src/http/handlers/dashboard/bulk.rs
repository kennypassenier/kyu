//! Bulk requeue/delete and message replay, as the pages' scripts call them.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;

use crate::engine::EngineError;

use crate::http::AppState;
use crate::http::error::ApiError;
use crate::http::handlers::engine_call::spawn_engine_write;

/// Most ids one bulk request may carry. Each id is its own small
/// transaction, so the cap keeps one request from holding the writer.
const BULK_MAX_IDS: usize = 1_000;

#[derive(Debug, Deserialize)]
struct BulkBody {
    subscription: String,
    ids: Vec<String>,
}

fn parse_bulk(body: &[u8]) -> Result<BulkBody, ApiError> {
    let parsed: BulkBody = serde_json::from_slice(body).map_err(|error| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("the bulk request body is not usable: {error}"),
            "send JSON like {\"subscription\": \"name\", \"ids\": [\"<message id>\"]}.",
        )
    })?;
    if parsed.ids.len() > BULK_MAX_IDS {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            format!(
                "a bulk request carries at most {BULK_MAX_IDS} messages, got {}",
                parsed.ids.len()
            ),
            "split the selection into several requests.",
        ));
    }
    Ok(parsed)
}

/// Runs `one` for each id and sorts the answers into `done` and `refused`.
/// A refusal is an id the subscription cannot do this to (not dead any
/// more, never delivered); anything else — an unknown topic or
/// subscription, a store fault — fails the whole request, because every id
/// would fail the same way.
fn bulk_each(
    ids: Vec<String>,
    mut one: impl FnMut(&str) -> Result<(), EngineError>,
) -> Result<serde_json::Value, EngineError> {
    let mut done = Vec::new();
    let mut refused = Vec::new();
    for id in ids {
        match one(&id) {
            Ok(()) => done.push(id),
            Err(EngineError::NoSuchDelivery { .. }) => {
                refused.push(json!({
                    "id": id,
                    "reason": "this subscription holds no such message: it was never \
                               delivered to it, or it was already deleted",
                }));
            }
            Err(error @ EngineError::NotDead { .. }) => {
                refused.push(json!({ "id": id, "reason": error.to_string() }));
            }
            Err(other) => return Err(other),
        }
    }
    Ok(json!({ "done": done, "refused": refused }))
}

/// `POST /t/{topic}/dashboard/bulk/requeue` — the dead-letter selection
/// bar's Requeue.
pub async fn dashboard_bulk_requeue(
    State(state): State<AppState>,
    Path(topic): Path<String>,
    body: axum::body::Bytes,
) -> Result<Response, ApiError> {
    let BulkBody { subscription, ids } = parse_bulk(&body)?;
    let engine = state.engine.clone();
    let (t, s) = (topic.clone(), subscription.clone());
    let answer =
        spawn_engine_write(move || bulk_each(ids, |id| engine.requeue_dead(&t, &s, id))).await?;
    state
        .notifiers
        .wake(&topic, std::slice::from_ref(&subscription));
    Ok(axum::Json(answer).into_response())
}

/// `POST /t/{topic}/dashboard/bulk/delete` — Delete on the dead-letter
/// selection bar and on the subscription backlog.
pub async fn dashboard_bulk_delete(
    State(state): State<AppState>,
    Path(topic): Path<String>,
    body: axum::body::Bytes,
) -> Result<Response, ApiError> {
    let BulkBody { subscription, ids } = parse_bulk(&body)?;
    let engine = state.engine.clone();
    let answer = spawn_engine_write(move || {
        bulk_each(ids, |id| engine.delete_delivery(&topic, &subscription, id))
    })
    .await?;
    Ok(axum::Json(answer).into_response())
}

/// `POST /t/{topic}/dashboard/messages/{id}/replay` — publishes a stored
/// message again as a new one.
pub async fn dashboard_replay(
    State(state): State<AppState>,
    Path((topic, id)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let engine = state.engine.clone();
    let (t, i) = (topic.clone(), id.clone());
    let published = spawn_engine_write(move || engine.replay(&t, &i)).await?;
    state.notifiers.wake(&topic, &published.delivered_to);
    tracing::info!(topic = %topic, original = %id, id = %published.id, "message replayed from the dashboard");
    Ok(axum::Json(json!({ "id": published.id })).into_response())
}
