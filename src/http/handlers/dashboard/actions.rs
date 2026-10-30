//! W9 · dashboard form actions: test publish, requeue, delete, archive and
//! delete a subscription, prune dead letters.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use serde_json::json;

use crate::http::AppState;
use crate::http::error::ApiError;
use crate::http::handlers::dashboard::form::{done, urldecode, wants_json};
use crate::http::handlers::engine_call::spawn_engine_write;

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
    // The optional "Deliver after (seconds)" field: the message is stored now
    // and becomes deliverable that many seconds later (W4, at most a day).
    let delay_seconds = form
        .split('&')
        .find_map(|pair| pair.strip_prefix("delay="))
        .map(urldecode)
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty());
    let due_at = match delay_seconds {
        None => None,
        Some(text) => match text.parse::<i64>() {
            Ok(seconds) if (0..=86_400).contains(&seconds) => {
                (seconds > 0).then(|| state.engine.now_ms().saturating_add(seconds * 1000))
            }
            _ => {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    format!("the delay {text:?} is not between 0 and 86400 seconds"),
                    "type a whole number of seconds from 0 to 86400, or leave it empty to \
                     deliver at once.",
                ));
            }
        },
    };

    let engine = state.engine.clone();
    let topic_for_engine = topic.clone();
    let published = spawn_engine_write(move || {
        engine.publish_due(
            &topic_for_engine,
            payload.as_bytes(),
            Some("application/json"),
            due_at,
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
