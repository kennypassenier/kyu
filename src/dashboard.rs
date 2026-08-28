//! The dashboard (K10): topics, subscriptions, backlogs, recent messages,
//! dead letters — and the copy-paste curl examples rendered from a real
//! recent payload, which is the mechanism behind the five-minute re-entry
//! test (S1).
//!
//! Rendering is minijinja with htmx (T4). Because those templates are
//! checked at runtime rather than at compile time, every one of them is
//! rendered with seeded state in the test suite; that is the compensation
//! this project owes for the choice.
//!
//! Payloads are untrusted input (AR11): autoescape stays on, the display is
//! capped, and the cap is always visible.

use std::sync::LazyLock;

use anyhow::{Context, Result};
use minijinja::Environment;
use serde::Serialize;

use crate::engine::clock::Millis;
use crate::engine::policy::Policy;
use crate::store::queries::{RecentMessage, StoredPolicy, SubscriptionSummary, TopicSummary};

/// How much of a payload the dashboard shows. Enough to recognise a
/// message; the rest is announced rather than dropped in silence (AR11).
pub const PAYLOAD_DISPLAY_LIMIT: usize = 4096;

/// Templates are embedded in the binary, so the image stays a single file
/// with nothing to mount beside it (T9).
static ENVIRONMENT: LazyLock<Environment<'static>> = LazyLock::new(|| {
    let mut environment = Environment::new();
    // Autoescape is the difference between a dashboard and a stored-XSS
    // delivery system: every payload on these pages came from outside.
    environment.set_auto_escape_callback(|_| minijinja::AutoEscape::Html);
    environment
        .add_template("layout.html", include_str!("../templates/layout.html"))
        .expect("the layout template must compile");
    environment
        .add_template("topics.html", include_str!("../templates/topics.html"))
        .expect("the topics template must compile");
    environment
        .add_template("topic.html", include_str!("../templates/topic.html"))
        .expect("the topic template must compile");
    environment
});

/// How a payload is shown: text as text, binary announced as binary, and
/// anything oversized marked as truncated with its real size.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PayloadView {
    pub text: String,
    pub bytes: usize,
    pub truncated: bool,
    pub binary: bool,
}

impl PayloadView {
    pub fn of(payload: &[u8]) -> Self {
        let bytes = payload.len();
        let shown = bytes.min(PAYLOAD_DISPLAY_LIMIT);
        match std::str::from_utf8(&payload[..shown]) {
            Ok(text) => Self {
                text: text.to_string(),
                bytes,
                truncated: bytes > shown,
                binary: false,
            },
            Err(_) => Self {
                text: String::new(),
                bytes,
                truncated: bytes > shown,
                binary: true,
            },
        }
    }

