//! The three verbs (K1, K2, K3) over the contract frozen in AR2.
//!
//! Handlers translate HTTP into engine calls and back. No delivery logic
//! lives here — only the two things that are genuinely HTTP's business:
//! how a message is shaped on the wire, and how a long poll waits.

use std::time::Duration;

use axum::Extension;
use axum::body::{Body, to_bytes};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use chassis::{Caller, Dashboard};
use serde::Deserialize;
use serde_json::json;
use tokio::time::Instant;

use crate::dashboard::{self, DeadLetterView, MessageView, SubscriptionView, TopicView};
use crate::engine::policy::Policy;
use crate::engine::{Claimed, EngineError, NewSubscription, Received, Settled, names};
use crate::store::queries::{self, StoredPolicy};

use super::AppState;
use super::error::ApiError;

/// Headers carrying the metadata a raw-body response cannot put in the
/// body (AR2). Lowercase on the wire either way; HTTP/2 and proxies
/// normalise the case, so clients must match case-insensitively.
const HEADER_ID: &str = "kyu-id";
const HEADER_TOPIC: &str = "kyu-topic";
const HEADER_ATTEMPT: &str = "kyu-attempt";
const HEADER_PUBLISHED_AT: &str = "kyu-published-at";
/// Says out loud what would otherwise be an unexplained empty answer.
const HEADER_NOTICE: &str = "kyu-notice";

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
        engine.publish_due(&topic_for_engine, &payload, content_type.as_deref(), due_at)
    })
    .await?;

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

