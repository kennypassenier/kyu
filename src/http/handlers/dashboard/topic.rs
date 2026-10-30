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
use crate::http::handlers::dashboard::page::{TOPIC_HTML, internal, page};
use crate::http::handlers::dashboard::topics::shell_view;

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

/// The base URL the page's commands are built with; the page script swaps it
/// for the address the browser used, which is the only one that is certainly
/// reachable from where the command is pasted.
const COMMAND_BASE: &str = "@@kyu-base@@";

/// A view as JSON with extra fields added (the raw stamps the view hides).
fn with_extra<T: serde::Serialize>(view: &T, extra: serde_json::Value) -> serde_json::Value {
    let mut value = serde_json::to_value(view).unwrap_or(serde_json::Value::Null);
    if let (Some(object), Some(more)) = (value.as_object_mut(), extra.as_object()) {
        for (key, item) in more {
            object.insert(key.clone(), item.clone());
        }
    }
    value
}

/// The shown form of a command: the working one with the token swapped for
/// `$KYU_TOKEN` (double-quoted, so a shell expands it).
fn shown_command(working: &str, token: Option<&str>) -> String {
    match token {
        Some(token) if !token.is_empty() => working.replace(
            &format!("-H 'authorization: Bearer {token}'"),
            "-H \"authorization: Bearer $KYU_TOKEN\"",
        ),
        _ => working.to_string(),
    }
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
    let built = tokio::task::spawn_blocking(move || -> anyhow::Result<Option<TopicPage>> {
        engine.store().read(|conn| {
            let Some(topic_id) = queries::topic_id_by_name_conn(conn, &topic_name)? else {
                return Ok(None);
            };
            let summary = queries::topic_summaries(conn)?
                .into_iter()
                .find(|summary| summary.name == topic_name);
            let Some(summary) = summary else {
                return Ok(None);
            };
            let now = engine.now_ms();
            let last_published_ms = summary.last_published_at;
            let raw_subscriptions = queries::subscription_summaries(conn, topic_id)?;
            let subscription_stamps: Vec<serde_json::Value> = raw_subscriptions
                .iter()
                .map(|s| json!({ "oldest_unacked_ms": s.oldest_unacked_at, "last_poll_ms": s.last_poll_at }))
                .collect();
            let subscriptions: Vec<SubscriptionView> = raw_subscriptions
                .into_iter()
                .map(|summary| SubscriptionView::at(summary, now))
                .collect();
            let raw_messages = queries::recent_messages(conn, topic_id, RECENT_MESSAGES)?;
            let message_stamps: Vec<serde_json::Value> = raw_messages
                .iter()
                .map(|m| json!({ "published_ms": m.published_at, "due_ms": m.due_at }))
                .collect();
            let messages: Vec<MessageView> = raw_messages
                .into_iter()
                .map(|message| MessageView::at(message, now))
                .collect();
            let raw_dead = queries::dead_letters_for_topic(conn, topic_id, DEAD_LETTERS_SHOWN)?;
            let dead_stamps: Vec<serde_json::Value> = raw_dead
                .iter()
                .map(|d| json!({ "published_ms": d.letter.published_at, "dead_ms": d.letter.dead_at }))
                .collect();
            let dead_letters: Vec<DeadLetterView> = raw_dead
                .into_iter()
                .map(|dead| DeadLetterView::at(dead, now))
                .collect();
            // The snippets carry this topic's own most recent payload and a
            // subscription that genuinely exists, so what you copy is what
            // your hub actually answers to (S1).
            let example = messages.first();
            let reader = subscriptions
                .iter()
                .find(|subscription| subscription.state != "archived")
                .map(|subscription| subscription.name.as_str());
            let snippets = dashboard::Snippets::build(
                COMMAND_BASE,
                &topic_name,
                reader,
                example.map(|message| &message.payload),
                example.and_then(|message| message.content_type.as_deref()),
                token.as_deref(),
                None,
            );
            let topic_view = TopicView::at(summary, now);
            let data = json!({
                "now_ms": now,
                "base": COMMAND_BASE,
                "topic": with_extra(&topic_view, json!({ "last_published_ms": last_published_ms })),
                "subscriptions": subscriptions.iter().zip(&subscription_stamps)
                    .map(|(v, e)| with_extra(v, e.clone())).collect::<Vec<_>>(),
                "messages": messages.iter().zip(&message_stamps)
                    .map(|(v, e)| with_extra(v, e.clone())).collect::<Vec<_>>(),
                "dead_letters": dead_letters.iter().zip(&dead_stamps)
                    .map(|(v, e)| with_extra(v, e.clone())).collect::<Vec<_>>(),
                "has_reader": reader.is_some(),
                "commands": {
                    "shown": {
                        "publish": shown_command(&snippets.publish.full, token.as_deref()),
                        "receive_raw": shown_command(&snippets.receive_raw.full, token.as_deref()),
                        "receive_envelope": shown_command(&snippets.receive_envelope.full, token.as_deref()),
                        "ack": shown_command(&snippets.ack.full, token.as_deref()),
                    },
                    "working": {
                        "publish": snippets.publish.full,
                        "receive_raw": snippets.receive_raw.full,
                        "receive_envelope": snippets.receive_envelope.full,
                        "ack": snippets.ack.full,
                    },
                    "has_token": token.as_deref().is_some_and(|t| !t.is_empty()),
                },
            });
            Ok(Some(TopicPage {
                topic: topic_view,
                subscriptions,
                messages,
                dead_letters,
                snippets,
                data,
            }))
        })
    })
    .await
    .map_err(|error| internal(format!("the dashboard task failed: {error}")))?
    .map_err(|error| internal(format!("{error:#}")))?;
    match built {
        Some(built) => page(
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
        ),
        None => Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("topic {topic:?} does not exist"),
            "a topic starts existing when something publishes to it. Open /topics to \
             see which topics this hub has.",
        )),
    }
}
