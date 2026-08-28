//! The three verbs (K1, K2, K3) over the contract frozen in AR2.
//!
//! Handlers translate HTTP into engine calls and back. No delivery logic
//! lives here — only the two things that are genuinely HTTP's business:
//! how a message is shaped on the wire, and how a long poll waits.

use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
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

/// W6 · `GET /healthz`
///
/// Answers on the two things that can be broken while the process still
/// accepts connections: a store that cannot be written to, and a sweeper
/// that has stopped. Both are invisible from outside otherwise — publishes
/// fail one by one, or messages quietly stop coming back.
pub async fn healthz(State(state): State<AppState>) -> Response {
    let engine = state.engine.clone();
    let store_error = tokio::task::spawn_blocking(move || {
        // Two questions, because they fail differently. The probe takes the
        // write lock, which catches a read-only store. The failure record
        // catches what a probe cannot: a disk that is full but writable only
        // refuses at commit time, so the hub reports that a write actually
        // failed rather than predicting that one would.
        engine.store().probe_writable()?;
        if let Some(ago) = engine.store().recent_write_failure() {
            anyhow::bail!(
                "a write failed {} seconds ago; the store may be full",
                ago.as_secs()
            );
        }
        Ok::<(), anyhow::Error>(())
    })
    .await
    .map_err(|error| format!("the health probe did not run: {error}"))
    .and_then(|probe| probe.map_err(|error| format!("{error:#}")))
    .err();

    let now = state.engine.now_ms();
    let sweeper_alive = state.heartbeat.is_alive(now);

    let healthy = store_error.is_none() && sweeper_alive;
    let status = if healthy {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    let mut body = json!({
        "status": if healthy { "ok" } else { "degraded" },
        "store": if store_error.is_none() { "writable" } else { "unwritable" },
        "sweeper": if sweeper_alive { "alive" } else { "stalled" },
    });
    let map = body.as_object_mut().expect("a JSON object");

    if let Some(error) = &store_error {
        tracing::error!(error = %error, "the store is not writable");
        map.insert("error".to_string(), json!(error));
        map.insert(
            "remedy".to_string(),
            json!(
                "check free space on the data volume first, then that it is still \
                 mounted, writable by this user and not locked by another process. \
                 mailbox reports itself unhealthy rather than accepting publishes it \
                 cannot store, and recovers by itself once writes succeed again."
            ),
        );
    } else if !sweeper_alive {
        let behind_ms = now.saturating_sub(state.heartbeat.last_beat_ms());
        tracing::error!(behind_ms, "the sweeper has stopped");
        map.insert(
            "error".to_string(),
            json!(format!("the sweeper last ran {behind_ms} ms ago")),
        );
        map.insert(
            "remedy".to_string(),
            json!(
                "restart the container. While the sweeper is stopped, expired leases are \
                 not returned to the queue and nothing is dead-lettered, so messages \
                 appear to hang rather than to fail."
            ),
        );
    }

    (status, axum::Json(body)).into_response()
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
    // `curl -d` sends form-urlencoded unless told otherwise (AR2). mailbox
    // reports what it was given rather than guessing.
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);

    let engine = state.engine.clone();
    let topic_for_engine = topic.clone();
    let published = spawn_engine(move || {
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
    let mut backfilled = 0usize;

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
    let settled = spawn_engine(move || {
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
    let effective = spawn_engine(move || engine.set_policy(&topic, &subscription, stored)).await?;
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
    spawn_engine(move || engine.requeue_dead(&topic, &subscription, &id)).await?;

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

// ─── L6 · history and lifecycle (K8, K9, K11) ───────────────────────────────

/// K11 · `POST /api/t/{topic}/subs/{sub}/unarchive`
pub async fn unarchive(
    State(state): State<AppState>,
    Path((topic, subscription)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let engine = state.engine.clone();
    let topic_for_response = topic.clone();
    let subscription_for_response = subscription.clone();
    let changed = spawn_engine(move || engine.unarchive(&topic, &subscription)).await?;

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
    spawn_engine(move || engine.set_retention(&topic, retention)).await?;

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

/// K10 · `GET /` — every topic on the hub.
pub async fn dashboard_index(State(state): State<AppState>) -> Result<Html<String>, ApiError> {
    let engine = state.engine.clone();
    let protected = state.auth.is_protected();
    let page = tokio::task::spawn_blocking(move || -> anyhow::Result<String> {
        let topics = engine
            .store()
            .read(queries::topic_summaries)?
            .into_iter()
            .map(TopicView::from)
            .collect();
        dashboard::render_topics(topics, engine.now_ms(), protected)
    })
    .await
    .map_err(|error| internal(format!("the dashboard task failed: {error}")))?
    .map_err(|error| internal(format!("{error:#}")))?;

    Ok(Html(page))
}

/// `?app=` on the topic page: whose token the printed commands carry (W2).
#[derive(Debug, Deserialize, Default)]
pub struct TopicPageQuery {
    pub app: Option<String>,
}

/// K10 · `GET /t/{topic}/dashboard` — one topic in detail.
pub async fn dashboard_topic(
    State(state): State<AppState>,
    Path(topic): Path<String>,
    Query(query): Query<TopicPageQuery>,
) -> Result<Html<String>, ApiError> {
    let engine = state.engine.clone();
    let topic_name = topic.clone();
    let protected = state.auth.is_protected();

    // Which token the copy-paste commands carry. `?app=` picks a registered
    // app; without it the commands use the bootstrap token, which always
    // exists on a protected hub — so a pasted command works on the first try
    // even before any app is registered.
    let (token, app) = match (state.auth.key(), query.app.as_deref()) {
        (Some(key), Some(name)) => {
            let found = engine
                .app_token(name, key)
                .map_err(ApiError::from)?
                .ok_or_else(|| {
                    ApiError::from(EngineError::UnknownApp {
                        name: name.to_string(),
                    })
                })?;
            (Some(found), Some(name.to_string()))
        }
        _ => (state.auth.token().map(str::to_string), None),
    };

    let app_names = match state.auth.key() {
        Some(key) => engine
            .list_apps(key)
            .map_err(ApiError::from)?
            .into_iter()
            .filter(|app| app.is_live())
            .map(|app| app.name)
            .collect(),
        None => Vec::new(),
    };

    let page = tokio::task::spawn_blocking(move || -> anyhow::Result<Option<String>> {
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

            let subscriptions: Vec<SubscriptionView> =
                queries::subscription_summaries(conn, topic_id)?
                    .into_iter()
                    .map(SubscriptionView::from)
                    .collect();
            let messages: Vec<MessageView> =
                queries::recent_messages(conn, topic_id, RECENT_MESSAGES)?
                    .into_iter()
                    .map(MessageView::from)
                    .collect();
            let dead_letters: Vec<DeadLetterView> =
                queries::dead_letters_for_topic(conn, topic_id, DEAD_LETTERS_SHOWN)?
                    .into_iter()
                    .map(DeadLetterView::from)
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
                app.as_deref(),
            );

            Ok(Some(dashboard::render_topic(
                TopicView::from(summary),
                subscriptions,
                messages,
                dead_letters,
                snippets,
                engine.now_ms(),
                protected,
                app_names,
            )?))
        })
    })
    .await
    .map_err(|error| internal(format!("the dashboard task failed: {error}")))?
    .map_err(|error| internal(format!("{error:#}")))?;

    match page {
        Some(page) => Ok(Html(page)),
        None => Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("topic {topic:?} does not exist"),
            "a topic starts existing when something publishes to it. Open / to see \
             which topics this hub has.",
        )),
    }
}

/// W9 · `POST /t/{topic}/dashboard/publish` — the test-publish form.
///
/// Answers the question that costs the most time at 23:00: is the producer
/// broken, or the consumer? One click puts a known message on the topic.
pub async fn dashboard_publish(
    State(state): State<AppState>,
    Path(topic): Path<String>,
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
    let published = spawn_engine(move || {
        engine.publish(
            &topic_for_engine,
            payload.as_bytes(),
            Some("application/json"),
        )
    })
    .await?;

    state.notifiers.wake(&topic, &published.delivered_to);
    tracing::info!(topic = %topic, id = %published.id, "test message published from the dashboard");

    Ok(Redirect::to(&format!("/t/{topic}/dashboard")).into_response())
}

/// K6 · `POST /t/{topic}/dashboard/requeue` — the requeue button.
pub async fn dashboard_requeue(
    State(state): State<AppState>,
    Path(topic): Path<String>,
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
    spawn_engine(move || engine.requeue_dead(&topic_for_engine, &subscription, &id)).await?;

    state
        .notifiers
        .wake(&topic, std::slice::from_ref(&subscription_for_wake));

    Ok(Redirect::to(&format!("/t/{topic}/dashboard")).into_response())
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

/// W1 · `GET /metrics` in Prometheus text format.
///
/// The series that answer "is something silently broken": a growing backlog,
/// a dead-letter count above zero, an oldest-unacked age that keeps rising.
pub async fn metrics(State(state): State<AppState>) -> Result<Response, ApiError> {
    let engine = state.engine.clone();
    let heartbeat = state.heartbeat.clone();

    let body = tokio::task::spawn_blocking(move || -> anyhow::Result<String> {
        let now = engine.now_ms();
        engine.store().read(|conn| {
            let counts = queries::delivery_counts(conn)?;
            let topics = queries::scalar(conn, "SELECT count(*) FROM topics")?;
            let subscriptions = queries::scalar(conn, "SELECT count(*) FROM subscriptions")?;
            let messages = queries::scalar(conn, "SELECT count(*) FROM messages")?;
            let bytes = queries::scalar(
                conn,
                "SELECT page_count * page_size FROM pragma_page_count(), pragma_page_size()",
            )
            .unwrap_or(0);

            let mut out = String::new();
            out.push_str("# HELP mailbox_topics Topics on this hub.\n");
            out.push_str("# TYPE mailbox_topics gauge\n");
            out.push_str(&format!("mailbox_topics {topics}\n"));
            out.push_str("# HELP mailbox_subscriptions Subscriptions across all topics.\n");
            out.push_str("# TYPE mailbox_subscriptions gauge\n");
            out.push_str(&format!("mailbox_subscriptions {subscriptions}\n"));
            out.push_str("# HELP mailbox_messages Messages currently retained.\n");
            out.push_str("# TYPE mailbox_messages gauge\n");
            out.push_str(&format!("mailbox_messages {messages}\n"));
            out.push_str("# HELP mailbox_store_bytes Size of the store on disk.\n");
            out.push_str("# TYPE mailbox_store_bytes gauge\n");
            out.push_str(&format!("mailbox_store_bytes {bytes}\n"));
            out.push_str(
                "# HELP mailbox_deliveries Deliveries by topic, subscription and state.\n",
            );
            out.push_str("# TYPE mailbox_deliveries gauge\n");
            for count in counts {
                out.push_str(&format!(
                    "mailbox_deliveries{{topic=\"{}\",subscription=\"{}\",state=\"{}\"}} {}\n",
                    escape_label(&count.topic),
                    escape_label(&count.subscription),
                    count.state,
                    count.count
                ));
            }
            out.push_str("# HELP mailbox_sweeper_age_ms Time since the sweeper last ran.\n");
            out.push_str("# TYPE mailbox_sweeper_age_ms gauge\n");
            out.push_str(&format!(
                "mailbox_sweeper_age_ms {}\n",
                now.saturating_sub(heartbeat.last_beat_ms())
            ));
            Ok(out)
        })
    })
    .await
    .map_err(|error| internal(format!("the metrics task failed: {error}")))?
    .map_err(|error| internal(format!("{error:#}")))?;

    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        body,
    )
        .into_response())
}

/// Label values are topic and subscription names, which AR8 already limits
/// to `[a-z0-9._-]` — this is the belt to that braces.
fn escape_label(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

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
        let target = directory.join(format!("mailbox.backup-{}.db", engine.now_ms()));
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
                    "restore": "stop the hub, replace mailbox.db with this file (removing any \
                                mailbox.db-wal and mailbox.db-shm beside it), then start it \
                                again. The backup is a complete database, not a partial copy.",
                })),
            )
                .into_response())
        }
        Err(error) => Err(ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("the backup failed: {error:#}"),
            "check free space and that the data directory is writable. mailbox keeps \
             serving either way; a failed backup does not affect delivery.",
        )),
    }
}