#[derive(Debug, Deserialize)]
pub struct ReceiveQuery {
    /// The subscription name. `as` is a Rust keyword, hence the rename.
    ///
    /// Optional here so that leaving it out produces kyu's own error
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
    caller: Option<Extension<Caller>>,
    Path(topic): Path<String>,
    Query(query): Query<ReceiveQuery>,
) -> Result<Response, ApiError> {
    // Who polled goes into the attempt history the dashboard shows. The
    // login token is "admin"; an open hub (tests) has no caller at all.
    let app = caller.map(|Extension(caller)| match caller {
        Caller::Client { name, .. } => name,
        Caller::Admin => "admin".to_string(),
    });
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
    let mut backfilled = 0usize;

    loop {
        // Register for a wakeup *before* looking, so a publish landing
        // between the look and the wait cannot be missed.
        let waiter = notify.notified();
        let mut waiter = std::pin::pin!(waiter);
        waiter.as_mut().enable();

        let received = claim(&state, &topic, &subscription, from_beginning, app.clone()).await?;
        if received.created.is_some() {
            created = received.created;
        }
        if received.backfilled > 0 {
            backfilled += received.backfilled;
        }

        if let Some(claimed) = received.claimed {
            let mut response = render(claimed, envelope, created.as_ref());
            if backfilled > 0 {
                insert_str(
                    response.headers_mut(),
                    HEADER_NOTICE,
                    &format!(
                        "replayed {backfilled} retained message(s) into subscription {subscription:?}"
                    ),
                );
            }
            return Ok(response);
        }

        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            // Nothing waiting. 204 rather than an error: an empty topic is
            // the normal state of a healthy queue.
            let mut response = StatusCode::NO_CONTENT.into_response();
            if backfilled > 0 {
                insert_str(
                    response.headers_mut(),
                    HEADER_NOTICE,
                    &format!(
                        "replayed {backfilled} retained message(s) into subscription {subscription:?}"
                    ),
                );
            } else if let Some(new) = &created {
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
    app: Option<String>,
) -> Result<Received, ApiError> {
    let engine = state.engine.clone();
    let topic = topic.to_string();
    let subscription = subscription.to_string();
    spawn_engine_write(move || {
        engine.claim_next_as(&topic, &subscription, from_beginning, app.as_deref())
    })
    .await
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

    spawn_engine_write(move || engine.ack(&topic_for_engine, &subscription, &id_for_engine))
        .await?;

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

/// Like [`spawn_engine`], for calls that write the store (AR5, feat-backup-1
/// in chassis 3.1.0): holds a `chassis::shell::backup::writing_blocking()`
/// ticket for the call, so `<name> backup-pause` sees it as in flight and
/// waits for it, and a new one blocks until `backup-resume` instead of
/// racing the pause hook's WAL checkpoint.
async fn spawn_engine_write<T, F>(work: F) -> Result<T, ApiError>
where
    F: FnOnce() -> Result<T, crate::engine::EngineError> + Send + 'static,
    T: Send + 'static,
{
    spawn_engine(move || {
        let _ticket = chassis::shell::backup::writing_blocking();
        work()
    })
    .await
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
        // nosniff always: it costs nothing and stops a browser deciding that
        // a text/plain payload is really HTML.
        headers.insert(
            axum::http::header::X_CONTENT_TYPE_OPTIONS,
            axum::http::HeaderValue::from_static("nosniff"),
        );
        // attachment only for the types a browser would execute. A payload
        // published as text/html would otherwise run in the hub's own origin
        // for anyone opening this URL — an iframe on a hostile page is
        // enough. Everything else still opens normally in a tab, which is
        // what the AR2 mini-round settled on: protection without making the
        // API unbrowsable.
        if executes_in_a_browser(&content_type) {
            headers.insert(
                header::CONTENT_DISPOSITION,
                axum::http::HeaderValue::from_static("attachment"),
            );
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

/// Whether a browser would run this content type as code rather than show
/// it as data (AR2 amendment, 2026-08-28).
///
/// Deliberately a short, explicit list rather than a clever rule: the cost
/// of a wrong entry is a payload that renders itself in the hub's origin, so
/// the list errs towards including things. `nosniff` accompanies it, which
/// is what stops a browser executing something that is not on the list.
fn executes_in_a_browser(content_type: &str) -> bool {
    let media = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();

    matches!(
        media.as_str(),
        "text/html"
            | "application/xhtml+xml"
            | "image/svg+xml"
            | "text/xml"
            | "application/xml"
            | "text/xsl"
    ) || media.ends_with("+xml")
        || media.contains("javascript")
        || media.contains("ecmascript")
}

fn insert_str(headers: &mut HeaderMap, name: &'static str, value: &str) {
    if let Ok(parsed) = value.parse() {
        headers.insert(name, parsed);
    }
}

// ─── L4 · reliability endpoints (K6, K7, W5) ────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct NackQuery {
    #[serde(rename = "as")]
    pub as_: Option<String>,
    /// `true` sends the message straight to the dead-letter list instead of
    /// spending its remaining attempts on a payload that cannot work.
    pub dead: Option<bool>,
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

    let outcome = match settled {
        Settled::Redelivered => "redelivered",
        Settled::DeadLettered => "dead_lettered",
        Settled::Expired => "expired",
        Settled::Unchanged => "unchanged",
    };

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

/// K7 · `PUT /api/t/{topic}/subs/{sub}/policy`
///
/// The body replaces the policy in full: a field left out goes back to its
/// default. One rule beats two, and the response says what is now in force.
pub async fn put_policy(
    State(state): State<AppState>,
    Path((topic, subscription)): Path<(String, String)>,
    body: axum::body::Bytes,
) -> Result<Response, ApiError> {
    // Parsed by hand rather than with the Json extractor, so a malformed
    // body still answers with a remedy (standing rule 11).
    let parsed: PolicyBody = serde_json::from_slice(&body).map_err(|error| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("the policy body is not valid JSON: {error}"),
            "send an object with any of lease_ms, max_attempts, backoff_ms and ttl_ms, \
             for example {\"ttl_ms\": 600000}. Fields you leave out return to their \
             defaults, and the response tells you what is in force.",
        )
    })?;

    let stored = StoredPolicy {
        lease_ms: parsed.lease_ms,
        max_attempts: parsed.max_attempts,
        backoff_ms: parsed.backoff_ms,
        ttl_ms: parsed.ttl_ms,
        idle_flag_ms: parsed.idle_flag_ms,
        idle_archive_ms: parsed.idle_archive_ms,
    };

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

#[derive(Debug, Deserialize)]
pub struct DeadQuery {
    pub limit: Option<usize>,
}

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
    let limit = query.limit.unwrap_or(100).min(1000);
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

// ─── L6 · history and lifecycle (K8, K9, K11) ───────────────────────────────

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

#[derive(Debug, Deserialize)]
pub struct RetentionBody {
    pub retention_ms: Option<i64>,
    /// Explicit opt-out, so "keep forever" is something you write rather
    /// than something you get by leaving a field out.
    pub keep_forever: Option<bool>,
}

/// K9 · `GET|PUT /api/t/{topic}/retention`
pub async fn get_retention(
    State(state): State<AppState>,
    Path(topic): Path<String>,
) -> Result<Response, ApiError> {
    let engine = state.engine.clone();
    let (effective, explicit) = spawn_engine(move || engine.retention(&topic)).await?;
    Ok((
        StatusCode::OK,
        axum::Json(json!({ "effective_ms": effective, "explicit_ms": explicit })),
    )
        .into_response())
}

pub async fn put_retention(
    State(state): State<AppState>,
    Path(topic): Path<String>,
    body: axum::body::Bytes,
) -> Result<Response, ApiError> {
    let parsed: RetentionBody = serde_json::from_slice(&body).map_err(|error| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("the retention body is not valid JSON: {error}"),
            "send {\"retention_ms\": 604800000} for a week, {\"keep_forever\": true} to \
             keep messages indefinitely, or {} to fall back to the hub default.",
        )
    })?;

    let retention = if parsed.keep_forever.unwrap_or(false) {
        // Distinct from "unset": one means never collect, the other means
        // follow the hub default.
        Some(i64::MAX)
    } else {
        parsed.retention_ms
    };

    let engine = state.engine.clone();
    let topic_for_response = topic.clone();
    spawn_engine_write(move || engine.set_retention(&topic, retention)).await?;

    let engine = state.engine.clone();
    let (effective, explicit) = spawn_engine(move || engine.retention(&topic_for_response)).await?;

    Ok((
        StatusCode::OK,
        axum::Json(json!({ "effective_ms": effective, "explicit_ms": explicit })),
    )
        .into_response())
}

// ─── L7 · the dashboard (K10, W9, AR11) ─────────────────────────────────────

/// How many recent messages a topic page shows.
const RECENT_MESSAGES: usize = 20;

/// How many dead letters a topic page shows (K6). Bounded because a broken
/// consumer can produce a great many, and a page that will not load is no
/// use on the night you need it.
const DEAD_LETTERS_SHOWN: usize = 50;

/// The hub's own page templates, rendered inside the kit's layout (K2-2,
/// 2026-09-06): the kit brings nav, login, theme picker and CSP; these
/// fill `content` and add the two hub assets in `head`.
const TOPICS_HTML: &str = include_str!("../../templates/topics.html");
const TOPIC_HTML: &str = include_str!("../../templates/topic.html");
const SUBSCRIPTION_HTML: &str = include_str!("../../templates/subscription.html");
/// The cross-page shell (command palette, keys, help, tour, anchors, uses-map).
/// The kit renders a page from one source string, so an
/// `{% include "_kp.html" %}` line is expanded here, before rendering.
const KP_HTML: &str = include_str!("../../templates/_kp.html");
const KP_INCLUDE: &str = "{% include \"_kp.html\" %}";

fn page<C: serde::Serialize>(
    dash: &Dashboard,
    nav: &str,
    source: &str,
    ctx: C,
) -> Result<Html<String>, ApiError> {
    let expanded = source.replace(KP_INCLUDE, KP_HTML);
    dash.render_project(nav, &expanded, ctx)
        .map_err(|error| internal(format!("cannot render the page: {error}")))
}

/// A topic is "Backlog" on the Topics page from this many waiting messages.
const BACKLOG_STATUS_AT: i64 = 50;

/// A topic is "Quiet" on the Topics page after this long without a publish.
const QUIET_AFTER_MS: i64 = 24 * 3_600_000;

/// The longest dead-letter excerpt the Topics page keeps for its hover card.
const DEAD_NOTE_CHARS: usize = 80;

/// One row of the Topics page as the page's script reads it: the counts the
/// flow bar and the key figures are drawn from, the subscriptions behind the
/// "who reads this" card and the attention band, and the status word the
/// table sorts and filters by. Times are Unix milliseconds.
#[derive(serde::Serialize)]
struct TopicRowView {
    name: String,
    status: &'static str,
    messages: i64,
    subscriptions: Vec<String>,
    flagged: Vec<String>,
    backlog: i64,
    in_flight: i64,
    dead: i64,
    lapsed: i64,
    last_published_ms: Option<i64>,
    dead_note: Option<String>,
}

/// The status word of a topic: the worst thing true of it.
fn topic_status(dead: i64, backlog: i64, last_published_ms: Option<i64>, now: i64) -> &'static str {
    if dead > 0 {
        "Dead letters"
    } else if backlog >= BACKLOG_STATUS_AT {
        "Backlog"
    } else if last_published_ms.is_some_and(|at| now - at > QUIET_AFTER_MS) {
        "Quiet"
    } else {
        "Healthy"
    }
}

/// One topic as the shell lists it: a palette option and a node of the
/// "who reads what" map.
#[derive(serde::Serialize)]
struct ShellTopic {
    name: String,
    messages: i64,
    detail: String,
}

/// One subscription as the shell lists it: a palette option and a line of
/// the map. `tone` is the map's word for its state.
#[derive(serde::Serialize)]
struct ShellSubscription {
    topic: String,
    name: String,
    state: String,
    tone: &'static str,
    detail: String,
    /// Messages waiting for this subscription (the graph's optional node size).
    backlog: i64,
}

fn counted(n: i64, noun: &str) -> String {
    format!("{n} {noun}{}", if n == 1 { "" } else { "s" })
}

/// What every page's command palette and the Topics page's uses-map are
/// built from: every topic and every subscription, plus the hub's dead
/// letter total (what "Prune dead letters" would delete).
async fn shell_view(state: &AppState) -> Result<serde_json::Value, ApiError> {
    let engine = state.engine.clone();
    let built = tokio::task::spawn_blocking(move || {
        engine.store().read(|conn| {
            let now = engine.now_ms();
            let mut topics = Vec::new();
            let mut subscriptions = Vec::new();
            let mut dead_total = 0;
            for summary in queries::topic_summaries(conn)? {
                dead_total += summary.dead;
                let detail = if summary.dead > 0 {
                    format!(
                        "{}, {}",
                        counted(summary.messages, "message"),
                        counted(summary.dead, "dead letter")
                    )
                } else {
                    counted(summary.messages, "message")
                };
                topics.push(ShellTopic {
                    name: summary.name.clone(),
                    messages: summary.messages,
                    detail,
                });
                let Some(topic_id) = queries::topic_id_by_name_conn(conn, &summary.name)? else {
                    continue;
                };
                for sub in queries::subscription_summaries(conn, topic_id)? {
                    let view = SubscriptionView::at(sub, now);
                    let backlog = view.backlog;
                    let (tone, detail) = match view.state.as_str() {
                        "active" => ("ok", "active".to_string()),
                        "archived" => ("muted", "archived".to_string()),
                        other if view.dead > 0 => (
                            "bad",
                            format!("{other}: {}", counted(view.dead, "dead letter")),
                        ),
                        other => ("warn", format!("{other}: lease expired")),
                    };
                    subscriptions.push(ShellSubscription {
                        topic: summary.name.clone(),
                        name: view.name,
                        state: view.state,
                        tone,
                        detail,
                        backlog,
                    });
                }
            }
            Ok(json!({
                "topics": topics,
                "subscriptions": subscriptions,
                "dead_total": dead_total,
            }))
        })
    })
    .await
    .map_err(|error| internal(format!("the dashboard task failed: {error}")))?
    .map_err(|error: anyhow::Error| internal(format!("{error:#}")))?;
    Ok(built)
}

/// K10 · `GET /topics` — every topic on this hub. Until step 2 this was `/`;
/// the kit's status page owns `/` now and carries a Topics section.
///
/// The page's script reads the rows from one JSON block (`topics_json`), so
/// the automatic refresh, which fetches this same URL and swaps one region,
/// carries fresh numbers with it.
pub async fn topics_page(
    Extension(dash): Extension<Dashboard>,
    State(state): State<AppState>,
) -> Result<Html<String>, ApiError> {
    let engine = state.engine.clone();
    let built = tokio::task::spawn_blocking(move || -> anyhow::Result<(i64, Vec<TopicRowView>)> {
        let now = engine.now_ms();
        let rows = engine.store().read(|conn| {
            let mut rows = Vec::new();
            for summary in queries::topic_summaries(conn)? {
                let topic_id = queries::topic_id_by_name_conn(conn, &summary.name)?;
                let subscriptions = match topic_id {
                    Some(id) => queries::subscription_summaries(conn, id)?,
                    None => Vec::new(),
                };
                let dead_note = match topic_id {
                    Some(id) if summary.dead > 0 => queries::dead_letters_for_topic(conn, id, 1)?
                        .into_iter()
                        .next()
                        .map(|dead| {
                            let text = String::from_utf8_lossy(&dead.letter.payload);
                            let excerpt: String = text.chars().take(DEAD_NOTE_CHARS).collect();
                            format!("{}: {excerpt}", dead.subscription)
                        }),
                    _ => None,
                };
                rows.push(TopicRowView {
                    status: topic_status(
                        summary.dead,
                        summary.backlog,
                        summary.last_published_at,
                        now,
                    ),
                    name: summary.name,
                    messages: summary.messages,
                    flagged: subscriptions
                        .iter()
                        .filter(|sub| sub.state == "flagged")
                        .map(|sub| sub.name.clone())
                        .collect(),
                    in_flight: subscriptions.iter().map(|sub| sub.claimed).sum(),
                    lapsed: subscriptions.iter().map(|sub| sub.lapsed).sum(),
                    subscriptions: subscriptions.into_iter().map(|sub| sub.name).collect(),
                    backlog: summary.backlog,
                    dead: summary.dead,
                    last_published_ms: summary.last_published_at,
                    dead_note,
                });
            }
            Ok(rows)
        })?;
        Ok((now, rows))
    })
    .await
    .map_err(|error| internal(format!("the dashboard task failed: {error}")))?
    .map_err(|error| internal(format!("{error:#}")))?;
    let (now, topics) = built;
    // The block sits inside a <script type="application/json">: `<` is
    // escaped so a payload excerpt can never close the element.
    let topics_json = serde_json::to_string(&json!({ "now_ms": now, "topics": topics }))
        .map_err(|error| internal(format!("cannot encode the topics: {error}")))?
        .replace('<', "\\u003c");
    page(
        &dash,
        "/topics",
        TOPICS_HTML,
        json!({
            "topics": topics,
            "topics_json": topics_json,
            "shell": shell_view(&state).await?,
            "kyu_assets": ASSET_VERSION.as_str(),
        }),
    )
}

/// K2-3 · `GET /apps` — the 2.x name of the clients page; the kit's
/// `/clients` (labelled "Apps") took over, and old links keep working.
pub async fn apps_redirect() -> Redirect {
    Redirect::to("/clients")
}

/// What the topic page renders: the topic, its subscriptions, recent
/// messages, dead letters and the copy-paste commands.
struct TopicPage {
    topic: TopicView,
    subscriptions: Vec<SubscriptionView>,
    messages: Vec<MessageView>,
    dead_letters: Vec<DeadLetterView>,
    snippets: dashboard::Snippets,
}

/// K10 · `GET /t/{topic}/dashboard` — one topic in detail.
pub async fn dashboard_topic(
    Extension(dash): Extension<Dashboard>,
    State(state): State<AppState>,
    Path(topic): Path<String>,
) -> Result<Html<String>, ApiError> {
    let engine = state.engine.clone();
    let topic_name = topic.clone();
    // The copy-paste commands carry the login token, which always exists
    // (the kit refuses to start without it); a command with a client's own
    // token comes from the kit's clients page ("Copy command").
    let token = state.auth.token().map(str::to_string);
    let built = tokio::task::spawn_blocking(move || -> anyhow::Result<Option<TopicPage>> {
        engine.store().read(|conn| {
            let Some(topic_id) = queries::topic_id_by_name_conn(conn, &topic_name)? else {
                return Ok(None);
            };
            let summary = queries::topic_summaries(conn)?
                .into_iter()
                .find(|summary| summary.name == topic_name);
            let Some(summary) = summary else {
                return Ok(None);
            };
            let now = engine.now_ms();
            let subscriptions: Vec<SubscriptionView> =
                queries::subscription_summaries(conn, topic_id)?
                    .into_iter()
                    .map(|summary| SubscriptionView::at(summary, now))
                    .collect();
            let messages: Vec<MessageView> =
                queries::recent_messages(conn, topic_id, RECENT_MESSAGES)?
                    .into_iter()
                    .map(|message| MessageView::at(message, now))
                    .collect();
            let dead_letters: Vec<DeadLetterView> =
                queries::dead_letters_for_topic(conn, topic_id, DEAD_LETTERS_SHOWN)?
                    .into_iter()
                    .map(|dead| DeadLetterView::at(dead, now))
                    .collect();
            // The snippets carry this topic's own most recent payload and a
            // subscription that genuinely exists, so what you copy is what
            // your hub actually answers to (S1).
            let example = messages.first();
            let snippets = dashboard::Snippets::build(
                "",
                &topic_name,
                subscriptions
                    .iter()
                    .find(|subscription| subscription.state != "archived")
                    .map(|subscription| subscription.name.as_str()),
                example.map(|message| &message.payload),
                example.and_then(|message| message.content_type.as_deref()),
                token.as_deref(),
                None,
            );
            Ok(Some(TopicPage {
                topic: TopicView::at(summary, now),
                subscriptions,
                messages,
                dead_letters,
                snippets,
            }))
        })
    })
    .await
    .map_err(|error| internal(format!("the dashboard task failed: {error}")))?
    .map_err(|error| internal(format!("{error:#}")))?;
    match built {
        Some(built) => page(
            &dash,
            "/topics",
            TOPIC_HTML,
            json!({
                "topic": built.topic,
                "subscriptions": built.subscriptions,
                "messages": built.messages,
                "dead_letters": built.dead_letters,
                "snippets": built.snippets,
                "reveal_seconds": dash.reveal_seconds,
                "shell": shell_view(&state).await?,
                "kyu_assets": ASSET_VERSION.as_str(),
            }),
        ),
        None => Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("topic {topic:?} does not exist"),
            "a topic starts existing when something publishes to it. Open /topics to \
             see which topics this hub has.",
        )),
    }
}

