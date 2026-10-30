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
use crate::http::handlers::dashboard::page::{SUBSCRIPTION_HTML, internal, page};
use crate::http::handlers::dashboard::topics::shell_view;

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
