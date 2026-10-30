//! JSON the subscription page fetches: delivery attempts and daily counts.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;

use crate::engine::EngineError;
use crate::store::queries;

use crate::http::AppState;
use crate::http::error::ApiError;
use crate::http::handlers::engine_call::spawn_engine;

pub(crate) fn unknown_subscription(topic: &str, subscription: &str) -> ApiError {
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
