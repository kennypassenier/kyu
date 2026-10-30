//! K10 · one topic's dashboard page.

use axum::Extension;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Html;
use chassis::Dashboard;
use serde_json::json;

use crate::dashboard::{self, DeadLetterView, MessageView, SubscriptionView, TopicView};
use crate::store::queries;

use crate::http::AppState;
use crate::http::error::ApiError;
use crate::http::handlers::assets::ASSET_VERSION;
use crate::http::handlers::dashboard::page::{TOPIC_HTML, page, read_blocking};
use crate::http::handlers::dashboard::shell::shell_view;
use crate::http::handlers::dashboard::topic_data::{
    Stamps, build_snippets, first_live_reader, page_data,
};

/// How many recent messages a topic page shows.
const RECENT_MESSAGES: usize = 20;

/// How many dead letters a topic page shows (K6). Bounded because a broken
/// consumer can produce a great many, and a page that will not load is no
/// use on the night you need it.
const DEAD_LETTERS_SHOWN: usize = 50;

/// What the topic page renders: the topic, its subscriptions, recent
/// messages, dead letters and the copy-paste commands.
struct TopicPage {
    topic: TopicView,
    subscriptions: Vec<SubscriptionView>,
    messages: Vec<MessageView>,
    dead_letters: Vec<DeadLetterView>,
    snippets: dashboard::Snippets,
    /// Everything the page script draws from: the views above plus the raw
    /// Unix-millisecond stamps the views turn into words, and the commands.
    data: serde_json::Value,
}

/// Turns raw rows into their views plus the raw stamps the views hide.
fn stamped<R, V>(
    raw: Vec<R>,
    stamp: impl Fn(&R) -> serde_json::Value,
    view: impl Fn(R) -> V,
) -> (Vec<V>, Vec<serde_json::Value>) {
    let stamps = raw.iter().map(stamp).collect();
    (raw.into_iter().map(view).collect(), stamps)
}

/// The stamped views and counts the topic page is drawn from, in one read.
fn load_topic_page(
    conn: &rusqlite::Connection,
    topic_name: &str,
    now: i64,
    token: Option<&str>,
) -> anyhow::Result<Option<TopicPage>> {
    let Some(topic_id) = queries::topic_id_by_name_conn(conn, topic_name)? else {
        return Ok(None);
    };
    let summary = queries::topic_summaries(conn)?
        .into_iter()
        .find(|summary| summary.name == topic_name);
    let Some(summary) = summary else {
        return Ok(None);
    };
    let last_published_ms = summary.last_published_at;
    let (subscriptions, subscription_stamps) = stamped(
        queries::subscription_summaries(conn, topic_id)?,
        |s| json!({ "oldest_unacked_ms": s.oldest_unacked_at, "last_poll_ms": s.last_poll_at }),
        |summary| SubscriptionView::at(summary, now),
    );
    let (messages, message_stamps) = stamped(
        queries::recent_messages(conn, topic_id, RECENT_MESSAGES)?,
        |m| json!({ "published_ms": m.published_at, "due_ms": m.due_at }),
        |message| MessageView::at(message, now),
    );
    let (dead_letters, dead_stamps) = stamped(
        queries::dead_letters_for_topic(conn, topic_id, DEAD_LETTERS_SHOWN)?,
        |d| json!({ "published_ms": d.letter.published_at, "dead_ms": d.letter.dead_at }),
        |dead| DeadLetterView::at(dead, now),
    );
    let reader = first_live_reader(&subscriptions);
    let snippets = build_snippets(topic_name, reader, messages.first(), token);
    let topic_view = TopicView::at(summary, now);
    let stamps = Stamps {
        last_published_ms,
        subscriptions: &subscription_stamps,
        messages: &message_stamps,
        dead_letters: &dead_stamps,
    };
    let data = page_data(
        now,
        &topic_view,
        (&subscriptions, &messages, &dead_letters),
        &stamps,
        (&snippets, reader.is_some()),
        token,
    );
    Ok(Some(TopicPage {
        topic: topic_view,
        subscriptions,
        messages,
        dead_letters,
        snippets,
        data,
    }))
}

/// K10 · `GET /t/{topic}/dashboard` — one topic in detail.
pub async fn dashboard_topic(
    Extension(dash): Extension<Dashboard>,
    State(state): State<AppState>,
    Path(topic): Path<String>,
) -> Result<Html<String>, ApiError> {
    let engine = state.engine.clone();
    let topic_name = topic.clone();
    // The copy-paste commands carry the login token, which always exists
    // (the kit refuses to start without it); a command with a client's own
    // token comes from the kit's clients page ("Copy command").
    let token = state.auth.token().map(str::to_string);
    let built = read_blocking(engine, move |conn, now| {
        load_topic_page(conn, &topic_name, now, token.as_deref())
    })
    .await?;
    let Some(built) = built else {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("topic {topic:?} does not exist"),
            "a topic starts existing when something publishes to it. Open /topics to \
             see which topics this hub has.",
        ));
    };
    page(
        &dash,
        "/topics",
        TOPIC_HTML,
        json!({
            "topic": built.topic,
            "subscriptions": built.subscriptions,
            "messages": built.messages,
            "dead_letters": built.dead_letters,
            "snippets": { "bootstrap_note": built.snippets.bootstrap_note },
            "data": built.data,
            "reveal_seconds": dash.reveal_seconds,
            "shell": shell_view(&state).await?,
            "kyu_assets": ASSET_VERSION.as_str(),
        }),
    )
}
