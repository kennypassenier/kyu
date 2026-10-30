//! The page shell every dashboard page shares: the topic and subscription
//! lists behind the command palette and the "who reads what" map.

use serde_json::json;

use crate::dashboard::SubscriptionView;
use crate::store::queries;

use crate::http::AppState;
use crate::http::error::ApiError;
use crate::http::handlers::dashboard::page::internal;

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

/// The "Dead letters" tail of a topic's palette line, only when there are any.
fn topic_detail(messages: i64, dead: i64) -> String {
    if dead > 0 {
        format!(
            "{}, {}",
            counted(messages, "message"),
            counted(dead, "dead letter")
        )
    } else {
        counted(messages, "message")
    }
}

/// The map's tone word and detail line for a subscription's state.
fn tone_and_detail(view: &SubscriptionView) -> (&'static str, String) {
    match view.state.as_str() {
        "active" => ("ok", "active".to_string()),
        "archived" => ("muted", "archived".to_string()),
        other if view.dead > 0 => (
            "bad",
            format!("{other}: {}", counted(view.dead, "dead letter")),
        ),
        other => ("warn", format!("{other}: lease expired")),
    }
}

/// Reads every topic and subscription in one store read.
fn read_shell(conn: &rusqlite::Connection, now: i64) -> anyhow::Result<serde_json::Value> {
    let mut topics = Vec::new();
    let mut subscriptions = Vec::new();
    let mut dead_total = 0;
    for summary in queries::topic_summaries(conn)? {
        dead_total += summary.dead;
        topics.push(ShellTopic {
            name: summary.name.clone(),
            messages: summary.messages,
            detail: topic_detail(summary.messages, summary.dead),
        });
        let Some(topic_id) = queries::topic_id_by_name_conn(conn, &summary.name)? else {
            continue;
        };
        for sub in queries::subscription_summaries(conn, topic_id)? {
            let view = SubscriptionView::at(sub, now);
            let (tone, detail) = tone_and_detail(&view);
            subscriptions.push(ShellSubscription {
                topic: summary.name.clone(),
                backlog: view.backlog,
                name: view.name,
                state: view.state,
                tone,
                detail,
            });
        }
    }
    Ok(json!({
        "topics": topics,
        "subscriptions": subscriptions,
        "dead_total": dead_total,
    }))
}

/// What every page's command palette and the Topics page's uses-map are
/// built from: every topic and every subscription, plus the hub's dead
/// letter total (what "Prune dead letters" would delete).
pub(crate) async fn shell_view(state: &AppState) -> Result<serde_json::Value, ApiError> {
    let engine = state.engine.clone();
    tokio::task::spawn_blocking(move || {
        engine
            .store()
            .read(|conn| read_shell(conn, engine.now_ms()))
    })
    .await
    .map_err(|error| internal(format!("the dashboard task failed: {error}")))?
    .map_err(|error: anyhow::Error| internal(format!("{error:#}")))
}
