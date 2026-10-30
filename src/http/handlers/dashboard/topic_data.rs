//! Builds the JSON the topic page's script draws from: stamped views, the
//! copy-paste commands (token masked as shown, real as copied) and snippets.

use serde_json::json;

use crate::http::handlers::dashboard::page::to_json;

use crate::dashboard::{self, DeadLetterView, MessageView, SubscriptionView, TopicView};

/// The base URL the page's commands are built with; the page script swaps it
/// for the address the browser used, which is the only one that is certainly
/// reachable from where the command is pasted.
pub(super) const COMMAND_BASE: &str = "@@kyu-base@@";

/// A view as JSON with extra fields added (the raw stamps the view hides).
pub(super) fn with_extra<T: serde::Serialize>(
    view: &T,
    extra: serde_json::Value,
) -> serde_json::Value {
    let mut value = to_json(view);
    if let (Some(object), Some(more)) = (value.as_object_mut(), extra.as_object()) {
        for (key, item) in more {
            object.insert(key.clone(), item.clone());
        }
    }
    value
}

/// The shown form of a command: the working one with the token swapped for
/// `$KYU_TOKEN` (double-quoted, so a shell expands it).
pub(super) fn shown_command(working: &str, token: Option<&str>) -> String {
    match token {
        Some(token) if !token.is_empty() => working.replace(
            &format!("-H 'authorization: Bearer {token}'"),
            "-H \"authorization: Bearer $KYU_TOKEN\"",
        ),
        _ => working.to_string(),
    }
}

/// The raw Unix-millisecond stamps the views turn into words.
pub(super) struct Stamps<'a> {
    pub(super) last_published_ms: Option<i64>,
    pub(super) subscriptions: &'a [serde_json::Value],
    pub(super) messages: &'a [serde_json::Value],
    pub(super) dead_letters: &'a [serde_json::Value],
}

/// The first subscription that is not archived: the one the snippets read as.
pub(super) fn first_live_reader(subscriptions: &[SubscriptionView]) -> Option<&str> {
    subscriptions
        .iter()
        .find(|subscription| subscription.state != "archived")
        .map(|subscription| subscription.name.as_str())
}

/// The snippets carry this topic's own most recent payload and a
/// subscription that genuinely exists, so what you copy is what your hub
/// actually answers to (S1).
pub(super) fn build_snippets(
    topic_name: &str,
    reader: Option<&str>,
    example: Option<&MessageView>,
    token: Option<&str>,
) -> dashboard::Snippets {
    dashboard::Snippets::build(
        COMMAND_BASE,
        topic_name,
        reader,
        example.map(|message| &message.payload),
        example.and_then(|message| message.content_type.as_deref()),
        token,
        None,
    )
}

/// Zips each view with its raw stamps into the JSON the page script reads.
pub(super) fn with_stamps<T: serde::Serialize>(
    views: &[T],
    stamps: &[serde_json::Value],
) -> Vec<serde_json::Value> {
    views
        .iter()
        .zip(stamps)
        .map(|(view, extra)| with_extra(view, extra.clone()))
        .collect()
}

/// The commands as the page shows them (token masked) and as they work.
pub(super) fn commands_json(
    snippets: &dashboard::Snippets,
    token: Option<&str>,
) -> serde_json::Value {
    json!({
        "shown": {
            "publish": shown_command(&snippets.publish.full, token),
            "receive_raw": shown_command(&snippets.receive_raw.full, token),
            "receive_envelope": shown_command(&snippets.receive_envelope.full, token),
            "ack": shown_command(&snippets.ack.full, token),
        },
        "working": {
            "publish": snippets.publish.full,
            "receive_raw": snippets.receive_raw.full,
            "receive_envelope": snippets.receive_envelope.full,
            "ack": snippets.ack.full,
        },
        "has_token": token.is_some_and(|t| !t.is_empty()),
    })
}

/// Everything the page script draws from.
pub(super) fn page_data(
    now: i64,
    topic_view: &TopicView,
    views: (&[SubscriptionView], &[MessageView], &[DeadLetterView]),
    stamps: &Stamps<'_>,
    (snippets, has_reader): (&dashboard::Snippets, bool),
    token: Option<&str>,
) -> serde_json::Value {
    let (subscriptions, messages, dead_letters) = views;
    json!({
        "now_ms": now,
        "base": COMMAND_BASE,
        "topic": with_extra(topic_view, json!({ "last_published_ms": stamps.last_published_ms })),
        "subscriptions": with_stamps(subscriptions, stamps.subscriptions),
        "messages": with_stamps(messages, stamps.messages),
        "dead_letters": with_stamps(dead_letters, stamps.dead_letters),
        "has_reader": has_reader,
        "commands": commands_json(snippets, token),
    })
}
