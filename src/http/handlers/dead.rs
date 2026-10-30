//! K6, K8, K11 · dead letters and individual deliveries: list, requeue,
//! delete, and unarchive a subscription.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::Deserialize;
use serde_json::json;

use crate::http::AppState;
use crate::http::error::ApiError;
use crate::http::handlers::engine_call::{spawn_engine, spawn_engine_write};

#[derive(Debug, Deserialize)]
pub struct DeadQuery {
    pub limit: Option<usize>,
}

/// How many dead letters a listing returns by default, and at most.
const DEFAULT_DEAD_LIMIT: usize = 100;
const MAX_DEAD_LIMIT: usize = 1000;

/// How much of a payload the dead-letter list shows. Enough to recognise a
/// message, bounded so listing a queue of megabyte payloads stays cheap; the
/// full byte count travels with it, and truncation is always marked (AR11).
const DEAD_PAYLOAD_PREVIEW: usize = 4096;

/// K6 · `GET /api/t/{topic}/subs/{sub}/dead`
pub async fn list_dead(
    State(state): State<AppState>,
    Path((topic, subscription)): Path<(String, String)>,
    Query(query): Query<DeadQuery>,
) -> Result<Response, ApiError> {
    let limit = query
        .limit
        .unwrap_or(DEFAULT_DEAD_LIMIT)
        .min(MAX_DEAD_LIMIT);
    let engine = state.engine.clone();
    let letters = spawn_engine(move || engine.dead_letters(&topic, &subscription, limit)).await?;

    let items: Vec<_> = letters
        .into_iter()
        .map(|letter| {
            let total = letter.payload.len();
            let shown = total.min(DEAD_PAYLOAD_PREVIEW);
            let mut item = json!({
                "id": letter.id,
                "published_at": letter.published_at,
                "dead_at": letter.dead_at,
                "attempts": letter.attempts,
                "content_type": letter.content_type,
                "payload_bytes": total,
                "truncated": total > shown,
            });
            let map = item.as_object_mut().expect("a JSON object");
            match std::str::from_utf8(&letter.payload[..shown]) {
                Ok(text) => {
                    map.insert("payload_text".to_string(), json!(text));
                }
                Err(_) => {
                    map.insert(
                        "payload_base64".to_string(),
                        json!(BASE64.encode(&letter.payload[..shown])),
                    );
                }
            }
            item
        })
        .collect();

    Ok((StatusCode::OK, axum::Json(json!({ "dead_letters": items }))).into_response())
}

#[derive(Debug, Deserialize)]
pub struct RequeueQuery {
    #[serde(rename = "as")]
    pub as_: Option<String>,
}

/// K6 · `POST /api/t/{topic}/subs/{sub}/dead/{id}/requeue`
pub async fn requeue_dead(
    State(state): State<AppState>,
    Path((topic, subscription, id)): Path<(String, String, String)>,
) -> Result<Response, ApiError> {
    let engine = state.engine.clone();
    let topic_for_wake = topic.clone();
    let subscription_for_wake = subscription.clone();
    let id_for_response = id.clone();
    spawn_engine_write(move || engine.requeue_dead(&topic, &subscription, &id)).await?;

    state.notifiers.wake(
        &topic_for_wake,
        std::slice::from_ref(&subscription_for_wake),
    );

    tracing::info!(
        topic = %topic_for_wake,
        subscription = %subscription_for_wake,
        id = %id_for_response,
        "dead letter requeued"
    );

    Ok((
        StatusCode::OK,
        axum::Json(json!({ "requeued": id_for_response })),
    )
        .into_response())
}

/// [W15, W16] `POST /api/t/{topic}/subs/{sub}/deliveries/{id}/delete` —
/// removes one delivery for good, whatever state it is in. No wake:
/// deleting creates no new pending work for the subscription's poller.
pub async fn delete_delivery(
    State(state): State<AppState>,
    Path((topic, subscription, id)): Path<(String, String, String)>,
) -> Result<Response, ApiError> {
    let engine = state.engine.clone();
    let topic_for_log = topic.clone();
    let subscription_for_log = subscription.clone();
    let id_for_response = id.clone();
    spawn_engine_write(move || engine.delete_delivery(&topic, &subscription, &id)).await?;

    tracing::info!(
        topic = %topic_for_log,
        subscription = %subscription_for_log,
        id = %id_for_response,
        "dead letter deleted"
    );

    Ok((
        StatusCode::OK,
        axum::Json(json!({ "deleted": id_for_response })),
    )
        .into_response())
}

/// K11 · `POST /api/t/{topic}/subs/{sub}/unarchive`
pub async fn unarchive(
    State(state): State<AppState>,
    Path((topic, subscription)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let engine = state.engine.clone();
    let topic_for_response = topic.clone();
    let subscription_for_response = subscription.clone();
    let changed = spawn_engine_write(move || engine.unarchive(&topic, &subscription)).await?;

    tracing::info!(
        topic = %topic_for_response,
        subscription = %subscription_for_response,
        "subscription unarchived"
    );

    let note = if changed {
        "this subscription receives messages published from now on; the backlog it \
         held when it was archived was settled as lapsed. Poll with ?from=beginning \
         to pick up what the topic still retains."
    } else {
        "this subscription was not archived, so nothing changed and no event was \
         published."
    };

    Ok((
        StatusCode::OK,
        axum::Json(json!({
            "unarchived": subscription_for_response,
            "topic": topic_for_response,
            "changed": changed,
            "note": note,
        })),
    )
        .into_response())
}
