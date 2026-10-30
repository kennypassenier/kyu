//! JSON the Topics page fetches: hourly and total statistics, and a topic's
//! dead-letter retention.

use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;

use crate::engine::EngineError;
use crate::store::queries;

use crate::http::AppState;
use crate::http::error::ApiError;
use crate::http::handlers::engine_call::spawn_engine;

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
