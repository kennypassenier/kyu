//! The three verbs (K1, K2, K3) over the contract frozen in AR2.
//!
//! Handlers translate HTTP into engine calls and back. No delivery logic
//! lives here — only the two things that are genuinely HTTP's business:
//! how a message is shaped on the wire, and how a long poll waits.

use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::Deserialize;
use serde_json::json;
use tokio::time::Instant;

use crate::engine::{Claimed, EngineError, NewSubscription, Received, names};

use super::AppState;
use super::error::ApiError;

/// Headers carrying the metadata a raw-body response cannot put in the
/// body (AR2). Lowercase on the wire either way; HTTP/2 and proxies
/// normalise the case, so clients must match case-insensitively.
const HEADER_ID: &str = "mailbox-id";
const HEADER_TOPIC: &str = "mailbox-topic";
const HEADER_ATTEMPT: &str = "mailbox-attempt";
const HEADER_PUBLISHED_AT: &str = "mailbox-published-at";
/// Says out loud what would otherwise be an unexplained empty answer.
const HEADER_NOTICE: &str = "mailbox-notice";

/// A fresh subscription's first poll cannot see anything published before
/// it existed (G7). Saying so turns a confusing 204 into a lesson about how
/// the hub works — and points at the way to get that history instead.
fn notice(new: &NewSubscription) -> String {
    if new.retained_before == 0 {
        format!(
            "subscription {:?} was created by this poll and receives messages published from now on",
            new.name
        )
    } else {
        format!(
            "subscription {:?} was created by this poll and receives messages published from now on; \
             {} earlier message(s) on this topic predate it and were not delivered to it",
            new.name, new.retained_before
        )
    }
}

pub async fn healthz() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"status":"ok"}"#,
    )
}

/// K1 · `POST /t/{topic}`
pub async fn publish(
    State(state): State<AppState>,
    Path(topic): Path<String>,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, ApiError> {
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
    // `curl -d` sends form-urlencoded unless told otherwise (AR2). mailbox
    // reports what it was given rather than guessing.
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);

    let engine = state.engine.clone();
    let topic_for_engine = topic.clone();
    let published =
        spawn_engine(move || engine.publish(&topic_for_engine, &payload, content_type.as_deref()))
            .await?;

    state.notifiers.wake(&topic, &published.delivered_to);

    tracing::info!(
        topic = %topic,
        id = %published.id,
        delivered_to = published.delivered_to.len(),
        "message published"
    );

    Ok((
        StatusCode::CREATED,
        axum::Json(json!({ "id": published.id })),
    )
        .into_response())
}

#[derive(Debug, Deserialize)]
pub struct ReceiveQuery {
    /// The subscription name. `as` is a Rust keyword, hence the rename.
    ///
    /// Optional here so that leaving it out produces mailbox's own error
    /// with a remedy, rather than the framework's bare rejection text
    /// (standing rule 11).
    #[serde(rename = "as")]
    pub as_: Option<String>,
    pub wait: Option<u64>,
    pub envelope: Option<String>,
    pub from: Option<String>,
}

/// K2 · `GET /t/{topic}/next?as={subscription}`
pub async fn receive(
    State(state): State<AppState>,
    Path(topic): Path<String>,
    Query(query): Query<ReceiveQuery>,
) -> Result<Response, ApiError> {
    let subscription = query
        .as_
        .clone()
        .ok_or_else(ApiError::missing_subscription)?;

    // Validated here, before anything is allocated for this subscription:
    // the notifier map is keyed by name, and a typo should not be able to
    // leave an entry behind in it. The engine validates again for its own
    // callers, and both paths report the same error.
    for (kind, name) in [("topic", &topic), ("subscription", &subscription)] {
        if !names::is_valid(name) {
            return Err(EngineError::InvalidName {
                kind,
                name: name.clone(),
            }
            .into());
        }
    }

    let wait = match query.wait {
        None => Duration::from_secs(state.limits.default_wait_s),
        Some(seconds) if seconds <= state.limits.max_wait_s => Duration::from_secs(seconds),
        Some(_) => return Err(ApiError::invalid_wait(state.limits.max_wait_s)),
    };
    let envelope = matches!(query.envelope.as_deref(), Some("json"));
    let from_beginning = matches!(query.from.as_deref(), Some("beginning"));

    let deadline = Instant::now() + wait;
    let notify = state.notifiers.for_subscription(&topic, &subscription);
    // Only the first pass can create the subscription, so remember what it
    // said and report it however the poll ends.
    let mut created = None;

    loop {
        // Register for a wakeup *before* looking, so a publish landing
        // between the look and the wait cannot be missed.
        let waiter = notify.notified();
        let mut waiter = std::pin::pin!(waiter);
        waiter.as_mut().enable();

        let received = claim(&state, &topic, &subscription, from_beginning).await?;
        if received.created.is_some() {
            created = received.created;
        }

        if let Some(claimed) = received.claimed {
            return Ok(render(claimed, envelope, created.as_ref()));
        }

        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            // Nothing waiting. 204 rather than an error: an empty topic is
            // the normal state of a healthy queue.
            let mut response = StatusCode::NO_CONTENT.into_response();
            if let Some(new) = &created {
                insert_str(response.headers_mut(), HEADER_NOTICE, &notice(new));
            }
            return Ok(response);
        }

        let recheck = remaining.min(state.limits.recheck_interval);
        tokio::select! {
            _ = waiter => {}
            _ = tokio::time::sleep(recheck) => {}
        }
    }
}

