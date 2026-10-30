//! K9 · a topic's message retention window.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;

use crate::http::AppState;
use crate::http::error::ApiError;
use crate::http::handlers::engine_call::{spawn_engine, spawn_engine_write};

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