/// How many backlog items a subscription page shows (W16). Bounded for the
/// same reason `DEAD_LETTERS_SHOWN` is.
const BACKLOG_SHOWN: usize = 200;

/// What the Subscription page reads from the store in one transaction.
struct SubscriptionPageRows {
    view: SubscriptionView,
    backlog: Vec<queries::BacklogItem>,
    created_at: i64,
    last_poll_ms: Option<i64>,
    oldest_ms: Option<i64>,
}

/// [W16] `GET /t/{topic}/dashboard/subs/{subscription}` — one subscription's
/// live backlog, individually rather than as a count.
pub async fn dashboard_subscription(
    Extension(dash): Extension<Dashboard>,
    State(state): State<AppState>,
    Path((topic, subscription)): Path<(String, String)>,
) -> Result<Html<String>, ApiError> {
    let engine = state.engine.clone();
    let topic_name = topic.clone();
    let subscription_name = subscription.clone();
    let built =
        tokio::task::spawn_blocking(move || -> anyhow::Result<Option<SubscriptionPageRows>> {
            engine.store().read(|conn| {
                let Some(topic_id) = queries::topic_id_by_name_conn(conn, &topic_name)? else {
                    return Ok(None);
                };
                let Some(sub_id) =
                    queries::subscription_id_by_name_conn(conn, topic_id, &subscription_name)?
                else {
                    return Ok(None);
                };
                let summary = queries::subscription_summaries(conn, topic_id)?
                    .into_iter()
                    .find(|summary| summary.name == subscription_name);
                let Some(summary) = summary else {
                    return Ok(None);
                };
                let now = engine.now_ms();
                let raw_last_poll = summary.last_poll_at;
                let raw_oldest = summary.oldest_unacked_at;
                let view = dashboard::SubscriptionView::at(summary, now);
                let backlog = queries::backlog_for_subscription(conn, sub_id, BACKLOG_SHOWN)?;
                let created_at: i64 = conn.query_row(
                    "SELECT created_at FROM subscriptions WHERE id = ?1",
                    [sub_id],
                    |row| row.get(0),
                )?;
                Ok(Some(SubscriptionPageRows {
                    view,
                    backlog,
                    created_at,
                    last_poll_ms: raw_last_poll,
                    oldest_ms: raw_oldest,
                }))
            })
        })
        .await
        .map_err(|error| internal(format!("the dashboard task failed: {error}")))?
        .map_err(|error| internal(format!("{error:#}")))?;
    match built {
        Some(SubscriptionPageRows {
            view,
            backlog: items,
            created_at,
            last_poll_ms,
            oldest_ms,
        }) => {
            let now = state.engine.now_ms();
            let rows: Vec<serde_json::Value> = items
                .into_iter()
                .map(|item| {
                    let published_ms = item.published_at;
                    let view = dashboard::BacklogItemView::at(item, now);
                    let mut row = serde_json::to_value(&view).unwrap_or_default();
                    row["published_ms"] = json!(published_ms);
                    row
                })
                .collect();
            let policy_rows = subscription_policy_rows(&view, created_at);
            // What the page's script reads: raw milliseconds and counts, so it
            // formats every time itself (dd/mm/yyyy HH:mm, Europe/Brussels).
            let page_data = json!({
                "topic": topic,
                "subscription": view.name,
                "state": view.state,
                "now_ms": now,
                "last_poll_ms": last_poll_ms,
                "oldest_unacked_ms": oldest_ms,
                "pending": view.backlog,
                "claimed": view.claimed,
                "dead": view.dead,
                "lease_ms": view.lease_ms,
                "max_attempts": view.max_attempts,
                "shown": rows.len(),
                "items": rows.iter().map(|row| json!({
                    "id": row["id"],
                    "state": row["state"],
                    "published_ms": row["published_ms"],
                    "attempts": row["attempts"],
                })).collect::<Vec<_>>(),
            });
            let page_data = page_data
                .to_string()
                .replace('<', "\\u003c")
                .replace('>', "\\u003e")
                .replace('&', "\\u0026");
            let lease_words = format_duration_ms(view.lease_ms);
            page(
                &dash,
                "/topics",
                SUBSCRIPTION_HTML,
                json!({
                    "topic_name": topic,
                    "subscription": view,
                    "backlog": rows,
                    "backlog_total": rows.len(),
                    "policy_rows": policy_rows,
                    "lease_words": lease_words,
                    "page_data": page_data,
                    "shell": shell_view(&state).await?,
                    "kyu_assets": ASSET_VERSION.as_str(),
                }),
            )
        }
        None => Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("subscription {subscription:?} on topic {topic:?} does not exist"),
            "a subscription is created by polling, so this name has never polled this \
             topic. Check the spelling, or open the topic's own page to see which \
             subscriptions it has.",
        )),
    }
}

