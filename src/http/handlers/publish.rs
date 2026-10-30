//! K1 · publishing a message.

use axum::body::{Body, to_bytes};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;

use crate::http::AppState;
use crate::http::error::ApiError;
use crate::http::handlers::engine_call::spawn_engine_write;

#[derive(Debug, Deserialize)]
pub struct PublishQuery {
    /// W4 · deliver this many milliseconds from now.
    pub delay: Option<i64>,
    /// W4 · deliver at this Unix millisecond timestamp.
    pub at: Option<i64>,
}

/// K1 · `POST /t/{topic}`
pub async fn publish(
    State(state): State<AppState>,
    Path(topic): Path<String>,
    Query(query): Query<PublishQuery>,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, ApiError> {
    let now = state.engine.now_ms();
    let due_at = match (query.delay, query.at) {
        (None, None) => None,
        (Some(_), Some(_)) => {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "delay and at cannot both be given",
                "use delay=<milliseconds from now> or at=<unix milliseconds>, not both. \
                 Two answers to when would mean one of them is quietly ignored.",
            ));
        }
        (Some(delay), None) => {
            if delay < 0 {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    format!("delay must not be negative, got {delay}"),
                    "use a positive number of milliseconds, or leave delay out to \
                     deliver immediately.",
                ));
            }
            Some(now.saturating_add(delay))
        }
        (None, Some(at)) => Some(at),
    };

    let dedup = crate::http::dedup::from_headers(&headers)?;

    let limit = state.limits.max_body_bytes;
    let payload = to_bytes(body, limit).await.map_err(|error| {
        // to_bytes fails both for an oversized body and for a broken
        // connection; only the size case has a useful remedy about limits.
        if error.to_string().contains("length limit") {
            ApiError::payload_too_large(limit)
        } else {
            ApiError::unreadable_body(&error.to_string())
        }
    })?;

    // Stored verbatim, including a content type the client got wrong:
    // `curl -d` sends form-urlencoded unless told otherwise (AR2). kyu
    // reports what it was given rather than guessing.
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);

    let engine = state.engine.clone();
    let topic_for_engine = topic.clone();
    let published = spawn_engine_write(move || {
        engine.publish_damped(
            &topic_for_engine,
            &payload,
            content_type.as_deref(),
            due_at,
            dedup.as_ref(),
        )
    })
    .await?;

    if published.duplicate {
        tracing::info!(topic = %topic, id = %published.id, "duplicate publish suppressed");
        return Ok(crate::http::dedup::duplicate_response(&published.id));
    }

    state.notifiers.wake(&topic, &published.delivered_to);

    tracing::info!(
        topic = %topic,
        id = %published.id,
        delivered_to = published.delivered_to.len(),
        due_at = ?published.due_at,
        "message published"
    );

    let mut body = json!({ "id": published.id });
    if let Some(due) = published.due_at {
        body.as_object_mut()
            .expect("a JSON object")
            .insert("due_at".to_string(), json!(due));
    }

    Ok((StatusCode::CREATED, axum::Json(body)).into_response())
}
