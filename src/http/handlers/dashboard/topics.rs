//! K10 · the Topics page: every topic on the hub with its status.

use axum::Extension;
use axum::extract::State;
use axum::response::{Html, Redirect};
use chassis::Dashboard;
use serde_json::json;

use crate::store::queries;

use crate::http::AppState;
use crate::http::error::ApiError;
use crate::http::handlers::assets::ASSET_VERSION;
use crate::http::handlers::dashboard::page::{TOPICS_HTML, internal, page};
use crate::http::handlers::dashboard::shell::shell_view;

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

/// The newest dead letter of a topic as a one-line excerpt for the hover card.
fn dead_note(conn: &rusqlite::Connection, topic_id: i64) -> anyhow::Result<Option<String>> {
    Ok(queries::dead_letters_for_topic(conn, topic_id, 1)?
        .into_iter()
        .next()
        .map(|dead| {
            let text = String::from_utf8_lossy(&dead.letter.payload);
            let excerpt: String = text.chars().take(DEAD_NOTE_CHARS).collect();
            format!("{}: {excerpt}", dead.subscription)
        }))
}

/// One topic's row: counts, the subscriptions behind it and its status word.
fn topic_row(
    conn: &rusqlite::Connection,
    summary: queries::TopicSummary,
    now: i64,
) -> anyhow::Result<TopicRowView> {
    let topic_id = queries::topic_id_by_name_conn(conn, &summary.name)?;
    let subscriptions = match topic_id {
        Some(id) => queries::subscription_summaries(conn, id)?,
        None => Vec::new(),
    };
    let dead_note = match topic_id {
        Some(id) if summary.dead > 0 => dead_note(conn, id)?,
        _ => None,
    };
    Ok(TopicRowView {
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
    })
}

/// Every topic's row, read in one store read.
fn read_rows(conn: &rusqlite::Connection, now: i64) -> anyhow::Result<Vec<TopicRowView>> {
    queries::topic_summaries(conn)?
        .into_iter()
        .map(|summary| topic_row(conn, summary, now))
        .collect()
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
        let rows = engine.store().read(|conn| read_rows(conn, now))?;
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