/// A duration in words a person reads: `30 s`, `2 min`, `7 days`.
fn format_duration_ms(ms: i64) -> String {
    let trim = |value: f64| {
        let text = format!("{value:.1}");
        text.strip_suffix(".0").map_or(text.clone(), str::to_string)
    };
    match ms {
        i64::MIN..=-1 => "0 s".to_string(),
        0..=999 => format!("{ms} ms"),
        1_000..=59_999 => format!("{} s", trim(ms as f64 / 1_000.0)),
        60_000..=3_599_999 => format!("{} min", trim(ms as f64 / 60_000.0)),
        3_600_000..=86_399_999 => format!("{} h", trim(ms as f64 / 3_600_000.0)),
        _ => {
            let days = trim(ms as f64 / 86_400_000.0);
            format!("{days} {}", if days == "1" { "day" } else { "days" })
        }
    }
}

/// The policy card's rows: label, value, and whether kyu chose the value
/// itself (the page prints a quiet "(default)" beside those).
fn subscription_policy_rows(view: &SubscriptionView, created_at: i64) -> Vec<serde_json::Value> {
    let wait = if view.backoff_ms == 0 {
        "No wait".to_string()
    } else {
        format!(
            "{}, times the attempt number",
            format_duration_ms(view.backoff_ms)
        )
    };
    vec![
        json!({ "label": "Lease", "value": format_duration_ms(view.lease_ms),
                "is_default": view.explicit.lease_ms.is_none() }),
        json!({ "label": "Attempts before a message is dead", "value": view.max_attempts.to_string(),
                "is_default": view.explicit.max_attempts.is_none() }),
        json!({ "label": "Wait between attempts", "value": wait,
                "is_default": view.explicit.backoff_ms.is_none() }),
        json!({ "label": "Time to live",
                "value": view.ttl_ms.map_or("No limit".to_string(), format_duration_ms),
                "is_default": view.explicit.ttl_ms.is_none() }),
        json!({ "label": "Created", "created_ms": created_at, "value": "", "is_default": false }),
    ]
}