    /// What a human should read next to the payload. Never empty, because a
    /// blank space where bytes used to be is the silence G8 forbids.
    pub fn note(&self) -> Option<String> {
        match (self.binary, self.truncated) {
            (true, _) => Some(format!("binary payload ({} bytes)", self.bytes)),
            (false, true) => Some(format!(
                "showing the first {PAYLOAD_DISPLAY_LIMIT} of {} bytes",
                self.bytes
            )),
            (false, false) => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MessageView {
    pub id: String,
    pub published_at: Millis,
    pub due_at: Option<Millis>,
    pub content_type: Option<String>,
    pub payload: PayloadView,
    pub note: Option<String>,
}

impl From<RecentMessage> for MessageView {
    fn from(message: RecentMessage) -> Self {
        let payload = PayloadView::of(&message.payload);
        let note = payload.note();
        Self {
            id: message.id,
            published_at: message.published_at,
            due_at: message.due_at,
            content_type: message.content_type,
            payload,
            note,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SubscriptionView {
    pub name: String,
    pub state: String,
    pub backlog: i64,
    pub claimed: i64,
    pub dead: i64,
    pub last_poll_at: Option<Millis>,
    pub oldest_unacked_at: Option<Millis>,
    pub lease_ms: i64,
    pub max_attempts: i64,
    pub backoff_ms: i64,
    pub ttl_ms: Option<i64>,
    pub explicit: StoredPolicyView,
}

#[derive(Debug, Clone, Serialize)]
pub struct StoredPolicyView {
    pub lease_ms: Option<i64>,
    pub max_attempts: Option<i64>,
    pub backoff_ms: Option<i64>,
    pub ttl_ms: Option<i64>,
}

impl From<StoredPolicy> for StoredPolicyView {
    fn from(stored: StoredPolicy) -> Self {
        Self {
            lease_ms: stored.lease_ms,
            max_attempts: stored.max_attempts,
            backoff_ms: stored.backoff_ms,
            ttl_ms: stored.ttl_ms,
        }
    }
}

impl From<SubscriptionSummary> for SubscriptionView {
    fn from(summary: SubscriptionSummary) -> Self {
        let effective = Policy::effective(summary.policy);
        Self {
            name: summary.name,
            state: summary.state,
            backlog: summary.backlog,
            claimed: summary.claimed,
            dead: summary.dead,
            last_poll_at: summary.last_poll_at,
            oldest_unacked_at: summary.oldest_unacked_at,
            lease_ms: effective.lease_ms,
            max_attempts: effective.max_attempts,
            backoff_ms: effective.backoff_ms,
            ttl_ms: effective.ttl_ms,
            explicit: summary.policy.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TopicView {
    pub name: String,
    pub retention_ms: Option<i64>,
    pub messages: i64,
    pub subscriptions: i64,
    pub backlog: i64,
    pub dead: i64,
    pub last_published_at: Option<Millis>,
}

impl From<TopicSummary> for TopicView {
    fn from(summary: TopicSummary) -> Self {
        Self {
            name: summary.name,
            retention_ms: summary.retention_ms,
            messages: summary.messages,
            subscriptions: summary.subscriptions,
            backlog: summary.backlog,
            dead: summary.dead,
            last_published_at: summary.last_published_at,
        }
    }
}

/// The copy-paste examples, rendered with this topic's own real payload and
/// a subscription name that actually exists. Generic documentation is what
/// you skim; your own data is what you trust (S1).
#[derive(Debug, Clone, Serialize)]
pub struct Snippets {
    pub publish: String,
    pub receive_raw: String,
    pub receive_envelope: String,
    pub ack: String,
    pub bootstrap_note: Option<String>,
}

impl Snippets {
    pub fn build(
        base_url: &str,
        topic: &str,
        subscription: Option<&str>,
        example_payload: Option<&PayloadView>,
        content_type: Option<&str>,
    ) -> Self {
        let payload = match example_payload {
            Some(view) if !view.binary && !view.text.is_empty() => {
                let single_line: String = view.text.lines().collect::<Vec<_>>().join("");
                single_line.replace('\'', "'\\''")
            }
            _ => r#"{"hello":"world"}"#.to_string(),
        };
        let content_type = content_type.unwrap_or("application/json");
        let name = subscription.unwrap_or("my-consumer");

        Self {
            publish: format!(
                "curl -s -H 'content-type: {content_type}' \\\n     -d '{payload}' \\\n     {base_url}/t/{topic}"
            ),
            receive_raw: format!(
                "curl -s -D- -o message.body \"{base_url}/t/{topic}/next?as={name}\""
            ),
            receive_envelope: format!(
                "curl -s \"{base_url}/t/{topic}/next?as={name}&envelope=json\""
            ),
            ack: format!("curl -s -X POST \"{base_url}/t/{topic}/ack/<id>?as={name}\""),
            bootstrap_note: subscription.is_none().then(|| {
                format!(
                    "no subscription has polled {topic} yet. A subscription starts existing \
                     when it first polls, and it receives what is published after that — so \
                     run the receive command once before publishing the message you want it \
                     to see, or add &from=beginning to pick up what the topic still retains."
                )
            }),
        }
    }
}

pub fn render_topics(topics: Vec<TopicView>, now: Millis) -> Result<String> {
    ENVIRONMENT
        .get_template("topics.html")
        .context("the topics template is missing")?
        .render(minijinja::context! { topics => topics, now => now })
        .context("cannot render the topic list")
}

#[allow(clippy::too_many_arguments)]
pub fn render_topic(
    topic: TopicView,
    subscriptions: Vec<SubscriptionView>,
    messages: Vec<MessageView>,
    snippets: Snippets,
    now: Millis,
) -> Result<String> {
    ENVIRONMENT
        .get_template("topic.html")
        .context("the topic template is missing")?
        .render(minijinja::context! {
            topic => topic,
            subscriptions => subscriptions,
            messages => messages,
            snippets => snippets,
            now => now,
        })
        .context("cannot render the topic page")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l7_a_text_payload_is_shown_as_text() {
        let view = PayloadView::of(br#"{"title":"Backup klaar"}"#);
        assert_eq!(view.text, r#"{"title":"Backup klaar"}"#);
        assert!(!view.binary);
        assert!(!view.truncated);
        assert_eq!(
            view.note(),
            None,
            "nothing to announce about a plain payload"
        );
    }

    #[test]
    fn l7_a_binary_payload_is_announced_rather_than_mangled() {
        let view = PayloadView::of(&[0x00, 0xff, 0x1b, 0x80]);
        assert!(view.binary);
        assert_eq!(view.text, "");
        assert_eq!(view.note().as_deref(), Some("binary payload (4 bytes)"));
    }

    #[test]
    fn l7_an_oversized_payload_says_how_much_is_hidden() {
        let payload = "x".repeat(PAYLOAD_DISPLAY_LIMIT * 2);
        let view = PayloadView::of(payload.as_bytes());

        assert!(view.truncated);
        assert_eq!(view.text.len(), PAYLOAD_DISPLAY_LIMIT);
        assert_eq!(view.bytes, PAYLOAD_DISPLAY_LIMIT * 2);
        let note = view.note().expect("a truncated payload must say so");
        assert!(
            note.contains(&PAYLOAD_DISPLAY_LIMIT.to_string()) && note.contains("8192"),
            "the note carries both numbers: {note}"
        );
    }

    #[test]
    fn l7_snippets_use_the_topic_and_a_real_subscription() {
        let payload = PayloadView::of(br#"{"title":"Backup klaar"}"#);
        let snippets = Snippets::build(
            "http://hub.lan",
            "notify.kenny",
            Some("printer"),
            Some(&payload),
            Some("application/json"),
        );

        assert!(snippets.publish.contains("notify.kenny"));
        assert!(snippets.publish.contains(r#"{"title":"Backup klaar"}"#));
        assert!(snippets.receive_raw.contains("as=printer"));
        assert!(snippets.receive_envelope.contains("envelope=json"));
        assert!(snippets.ack.contains("as=printer"));
        assert!(snippets.bootstrap_note.is_none());
    }

    #[test]
    fn l7_a_topic_without_consumers_explains_the_bootstrap_order() {
        let snippets = Snippets::build("http://hub.lan", "notify.kenny", None, None, None);
        let note = snippets
            .bootstrap_note
            .expect("a topic nobody polls must explain the order");
        assert!(
            note.contains("first polls") && note.contains("from=beginning"),
            "the note has to teach the G7 rule, since that is the trap: {note}"
        );
    }

    #[test]
    fn l7_a_quote_in_a_payload_cannot_break_out_of_the_snippet() {
        let payload = PayloadView::of(b"it's a trap");
        let snippets = Snippets::build("http://hub.lan", "t", Some("s"), Some(&payload), None);
        assert!(
            snippets.publish.contains(r"'\''"),
            "a single quote must be escaped for the shell: {}",
            snippets.publish
        );
    }
}
