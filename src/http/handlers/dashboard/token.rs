//! `POST /t/{topic}/dashboard/token`: the login token and a working command, handed to the
//! page only at the moment of Reveal or Copy, so the page itself never carries them.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::json;

use crate::dashboard::Snippets;
use crate::http::AppState;
use crate::http::error::ApiError;
use crate::http::handlers::dashboard::page::read_blocking;
use crate::http::handlers::dashboard::topic::load_topic_page;

#[derive(Debug, Deserialize)]
pub struct StepQuery {
    /// Which command to return with the token: publish, receive_raw, receive_envelope or ack.
    step: Option<String>,
}

/// The working command of a step, or None for an unknown step.
fn working<'a>(snippets: &'a Snippets, step: &str) -> Option<&'a str> {
    match step {
        "publish" => Some(&snippets.publish.full),
        "receive_raw" => Some(&snippets.receive_raw.full),
        "receive_envelope" => Some(&snippets.receive_envelope.full),
        "ack" => Some(&snippets.ack.full),
        _ => None,
    }
}

/// The token, and the working command for `?step=` when one is asked for.
pub async fn dashboard_token(
    State(state): State<AppState>,
    Path(topic): Path<String>,
    Query(query): Query<StepQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let token = state.auth.token().map(str::to_string).unwrap_or_default();
    let Some(step) = query.step else {
        return Ok(Json(json!({ "token": token })));
    };
    let (engine, name, shown_token) = (state.engine.clone(), topic.clone(), token.clone());
    let built = read_blocking(engine, move |conn, now| {
        load_topic_page(conn, &name, now, Some(&shown_token))
    })
    .await?;
    let command = built
        .as_ref()
        .and_then(|page| working(&page.snippets, &step));
    let Some(command) = command else {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("topic {topic:?} or step {step:?} is unknown"),
            "ask for publish, receive_raw, receive_envelope or ack on a topic that exists.",
        ));
    };
    Ok(Json(json!({ "token": token, "command": command })))
}