/// W9 · `POST /t/{topic}/dashboard/publish` — the test-publish form.
///
/// Answers the question that costs the most time at 23:00: is the producer
/// broken, or the consumer? One click puts a known message on the topic.
pub async fn dashboard_publish(
    State(state): State<AppState>,
    Path(topic): Path<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Response, ApiError> {
    // A browser form posts urlencoded; take the payload field out of it.
    let form = String::from_utf8_lossy(&body);
    let payload = form
        .split('&')
        .find_map(|pair| pair.strip_prefix("payload="))
        .map(urldecode)
        .unwrap_or_default();

    let engine = state.engine.clone();
    let topic_for_engine = topic.clone();
    let published = spawn_engine_write(move || {
        engine.publish(
            &topic_for_engine,
            payload.as_bytes(),
            Some("application/json"),
        )
    })
    .await?;

    state.notifiers.wake(&topic, &published.delivered_to);
    tracing::info!(topic = %topic, id = %published.id, "test message published from the dashboard");

    if wants_json(&headers) {
        return Ok(axum::Json(json!({ "ok": true, "id": published.id })).into_response());
    }
    Ok(Redirect::to(&format!("/t/{topic}/dashboard")).into_response())
}

/// K6 · `POST /t/{topic}/dashboard/requeue` — the requeue button.
pub async fn dashboard_requeue(
    State(state): State<AppState>,
    Path(topic): Path<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Response, ApiError> {
    let form = String::from_utf8_lossy(&body);
    let field = |name: &str| {
        form.split('&')
            .find_map(|pair| pair.strip_prefix(&format!("{name}=")))
            .map(urldecode)
    };

    let subscription = field("subscription").ok_or_else(|| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "the requeue form did not name a subscription",
            "use the button on the dashboard's dead-letter table, or call \
             POST /api/t/<topic>/subs/<subscription>/dead/<id>/requeue directly.",
        )
    })?;
    let id = field("id").ok_or_else(|| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "the requeue form did not name a message",
            "use the button on the dashboard's dead-letter table, which fills this in.",
        )
    })?;

    let engine = state.engine.clone();
    let topic_for_engine = topic.clone();
    let subscription_for_wake = subscription.clone();
    spawn_engine_write(move || engine.requeue_dead(&topic_for_engine, &subscription, &id)).await?;

    state
        .notifiers
        .wake(&topic, std::slice::from_ref(&subscription_for_wake));

    Ok(done(&headers, &format!("/t/{topic}/dashboard")))
}

