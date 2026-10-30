//! W16 · one subscription's page: live backlog and policy rows.

use axum::Extension;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Html;
use chassis::Dashboard;
use serde_json::json;

use crate::dashboard::{self, SubscriptionView};
use crate::store::queries;

use crate::http::AppState;
use crate::http::error::ApiError;
use crate::http::handlers::assets::ASSET_VERSION;
use crate::http::handlers::dashboard::page::{SUBSCRIPTION_HTML, internal, page, to_json};
use crate::http::handlers::dashboard::shell::shell_view;
use crate::http::handlers::dashboard::subscription_rows::{
    format_duration_ms, subscription_policy_rows,
};

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

/// Reads one subscription's page rows in a single store read; `None` when
/// the topic or the subscription does not exist.
fn load_rows(
    conn: &rusqlite::Connection,
    topic_name: &str,
    subscription_name: &str,
    now: i64,
) -> anyhow::Result<Option<SubscriptionPageRows>> {
    let Some(topic_id) = queries::topic_id_by_name_conn(conn, topic_name)? else {
        return Ok(None);
    };
    let Some(sub_id) = queries::subscription_id_by_name_conn(conn, topic_id, subscription_name)?
    else {
        return Ok(None);
    };
    let summary = queries::subscription_summaries(conn, topic_id)?
        .into_iter()
        .find(|summary| summary.name == subscription_name);
    let Some(summary) = summary else {
        return Ok(None);
    };
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
}

/// Backlog items as JSON rows, with the raw publish stamp the script needs.
fn backlog_rows(items: Vec<queries::BacklogItem>, now: i64) -> Vec<serde_json::Value> {
    items
        .into_iter()
        .map(|item| {
            let published_ms = item.published_at;
            let view = dashboard::BacklogItemView::at(item, now);
            let mut row = to_json(&view);
            row["published_ms"] = json!(published_ms);
            row
        })
        .collect()
}

/// What the page's script reads: raw milliseconds and counts, so it formats
/// every time itself (dd/mm/yyyy HH:mm, Europe/Brussels). Escaped for
/// embedding in a `<script>` block.
fn script_data(
    topic: &str,
    rows: &[serde_json::Value],
    page: &SubscriptionPageRows,
    now: i64,
) -> String {
    let view = &page.view;
    json!({
        "topic": topic,
        "subscription": view.name,
        "state": view.state,
        "now_ms": now,
        "last_poll_ms": page.last_poll_ms,
        "oldest_unacked_ms": page.oldest_ms,
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
    })
    .to_string()
    .replace('<', "\\u003c")
    .replace('>', "\\u003e")
    .replace('&', "\\u0026")
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
            engine
                .store()
                .read(|conn| load_rows(conn, &topic_name, &subscription_name, engine.now_ms()))
        })
        .await
        .map_err(|error| internal(format!("the dashboard task failed: {error}")))?
        .map_err(|error| internal(format!("{error:#}")))?;
    let Some(mut built) = built else {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("subscription {subscription:?} on topic {topic:?} does not exist"),
            "a subscription is created by polling, so this name has never polled this \
             topic. Check the spelling, or open the topic's own page to see which \
             subscriptions it has.",
        ));
    };
    let now = state.engine.now_ms();
    let policy_rows = subscription_policy_rows(&built.view, built.created_at);
    let lease_words = format_duration_ms(built.view.lease_ms);
    let rows = backlog_rows(std::mem::take(&mut built.backlog), now);
    let script = script_data(&topic, &rows, &built, now);
    page(
        &dash,
        "/topics",
        SUBSCRIPTION_HTML,
        json!({
            "topic_name": topic,
            "subscription": built.view,
            "backlog": rows,
            "backlog_total": rows.len(),
            "policy_rows": policy_rows,
            "lease_words": lease_words,
            "page_data": script,
            "shell": shell_view(&state).await?,
            "kyu_assets": ASSET_VERSION.as_str(),
        }),
    )
}