// ---------------------------------------------------------------------------
// W2 · the door: static assets, login, logout and app management.
// ---------------------------------------------------------------------------

/// The two files the pages need, compiled into the binary like the
/// templates (T4 amendment) so the container stays one artifact and the
/// distroless image needs no filesystem layout.
const BOOTSTRAP_CSS: &str = include_str!("../../static/bootstrap.min.css");
const APP_JS: &str = include_str!("../../static/app.js");

/// `GET /static/{file}` — served from memory, never from disk.
///
/// An explicit match rather than a path join: a lookup that builds a path
/// from user input is how a static handler turns into a file-disclosure
/// bug, and there are exactly two files.
pub async fn static_asset(Path(file): Path<String>) -> Response {
    let (body, content_type) = match file.as_str() {
        "bootstrap.min.css" => (BOOTSTRAP_CSS, "text/css; charset=utf-8"),
        "app.js" => (APP_JS, "text/javascript; charset=utf-8"),
        _ => {
            return ApiError::new(
                StatusCode::NOT_FOUND,
                format!("mailbox serves no asset named {file:?}"),
                "the dashboard needs only bootstrap.min.css and app.js.".to_string(),
            )
            .into_response();
        }
    };
    (
        [
            (header::CONTENT_TYPE, content_type),
            // Immutable for a day: these change only when the binary does,
            // and a dashboard that refetches 230 kB on every page view is
            // needlessly slow over wifi.
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        body,
    )
        .into_response()
}

/// `GET /login` — the form. Already-authenticated visitors are sent on
/// rather than shown a login page they do not need.
pub async fn login_form(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !state.auth.is_protected() {
        return Redirect::to("/").into_response();
    }
    if let Some(candidate) = super::auth::session_cookie(&headers)
        && super::auth::authenticate(&state, &candidate).is_some()
    {
        return Redirect::to("/").into_response();
    }
    match dashboard::render_login(None) {
        Ok(page) => Html(page).into_response(),
        Err(error) => ApiError::from(EngineError::Internal(error)).into_response(),
    }
}

/// `POST /login` — check the token, set the cookie.
///
/// A wrong token re-renders the form with a message and **200**, not 401:
/// the response is a page, and a browser handling a 401 by popping its own
/// credential dialog on top of our form is a confusing mess.
pub async fn login(State(state): State<AppState>, body: axum::body::Bytes) -> Response {
    let form = FormFields::parse(&body);
    let token = form.get("token").unwrap_or_default();
    let remember = form.get("remember").is_some();

    if super::auth::authenticate(&state, &token).is_none() {
        // Deliberately vague, and deliberately slow to be worth guessing:
        // the token is long enough that online guessing is hopeless, so no
        // artificial delay is added — but nothing here says whether the
        // token was wrong, expired or revoked.
        tracing::warn!("a login attempt was refused");
        return match dashboard::render_login(Some(
            "That token was not accepted. Check the value of MAILBOX_TOKEN in \
             your compose file, or use a token from the apps page.",
        )) {
            Ok(page) => (StatusCode::OK, Html(page)).into_response(),
            Err(error) => ApiError::from(EngineError::Internal(error)).into_response(),
        };
    }

    tracing::info!(remember, "a dashboard session started");
    (
        [(
            header::SET_COOKIE,
            super::auth::set_cookie_value(&token, remember),
        )],
        Redirect::to("/"),
    )
        .into_response()
}

/// `POST /logout` — drop the cookie.
pub async fn logout() -> Response {
    (
        [(header::SET_COOKIE, super::auth::clear_cookie_value())],
        Redirect::to("/login"),
    )
        .into_response()
}

/// `GET /apps` — register, inspect and revoke the apps that may talk to the
/// hub (W2). Tokens render masked; the reveal and copy controls live in
/// `app.js`.
pub async fn apps_page(State(state): State<AppState>) -> Result<Html<String>, ApiError> {
    let Some(key) = state.auth.key().cloned() else {
        return Err(ApiError::from(EngineError::Unprotected));
    };
    let engine = state.engine.clone();
    let now = engine.now_ms();
    let apps = spawn_engine(move || engine.list_apps(&key)).await?;
    let views: Vec<dashboard::AppView> = apps
        .into_iter()
        .map(|app| dashboard::AppView {
            masked: dashboard::mask_token(&app.token),
            live: app.is_live(),
            created_at: dashboard::human_age(now, app.created_at),
            revoked_at: app.revoked_at.map(|at| dashboard::human_age(now, at)),
            name: app.name,
            token: app.token,
        })
        .collect();
    let page = dashboard::render_apps(&views, None)
        .map_err(|error| ApiError::from(EngineError::Internal(error)))?;
    Ok(Html(page))
}

/// `POST /apps/create` — register an app and generate its token.
pub async fn apps_create(
    State(state): State<AppState>,
    body: axum::body::Bytes,
) -> Result<Response, ApiError> {
    let Some(key) = state.auth.key().cloned() else {
        return Err(ApiError::from(EngineError::Unprotected));
    };
    let name = FormFields::parse(&body).get("name").unwrap_or_default();

    let engine = state.engine.clone();
    let name_for_engine = name.clone();
    let created = spawn_engine(move || engine.register_app(&name_for_engine, &key)).await;

    match created {
        Ok(app) => {
            tracing::info!(app = %app.name, "an app was registered");
            // Anchor on the new row so a long list does not hide what just
            // happened.
            Ok(Redirect::to(&format!("/apps#app-{}", app.name)).into_response())
        }
        Err(error) => Err(error),
    }
}

/// `POST /apps/revoke` — turn an app off, keeping the record that it existed.
pub async fn apps_revoke(
    State(state): State<AppState>,
    body: axum::body::Bytes,
) -> Result<Response, ApiError> {
    if !state.auth.is_protected() {
        return Err(ApiError::from(EngineError::Unprotected));
    }
    let name = FormFields::parse(&body).get("name").unwrap_or_default();

    let engine = state.engine.clone();
    let name_for_engine = name.clone();
    spawn_engine(move || engine.revoke_app(&name_for_engine)).await?;
    tracing::info!(app = %name, "an app was revoked");
    Ok(Redirect::to("/apps").into_response())
}

/// A urlencoded form body, decoded once.
///
/// The existing handlers each picked their one field out of the raw string;
/// three fields across two forms is where that stops being cheaper than
/// parsing properly.
struct FormFields(Vec<(String, String)>);

impl FormFields {
    fn parse(body: &[u8]) -> Self {
        let raw = String::from_utf8_lossy(body);
        Self(
            raw.split('&')
                .filter(|pair| !pair.is_empty())
                .map(|pair| {
                    let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
                    (urldecode(name), urldecode(value))
                })
                .collect(),
        )
    }

    fn get(&self, name: &str) -> Option<String> {
        self.0
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
    }
}