/// [W15, W16] `POST /t/{topic}/dashboard/delivery/delete` — the Delete
/// button beside Requeue on the dead-letter table, and the same button on
/// the per-subscription backlog view.
pub async fn dashboard_delete_delivery(
    State(state): State<AppState>,
    Path(topic): Path<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Response, ApiError> {
    let form = String::from_utf8_lossy(&body);
    let field = |name: &str| {
        form.split('&')
            .find_map(|pair| pair.strip_prefix(&format!("{name}=")))
            .map(urldecode)
    };

    let subscription = field("subscription").ok_or_else(|| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "the delete form did not name a subscription",
            "use the button on the dashboard, or call \
             POST /api/t/<topic>/subs/<subscription>/deliveries/<id>/delete directly.",
        )
    })?;
    let id = field("id").ok_or_else(|| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "the delete form did not name a message",
            "use the button on the dashboard, which fills this in.",
        )
    })?;

    let engine = state.engine.clone();
    let topic_for_engine = topic.clone();
    spawn_engine_write(move || engine.delete_delivery(&topic_for_engine, &subscription, &id))
        .await?;

    Ok(done(&headers, &format!("/t/{topic}/dashboard")))
}

/// `POST /t/{topic}/dashboard/subs/{subscription}/archive` — the Archive
/// button on the topic page's subscriptions table (Kenny, 2026-09-30).
pub async fn dashboard_archive_subscription(
    State(state): State<AppState>,
    Path((topic, subscription)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let engine = state.engine.clone();
    let (t, s) = (topic.clone(), subscription.clone());
    if let Some(lapsed) = spawn_engine_write(move || engine.archive(&t, &s)).await? {
        tracing::info!(%topic, %subscription, lapsed, "subscription archived from the dashboard");
    }
    Ok(done(&headers, &format!("/t/{topic}/dashboard")))
}

/// `POST /t/{topic}/dashboard/subs/{subscription}/delete` — the Delete
/// button an archived subscription gets instead (Kenny, 2026-09-30).
pub async fn dashboard_delete_subscription(
    State(state): State<AppState>,
    Path((topic, subscription)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let engine = state.engine.clone();
    let (t, s) = (topic.clone(), subscription.clone());
    spawn_engine_write(move || engine.delete_subscription(&t, &s)).await?;
    tracing::info!(%topic, %subscription, "subscription deleted from the dashboard");
    Ok(done(&headers, &format!("/t/{topic}/dashboard")))
}

/// [K-actions] `POST /dashboard/dead-letters/prune` — the status page's
/// "Prune every dead letter" button (`kit::TopicsSection::actions`): every
/// topic and subscription in one confirmed sweep, after the storm that
/// caused them is fixed, instead of one topic page at a time.
///
/// Reached through the kit's `[data-post]` mechanism, not a classic form:
/// it wants JSON, not a redirect, and reloads the page itself on 2xx.
pub async fn dashboard_prune_dead_letters(
    State(state): State<AppState>,
) -> Result<Response, ApiError> {
    let engine = state.engine.clone();
    let count = spawn_engine_write(move || engine.prune_dead_letters()).await?;
    Ok(axum::Json(json!({ "pruned": count })).into_response())
}

/// Whether the page's script asked for JSON instead of the no-script
/// redirect (`Accept: application/json`).
fn wants_json(headers: &HeaderMap) -> bool {
    headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|accept| accept.contains("application/json"))
}

/// How a dashboard form post ends: `{ "ok": true }` for a script, the
/// redirect back to the page for a browser form.
fn done(headers: &HeaderMap, location: &str) -> Response {
    if wants_json(headers) {
        axum::Json(json!({ "ok": true })).into_response()
    } else {
        Redirect::to(location).into_response()
    }
}

// ─── kp-themes 11 · data the pages fetch ────────────────────────────────────

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

fn unknown_subscription(topic: &str, subscription: &str) -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        format!("subscription {subscription:?} on topic {topic:?} does not exist"),
        "a subscription is created by polling, so this name has never polled this \
         topic. Check the spelling, or open the topic's own page to see which \
         subscriptions it has.",
    )
}

#[derive(Debug, Deserialize)]
pub struct AttemptsQuery {
    pub id: Option<String>,
}