async fn claim(
    state: &AppState,
    topic: &str,
    subscription: &str,
    from_beginning: bool,
) -> Result<Received, ApiError> {
    let engine = state.engine.clone();
    let topic = topic.to_string();
    let subscription = subscription.to_string();
    spawn_engine(move || engine.claim_next(&topic, &subscription, from_beginning)).await
}

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

    spawn_engine(move || engine.ack(&topic_for_engine, &subscription, &id_for_engine)).await?;

    tracing::info!(topic = %topic, subscription = %query.as_, id = %id, "message acknowledged");

    Ok((StatusCode::OK, axum::Json(json!({ "acked": id }))).into_response())
}

/// Runs a blocking engine call off the async runtime (AR5): rusqlite is
/// synchronous, and holding the writer lock on a runtime thread would stall
/// every other request.
async fn spawn_engine<T, F>(work: F) -> Result<T, ApiError>
where
    F: FnOnce() -> Result<T, crate::engine::EngineError> + Send + 'static,
    T: Send + 'static,
{
    match tokio::task::spawn_blocking(work).await {
        Ok(result) => result.map_err(ApiError::from),
        Err(join_error) => Err(ApiError::from(crate::engine::EngineError::Internal(
            anyhow::anyhow!("a store task failed to run: {join_error}"),
        ))),
    }
}

/// AR2's two response shapes. Raw is the default so payload bytes stay
/// exactly what was published; the envelope exists because parsing headers
/// in a shell is friction the re-entry test cannot afford.
fn render(claimed: Claimed, envelope: bool, created: Option<&NewSubscription>) -> Response {
    let attempt = claimed.attempt();
    let message = claimed.message;

    if !envelope {
        let content_type = message
            .content_type
            .clone()
            .unwrap_or_else(|| "application/octet-stream".to_string());

        let mut response = (StatusCode::OK, message.payload).into_response();
        let headers = response.headers_mut();
        if let Ok(value) = content_type.parse() {
            headers.insert(header::CONTENT_TYPE, value);
        }
        insert_str(headers, HEADER_ID, &message.id);
        insert_str(headers, HEADER_TOPIC, &claimed.topic);
        insert_str(headers, HEADER_ATTEMPT, &attempt.to_string());
        insert_str(
            headers,
            HEADER_PUBLISHED_AT,
            &message.published_at.to_string(),
        );
        return response;
    }

    // Exactly one payload key is present, and which one says how the bytes
    // were understood — never a silent transformation (G8):
    //   payload        the bytes parsed as JSON
    //   payload_text   valid UTF-8 that is not JSON
    //   payload_base64 anything else
    let mut body = json!({
        "id": message.id,
        "topic": claimed.topic,
        "attempt": attempt,
        "published_at": message.published_at,
        "content_type": message.content_type,
    });
    let map = body.as_object_mut().expect("a JSON object");

    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&message.payload) {
        map.insert("payload".to_string(), value);
    } else if let Ok(text) = String::from_utf8(message.payload.clone()) {
        map.insert("payload_text".to_string(), json!(text));
    } else {
        map.insert(
            "payload_base64".to_string(),
            json!(BASE64.encode(&message.payload)),
        );
    }

    let mut response = (StatusCode::OK, axum::Json(body)).into_response();
    if let Some(new) = created {
        insert_str(response.headers_mut(), HEADER_NOTICE, &notice(new));
    }
    response
}

fn insert_str(headers: &mut HeaderMap, name: &'static str, value: &str) {
    if let Ok(parsed) = value.parse() {
        headers.insert(name, parsed);
    }
}
