//! K10 · the Topics page: every topic on the hub with its status.

use axum::Extension;
use axum::extract::State;
use axum::response::{Html, Redirect};
use chassis::Dashboard;
use serde_json::json;

use crate::dashboard::SubscriptionView;
use crate::store::queries;

use crate::http::AppState;
use crate::http::error::ApiError;
use crate::http::handlers::assets::ASSET_VERSION;
use crate::http::handlers::dashboard::page::{TOPICS_HTML, internal, page};

/// A topic is "Backlog" on the Topics page from this many waiting messages.
const BACKLOG_STATUS_AT: i64 = 50;

/// A topic is "Quiet" on the Topics page after this long without a publish.
const QUIET_AFTER_MS: i64 = 24 * 3_600_000;

/// The longest dead-letter excerpt the Topics page keeps for its hover card.
const DEAD_NOTE_CHARS: usize = 80;

/// One row of the Topics page as the page's script reads it: the counts the
/// flow bar and the key figures are drawn from, the subscriptions behind the
/// "who reads this" card and the attention band, and the status word the
/// table sorts and filters by. Times are Unix milliseconds.
#[derive(serde::Serialize)]
struct TopicRowView {
    name: String,
    status: &'static str,
    messages: i64,
    subscriptions: Vec<String>,
    flagged: Vec<String>,
    backlog: i64,
    in_flight: i64,
    dead: i64,
    lapsed: i64,
    last_published_ms: Option<i64>,
    dead_note: Option<String>,
}

/// The status word of a topic: the worst thing true of it.
fn topic_status(dead: i64, backlog: i64, last_published_ms: Option<i64>, now: i64) -> &'static str {
    if dead > 0 {
        "Dead letters"
    } else if backlog >= BACKLOG_STATUS_AT {
        "Backlog"
    } else if last_published_ms.is_some_and(|at| now - at > QUIET_AFTER_MS) {
        "Quiet"
    } else {
        "Healthy"
    }
}

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

/// What every page's command palette and the Topics page's uses-map are
/// built from: every topic and every subscription, plus the hub's dead
/// letter total (what "Prune dead letters" would delete).
pub(crate) async fn shell_view(state: &AppState) -> Result<serde_json::Value, ApiError> {
    let engine = state.engine.clone();
    let built = tokio::task::spawn_blocking(move || {
        engine.store().read(|conn| {
            let now = engine.now_ms();
            let mut topics = Vec::new();
            let mut subscriptions = Vec::new();
            let mut dead_total = 0;
            for summary in queries::topic_summaries(conn)? {
                dead_total += summary.dead;
                let detail = if summary.dead > 0 {
                    format!(
                        "{}, {}",
                        counted(summary.messages, "message"),
                        counted(summary.dead, "dead letter")
                    )
                } else {
                    counted(summary.messages, "message")
                };
                topics.push(ShellTopic {
                    name: summary.name.clone(),
                    messages: summary.messages,
                    detail,
                });
                let Some(topic_id) = queries::topic_id_by_name_conn(conn, &summary.name)? else {
                    continue;
                };
                for sub in queries::subscription_summaries(conn, topic_id)? {
                    let view = SubscriptionView::at(sub, now);
                    let backlog = view.backlog;
                    let (tone, detail) = match view.state.as_str() {
                        "active" => ("ok", "active".to_string()),
                        "archived" => ("muted", "archived".to_string()),
                        other if view.dead > 0 => (
                            "bad",
                            format!("{other}: {}", counted(view.dead, "dead letter")),
                        ),
                        other => ("warn", format!("{other}: lease expired")),
                    };
                    subscriptions.push(ShellSubscription {
                        topic: summary.name.clone(),
                        name: view.name,
                        state: view.state,
                        tone,
                        detail,
                        backlog,
                    });
                }
            }
            Ok(json!({
                "topics": topics,
                "subscriptions": subscriptions,
                "dead_total": dead_total,
            }))
        })
    })
    .await
    .map_err(|error| internal(format!("the dashboard task failed: {error}")))?
    .map_err(|error: anyhow::Error| internal(format!("{error:#}")))?;
    Ok(built)
}

/// K10 · `GET /topics` — every topic on this hub. Until step 2 this was `/`;
/// the kit's status page owns `/` now and carries a Topics section.
///
/// The page's script reads the rows from one JSON block (`topics_json`), so
/// the automatic refresh, which fetches this same URL and swaps one region,
/// carries fresh numbers with it.
pub async fn topics_page(
    Extension(dash): Extension<Dashboard>,
    State(state): State<AppState>,
) -> Result<Html<String>, ApiError> {
    let engine = state.engine.clone();
    let built = tokio::task::spawn_blocking(move || -> anyhow::Result<(i64, Vec<TopicRowView>)> {
        let now = engine.now_ms();
        let rows = engine.store().read(|conn| {
            let mut rows = Vec::new();
            for summary in queries::topic_summaries(conn)? {
                let topic_id = queries::topic_id_by_name_conn(conn, &summary.name)?;
                let subscriptions = match topic_id {
                    Some(id) => queries::subscription_summaries(conn, id)?,
                    None => Vec::new(),
                };
                let dead_note = match topic_id {
                    Some(id) if summary.dead > 0 => queries::dead_letters_for_topic(conn, id, 1)?
                        .into_iter()
                        .next()
                        .map(|dead| {
                            let text = String::from_utf8_lossy(&dead.letter.payload);
                            let excerpt: String = text.chars().take(DEAD_NOTE_CHARS).collect();
                            format!("{}: {excerpt}", dead.subscription)
                        }),
                    _ => None,
                };
                rows.push(TopicRowView {
                    status: topic_status(
                        summary.dead,
                        summary.backlog,
                        summary.last_published_at,
                        now,
                    ),
                    name: summary.name,
                    messages: summary.messages,
                    flagged: subscriptions
                        .iter()
                        .filter(|sub| sub.state == "flagged")
                        .map(|sub| sub.name.clone())
                        .collect(),
                    in_flight: subscriptions.iter().map(|sub| sub.claimed).sum(),
                    lapsed: subscriptions.iter().map(|sub| sub.lapsed).sum(),
                    subscriptions: subscriptions.into_iter().map(|sub| sub.name).collect(),
                    backlog: summary.backlog,
                    dead: summary.dead,
                    last_published_ms: summary.last_published_at,
                    dead_note,
                });
            }
            Ok(rows)
        })?;
        Ok((now, rows))
    })
    .await
    .map_err(|error| internal(format!("the dashboard task failed: {error}")))?
    .map_err(|error| internal(format!("{error:#}")))?;
    let (now, topics) = built;
    // The block sits inside a <script type="application/json">: `<` is
    // escaped so a payload excerpt can never close the element.
    let topics_json = serde_json::to_string(&json!({ "now_ms": now, "topics": topics }))
        .map_err(|error| internal(format!("cannot encode the topics: {error}")))?
        .replace('<', "\\u003c");
    page(
        &dash,
        "/topics",
        TOPICS_HTML,
        json!({
            "topics": topics,
            "topics_json": topics_json,
            "shell": shell_view(&state).await?,
            "kyu_assets": ASSET_VERSION.as_str(),
        }),
    )
}

/// K2-3 · `GET /apps` — the 2.x name of the clients page; the kit's
/// `/clients` (labelled "Apps") took over, and old links keep working.
pub async fn apps_redirect() -> Redirect {
    Redirect::to("/clients")
}