/// `GET /t/{topic}/dashboard/subs/{name}/attempts?id=…` — the delivery
/// attempts of one message on one subscription.
pub async fn dashboard_attempts(
    State(state): State<AppState>,
    Path((topic, subscription)): Path<(String, String)>,
    Query(query): Query<AttemptsQuery>,
) -> Result<Response, ApiError> {
    let Some(id) = query.id.filter(|id| !id.is_empty()) else {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "the id parameter is required: which message's attempts?",
            "add ?id=<message id>, for example one from the subscription's backlog.",
        ));
    };
    let engine = state.engine.clone();
    let (t, s, i) = (topic.clone(), subscription.clone(), id.clone());
    let attempts = spawn_engine(move || {
        engine
            .store()
            .read(|conn| {
                let Some(topic_id) = queries::topic_id_by_name_conn(conn, &t)? else {
                    return Ok(None);
                };
                let Some(sub_id) = queries::subscription_id_by_name_conn(conn, topic_id, &s)?
                else {
                    return Ok(None);
                };
                Ok(Some(queries::attempts_for(conn, sub_id, &i)?))
            })
            .map_err(EngineError::Internal)
    })
    .await?;
    let attempts = match attempts {
        None => return Err(unknown_subscription(&topic, &subscription)),
        Some(None) => {
            return Err(EngineError::NoSuchDelivery { id, subscription }.into());
        }
        Some(Some(rows)) => rows,
    };
    let rows: Vec<_> = attempts
        .into_iter()
        .map(|attempt| {
            json!({
                "n": attempt.n,
                "claimed_at": attempt.claimed_at,
                "ended_at": attempt.ended_at,
                "outcome": attempt.outcome,
                "app": attempt.app,
                "reason": attempt.reason.filter(|reason| !reason.is_empty()),
            })
        })
        .collect();
    Ok(axum::Json(json!({ "message": id, "attempts": rows })).into_response())
}

#[derive(Debug, Deserialize)]
pub struct DaysQuery {
    pub days: Option<i64>,
}

/// `GET /t/{topic}/dashboard/subs/{name}/daily?days=90` — delivered and
/// dead per Brussels day, every day of the window present.
pub async fn dashboard_daily(
    State(state): State<AppState>,
    Path((topic, subscription)): Path<(String, String)>,
    Query(query): Query<DaysQuery>,
) -> Result<Response, ApiError> {
    let days = query.days.unwrap_or(90).clamp(1, queries::DAILY_KEPT_DAYS);
    let engine = state.engine.clone();
    let (t, s) = (topic.clone(), subscription.clone());
    let found = spawn_engine(move || {
        engine
            .store()
            .read(|conn| {
                let Some(topic_id) = queries::topic_id_by_name_conn(conn, &t)? else {
                    return Ok(None);
                };
                if queries::subscription_id_by_name_conn(conn, topic_id, &s)?.is_none() {
                    return Ok(None);
                }
                let today = crate::engine::days::day_of(engine.now_ms());
                let from = today - days + 1;
                let rows = queries::daily_for(conn, &t, &s, from, today)?;
                Ok(Some((from, today, rows)))
            })
            .map_err(EngineError::Internal)
    })
    .await?;
    let Some((from, today, rows)) = found else {
        return Err(unknown_subscription(&topic, &subscription));
    };
    let by_day: std::collections::HashMap<i64, (i64, i64)> = rows
        .into_iter()
        .map(|(day, delivered, dead)| (day, (delivered, dead)))
        .collect();
    let days: Vec<_> = (from..=today)
        .map(|day| {
            let (delivered, dead) = by_day.get(&day).copied().unwrap_or((0, 0));
            json!({ "day": crate::engine::days::day_label(day), "delivered": delivered, "dead": dead })
        })
        .collect();
    Ok(axum::Json(json!({ "days": days })).into_response())
}

#[derive(Debug, Deserialize)]
pub struct HoursQuery {
    pub hours: Option<i64>,
}

/// `GET /dashboard/stats/hourly?hours=24` — published and dead per topic
/// per hour; `hours` lists the hour starts, oldest first, and every topic's
/// arrays line up with it.
pub async fn dashboard_stats_hourly(
    State(state): State<AppState>,
    Query(query): Query<HoursQuery>,
) -> Result<Response, ApiError> {
    use crate::engine::days;
    let hours = query.hours.unwrap_or(24).clamp(1, 720);
    let engine = state.engine.clone();
    let (starts, names, rows) = spawn_engine(move || {
        let last = days::hour_of(engine.now_ms());
        let first = last - (hours - 1) * days::HOUR;
        engine
            .store()
            .read(|conn| {
                let names = queries::topic_names_conn(conn)?;
                let rows = queries::hourly_since(conn, first)?;
                Ok(((first, last), names, rows))
            })
            .map_err(EngineError::Internal)
    })
    .await?;
    let (first, last) = starts;
    let starts: Vec<i64> = (0..hours).map(|index| first + index * days::HOUR).collect();
    let mut table: std::collections::BTreeMap<String, (Vec<i64>, Vec<i64>)> = names
        .into_iter()
        .map(|name| (name, (vec![0; hours as usize], vec![0; hours as usize])))
        .collect();
    for (topic, hour, published, dead) in rows {
        if hour > last {
            continue;
        }
        if let Some((published_row, dead_row)) = table.get_mut(&topic) {
            let index = ((hour - first) / days::HOUR) as usize;
            published_row[index] = published;
            dead_row[index] = dead;
        }
    }
    let topics: Vec<_> = table
        .into_iter()
        .map(|(topic, (published, dead))| json!({ "topic": topic, "published": published, "dead": dead }))
        .collect();
    Ok(axum::Json(json!({ "hours": starts, "topics": topics })).into_response())
}

/// `GET /dashboard/stats/totals?hours=24` — the hub totals the sweeper
/// samples every five minutes, oldest first.
pub async fn dashboard_stats_totals(
    State(state): State<AppState>,
    Query(query): Query<HoursQuery>,
) -> Result<Response, ApiError> {
    let hours = query.hours.unwrap_or(24).clamp(1, 48);
    let engine = state.engine.clone();
    let samples = spawn_engine(move || {
        let from = engine.now_ms() - hours * 3_600_000;
        engine
            .store()
            .read(|conn| queries::samples_since(conn, from))
            .map_err(EngineError::Internal)
    })
    .await?;
    let samples: Vec<_> = samples
        .into_iter()
        .map(|sample| {
            json!({
                "at": sample.at,
                "messages": sample.messages,
                "backlog": sample.backlog,
                "dead": sample.dead,
                "in_flight": sample.in_flight,
            })
        })
        .collect();
    Ok(axum::Json(json!({ "samples": samples })).into_response())
}

