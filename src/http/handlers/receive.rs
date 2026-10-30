//! K2 · receiving the next message, including the long-poll wait.

use std::time::Duration;

use axum::Extension;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chassis::Caller;
use serde::Deserialize;
use tokio::time::Instant;

use crate::engine::{EngineError, Received, names};

use crate::http::AppState;
use crate::http::error::ApiError;
use crate::http::handlers::engine_call::spawn_engine_write;
use crate::http::handlers::wire::{HEADER_NOTICE, insert_str, notice, render};

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