/// `GET /t/{topic}/dashboard/retention` — how long dead letters stay. A dead
/// letter lives as long as its message, which the topic's retention window
/// decides (explicit, else the hub default); `null` means kept forever.
pub async fn dashboard_retention(
    State(state): State<AppState>,
    Path(topic): Path<String>,
) -> Result<Response, ApiError> {
    const DAY_MS: i64 = 24 * 60 * 60 * 1_000;
    let engine = state.engine.clone();
    let t = topic.clone();
    let (effective, explicit) = spawn_engine(move || engine.retention(&t)).await?;
    Ok(axum::Json(json!({
        "dead_letter_days": effective.map(|ms| (ms + DAY_MS - 1) / DAY_MS),
        "retention_ms": effective,
        "explicit": explicit.is_some(),
    }))
    .into_response())
}

fn urldecode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        out.push(byte);
                        index += 3;
                    }
                    Err(_) => {
                        out.push(bytes[index]);
                        index += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn internal(message: String) -> ApiError {
    tracing::error!(error = %message, "dashboard failure");
    ApiError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "the dashboard could not be rendered",
        "check the hub's logs for the matching error line; the message API is \
         unaffected by a dashboard fault.",
    )
}

// ─── L8 · metrics and backup (W1, W8) ───────────────────────────────────────

/// W8 · `POST /api/backup`
///
/// Writes a consistent copy of the store while the hub keeps running.
pub async fn backup(State(state): State<AppState>) -> Result<Response, ApiError> {
    let engine = state.engine.clone();

    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<(String, u64)> {
        let store = engine.store();
        let Some(path) = store.path() else {
            anyhow::bail!("this hub runs on an in-memory store, which has nothing to back up");
        };
        let directory = path.parent().unwrap_or(std::path::Path::new("."));
        // Named for the moment it was taken, so several backups coexist and
        // the newest is obvious from a directory listing.
        let target = directory.join(format!("kyu.backup-{}.db", engine.now_ms()));
        let bytes = store.backup_to(&target)?;
        Ok((target.display().to_string(), bytes))
    })
    .await
    .map_err(|error| internal(format!("the backup task failed: {error}")))?;

    match result {
        Ok((path, bytes)) => {
            tracing::info!(path = %path, bytes, "backup written");
            Ok((
                StatusCode::OK,
                axum::Json(json!({
                    "backup": path,
                    "bytes": bytes,
                    "restore": "stop the hub, replace kyu.db with this file (removing any \
                                kyu.db-wal and kyu.db-shm beside it), then start it \
                                again. The backup is a complete database, not a partial copy.",
                })),
            )
                .into_response())
        }
        Err(error) => Err(ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("the backup failed: {error:#}"),
            "check free space and that the data directory is writable. kyu keeps \
             serving either way; a failed backup does not affect delivery.",
        )),
    }
}

// ---------------------------------------------------------------------------
// W2 · the door: static assets, login, logout and app management.
// ---------------------------------------------------------------------------

/// The hub's own two assets, beyond what the kit serves under `/static`:
/// the page styles and the copy/reveal behaviour of the command snippets.
/// Embedded in the binary, so the image stays a single file (T9).
/// kyu's own browser files, served from memory on `/assets/{file}`: one
/// script and one stylesheet per page beside the shared ones, so each page
/// carries only what it uses (kp-themes 11.0.0 adoption).
const ASSETS: &[(&str, &str, &str)] = &[
    (
        "app.js",
        include_str!("../../static/app.js"),
        "text/javascript; charset=utf-8",
    ),
    (
        "common.js",
        include_str!("../../static/common.js"),
        "text/javascript; charset=utf-8",
    ),
    (
        "topics.js",
        include_str!("../../static/topics.js"),
        "text/javascript; charset=utf-8",
    ),
    (
        "topic.js",
        include_str!("../../static/topic.js"),
        "text/javascript; charset=utf-8",
    ),
    (
        "subscription.js",
        include_str!("../../static/subscription.js"),
        "text/javascript; charset=utf-8",
    ),
    (
        "kyu.css",
        include_str!("../../static/kyu.css"),
        "text/css; charset=utf-8",
    ),
    (
        "topics.css",
        include_str!("../../static/topics.css"),
        "text/css; charset=utf-8",
    ),
    (
        "topic.css",
        include_str!("../../static/topic.css"),
        "text/css; charset=utf-8",
    ),
    (
        "subscription.css",
        include_str!("../../static/subscription.css"),
        "text/css; charset=utf-8",
    ),
];

/// The cache-busting fingerprint every page appends to its asset URLs.
///
/// Without it the year-long cache header below would serve yesterday's
/// JavaScript after an upgrade — which is exactly how a fixed bug appears
/// not to be fixed. The assets only ever change when the binary does, so
/// hashing their contents at first use is both sufficient and free.
pub static ASSET_VERSION: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    // FNV-1a: not cryptographic, and does not need to be — this answers
    // "did these bytes change", nothing more (T6: no crate for six lines).
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in ASSETS.iter().flat_map(|(_, body, _)| body.bytes()) {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
});

/// `GET /assets/{file}` — served from memory, never from disk.
///
/// An explicit match rather than a path join: a lookup that builds a path
/// from user input is how a static handler turns into a file-disclosure
/// bug, and there are exactly two files.
pub async fn kyu_asset(Path(file): Path<String>) -> Response {
    let Some(&(_, body, content_type)) = ASSETS.iter().find(|(name, _, _)| *name == file) else {
        return ApiError::new(
            StatusCode::NOT_FOUND,
            format!("no asset named {file:?}"),
            "the hub serves its own page scripts and stylesheets here; everything else is the kit's, under /static",
        )
        .into_response();
    };
    (
        [
            (header::CONTENT_TYPE, content_type),
            // Safe because every reference carries ?v=<ASSET_VERSION>.
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        body,
    )
        .into_response()
}
