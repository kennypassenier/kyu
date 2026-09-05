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
use crate::store::queries::{
    RecentMessage, StoredPolicy, SubscriptionSummary, TopicDeadLetter, TopicSummary,
};

/// How much of a payload the dashboard shows. Enough to recognise a
/// message; the rest is announced rather than dropped in silence (AR11).
pub const PAYLOAD_DISPLAY_LIMIT: usize = 4096;

/// The house themes, from `@kp-soft/themes` **v3.0.0** — the shared package
/// JobTracker, almanac and kp-soft use.
///
/// Only the name and the label. v1.0.0 removed the colour copies on purpose
/// (their TH24): a swatch now wears the theme it previews, reading the live
/// custom properties instead of a duplicate that drifts when a palette is
/// adjusted. The dark flag is gone from here for the same reason — the
/// package derives it from each theme's own `color-scheme`, which is how
/// kyu came to believe in four dark themes when there are three.
///
/// This list still exists because kyu renders its picker server-side: a
/// menu built by JavaScript is an empty box until the script runs, and this
/// dashboard is server-rendered HTML. It is kept honest by a gate that
/// compares it against the package's generated `js/theme-registry.js` and
/// refuses the commit when they disagree — the same guard as the vendored
/// stylesheets.
pub const THEMES: &[ThemeView] = &[
    ThemeView {
        name: "formal",
        label: "Formal",
    },
    ThemeView {
        name: "light",
        label: "Light",
    },
    ThemeView {
        name: "dark",
        label: "Dark",
    },
    ThemeView {
        name: "cyberpunk",
        label: "Cyberpunk",
    },
    ThemeView {
        name: "pastel",
        label: "Pastel",
    },
    ThemeView {
        name: "terminal",
        label: "Terminal",
    },
    ThemeView {
        name: "topo",
        label: "Topographic",
    },
    ThemeView {
        name: "high-contrast",
        label: "High contrast",
    },
    ThemeView {
        name: "sepia",
        label: "Sepia",
    },
    ThemeView {
        name: "blueprint",
        label: "Blueprint",
    },
    ThemeView {
        name: "solstice",
        label: "Solstice",
    },
];

/// One theme as the picker's markup needs it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ThemeView {
    pub name: &'static str,
    /// Shown to the reader. English, matching the rest of this dashboard —
    /// and, since the package's 2.0.0, its own default too. Before that
    /// release the package's labels were Dutch and so was this list; kept
    /// in step here rather than kept as a deliberate override, because
    /// nothing else on this dashboard is Dutch.
    pub label: &'static str,
}

/// Templates are embedded in the binary, so the image stays a single file
/// with nothing to mount beside it (T9).
static ENVIRONMENT: LazyLock<Environment<'static>> = LazyLock::new(|| {
    let mut environment = Environment::new();
    // Autoescape is the difference between a dashboard and a stored-XSS
    // delivery system: every payload on these pages came from outside.
    environment.set_auto_escape_callback(|_| minijinja::AutoEscape::Html);
    // A global rather than a context key on all four render functions: the
    // picker sits in the layout, so every page needs it and none of them
    // should have to remember to pass it.
    environment.add_global("themes", minijinja::Value::from_serialize(THEMES));
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
        .add_template("login.html", include_str!("../templates/login.html"))
        .expect("the login template must compile");
    environment
        .add_template("apps.html", include_str!("../templates/apps.html"))
        .expect("the apps template must compile");
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
        let truncated = bytes > shown;

        let text = match std::str::from_utf8(&payload[..shown]) {
            Ok(text) => Some(text),
            // `error_len() == None` means the slice ends in the middle of a
            // character, which is what truncation does to any non-ASCII
            // payload. Backing off to the last whole character is the
            // difference between a readable message and one that claims to
            // be binary.
            Err(error) if truncated && error.error_len().is_none() => {
                std::str::from_utf8(&payload[..error.valid_up_to()]).ok()
            }
            Err(_) => None,
        };

        match text {
            Some(text) => Self {
                text: text.to_string(),
                bytes,
                truncated,
                binary: false,
            },
            None => Self {
                text: String::new(),
                bytes,
                truncated,
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

/// K6 · a dead letter as the dashboard shows it: which subscription gave up
/// on it, when, after how many attempts, and what it actually said.
#[derive(Debug, Clone, Serialize)]
pub struct DeadLetterView {
    pub subscription: String,
    pub id: String,
    pub published_at: Millis,
    pub dead_at: Option<Millis>,
    pub attempts: i64,
    pub content_type: Option<String>,
    pub payload: PayloadView,
    pub note: Option<String>,
}

impl From<TopicDeadLetter> for DeadLetterView {
    fn from(dead: TopicDeadLetter) -> Self {
        let payload = PayloadView::of(&dead.letter.payload);
        let note = payload.note();
        Self {
            subscription: dead.subscription,
            id: dead.letter.id,
            published_at: dead.letter.published_at,
            dead_at: dead.letter.dead_at,
            attempts: dead.letter.attempts,
            content_type: dead.letter.content_type,
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
    pub lapsed: i64,
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
            lapsed: summary.lapsed,
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

/// Whether a stored content type is safe to print inside a shell command:
/// a token, a slash, a token, and optional `; key=value` parameters. No
/// quotes, no spaces beyond the parameter separators, no shell metacharacters.
fn is_plain_media_type(value: &str) -> bool {
    fn is_token(part: &str) -> bool {
        !part.is_empty()
            && part.bytes().all(|byte| {
                byte.is_ascii_alphanumeric()
                    || matches!(
                        byte,
                        b'!' | b'#' | b'$' | b'&' | b'^' | b'_' | b'.' | b'+' | b'-'
                    )
            })
    }

    let mut parts = value.split(';');
    let Some((kind, subtype)) = parts.next().and_then(|media| media.split_once('/')) else {
        return false;
    };
    if !is_token(kind) || !is_token(subtype) {
        return false;
    }
    parts.all(|parameter| {
        parameter
            .trim()
            .split_once('=')
            .is_some_and(|(name, argument)| is_token(name) && is_token(argument.trim_matches('"')))
    })
}

/// The copy-paste examples, rendered with this topic's own real payload and
/// a subscription name that actually exists. Generic documentation is what
/// you skim; your own data is what you trust (S1).
#[derive(Debug, Clone, Serialize)]
pub struct Snippets {
    pub publish: Snippet,
    pub receive_raw: Snippet,
    pub receive_envelope: Snippet,
    pub ack: Snippet,
    pub bootstrap_note: Option<String>,
    /// Which app's token the commands carry, for the selector on the page.
    pub app: Option<String>,
}

/// One copy-paste command in two forms (W2).
///
/// `shown` is what the page displays, with the token masked; `full` is what
/// the copy button puts on the clipboard. Keeping them apart is the whole
/// point of Kenny's requirement: pasting works immediately, but someone
/// glancing at the screen over your shoulder learns nothing.
#[derive(Debug, Clone, Serialize)]
pub struct Snippet {
    pub shown: String,
    pub full: String,
}

impl Snippet {
    /// Builds both forms from a command that may carry a token.
    ///
    /// The masked form is produced by substituting the token, not by
    /// re-rendering: any future edit to the command shape then cannot make
    /// the two drift apart and accidentally print a real token.
    fn new(full: String, token: Option<&str>) -> Self {
        let shown = match token {
            Some(token) if !token.is_empty() => full.replace(token, &mask_token(token)),
            _ => full.clone(),
        };
        Self { shown, full }
    }
}

impl Snippets {
    pub fn build(
        base_url: &str,
        topic: &str,
        subscription: Option<&str>,
        example_payload: Option<&PayloadView>,
        content_type: Option<&str>,
        token: Option<&str>,
        app: Option<&str>,
    ) -> Self {
        let payload = match example_payload {
            Some(view) if !view.binary && !view.text.is_empty() => {
                let single_line: String = view.text.lines().collect::<Vec<_>>().join("");
                single_line.replace('\'', "'\\''")
            }
            _ => r#"{"hello":"world"}"#.to_string(),
        };
        // The content type is whatever a publisher put in the header, and
        // this string is pasted into a shell. A quote in it closes the
        // argument and everything after it runs on the operator's machine —
        // HTML escaping protects the browser, not the clipboard. Anything
        // that is not an ordinary media type is replaced rather than
        // escaped, because a snippet nobody can read is no use either.
        let content_type = content_type
            .filter(|value| is_plain_media_type(value))
            .unwrap_or("application/json");
        let name = subscription.unwrap_or("my-consumer");

        // One extra argument line when the hub has a door, and none at all
        // when it does not — an unprotected hub must not print an
        // authorization header that would only confuse whoever pastes it.
        let auth = match token {
            Some(token) if !token.is_empty() => {
                format!("-H 'authorization: Bearer {token}' \\\n     ")
            }
            _ => String::new(),
        };

        Self {
            publish: Snippet::new(
                format!(
                    "curl -s {auth}-H 'content-type: {content_type}' \\\n     -d '{payload}' \\\n     {base_url}/t/{topic}"
                ),
                token,
            ),
            receive_raw: Snippet::new(
                format!(
                    "curl -s {auth}-D- -o message.body \"{base_url}/t/{topic}/next?as={name}\""
                ),
                token,
            ),
            receive_envelope: Snippet::new(
                format!("curl -s {auth}\"{base_url}/t/{topic}/next?as={name}&envelope=json\""),
                token,
            ),
            ack: Snippet::new(
                format!("curl -s -X POST {auth}\"{base_url}/t/{topic}/ack/<id>?as={name}\""),
                token,
            ),
            app: app.map(str::to_string),
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

/// The cache-busting fingerprint every page appends to its asset URLs.
fn asset_version() -> &'static str {
    crate::http::handlers::ASSET_VERSION.as_str()
}

pub fn render_topics(topics: Vec<TopicView>, now: Millis, protected: bool) -> Result<String> {
    ENVIRONMENT
        .get_template("topics.html")
        .context("the topics template is missing")?
        .render(minijinja::context! {
            topics => topics,
            now => now,
            protected => protected,
            active_nav => "topics",
            assets => asset_version(),
        })
        .context("cannot render the topic list")
}

/// W2 · the login page. `error` is shown above the form after a refusal.
pub fn render_login(error: Option<&str>) -> Result<String> {
    ENVIRONMENT
        .get_template("login.html")
        .context("the login template is missing")?
        .render(minijinja::context! { error => error, assets => asset_version() })
        .context("cannot render the login page")
}

/// W2 · one registered app as the apps page shows it.
///
/// `token` is the value the copy button puts on the clipboard and `masked`
/// is what the page displays until someone reveals it. Both are rendered
/// into the HTML — which is exactly why this page is behind the door.
#[derive(Debug, Clone, Serialize)]
pub struct AppView {
    pub name: String,
    pub token: String,
    pub masked: String,
    pub live: bool,
    pub created_at: String,
    pub revoked_at: Option<String>,
}

/// Shows enough of a token to tell two of them apart, and no more.
pub fn mask_token(token: &str) -> String {
    const SHOWN: usize = 4;
    if token.len() <= SHOWN {
        return "\u{2022}".repeat(8);
    }
    format!("{}{}", "\u{2022}".repeat(8), &token[token.len() - SHOWN..])
}

/// A coarse age. The rest of the dashboard prints raw millisecond stamps;
/// for "when did I create this token" the only question anyone actually has
/// is how long ago, and a 13-digit number does not answer it.
pub fn human_age(now: Millis, then: Millis) -> String {
    let elapsed = (now - then).max(0);
    let minutes = elapsed / 60_000;
    let hours = minutes / 60;
    let days = hours / 24;
    match (days, hours, minutes) {
        (0, 0, 0) => "just now".to_string(),
        (0, 0, 1) => "1 minute ago".to_string(),
        (0, 0, m) => format!("{m} minutes ago"),
        (0, 1, _) => "1 hour ago".to_string(),
        (0, h, _) => format!("{h} hours ago"),
        (1, _, _) => "yesterday".to_string(),
        (d, _, _) => format!("{d} days ago"),
    }
}

pub fn render_apps(apps: &[AppView], error: Option<&str>) -> Result<String> {
    ENVIRONMENT
        .get_template("apps.html")
        .context("the apps template is missing")?
        .render(minijinja::context! {
            apps => apps,
            error => error,
            protected => true,
            active_nav => "apps",
            reveal_seconds => crate::config::REVEAL_SECONDS,
            assets => asset_version(),
        })
        .context("cannot render the apps page")
}

/// W2 · the apps page on a hub with no door yet.
///
/// AR11 keeps app-token *creation* behind a bootstrap token on purpose — a
/// per-app token only means something once something already decides who
/// may in at all. But the page itself should exist regardless, so a visitor
/// finds the ten-second fix (set two environment variables and restart)
/// rather than a route that looks like it was never built. `example_token`
/// and `example_key` are generated fresh per request, the same way the CLI
/// prints one when refusing to start on a token without a key.
pub fn render_apps_setup(example_token: &str, example_key: &str) -> Result<String> {
    ENVIRONMENT
        .get_template("apps.html")
        .context("the apps template is missing")?
        .render(minijinja::context! {
            apps => Vec::<AppView>::new(),
            error => Option::<&str>::None,
            protected => false,
            example_token => example_token,
            example_key => example_key,
            active_nav => "apps",
            reveal_seconds => crate::config::REVEAL_SECONDS,
            assets => asset_version(),
        })
        .context("cannot render the apps setup page")
}

#[allow(clippy::too_many_arguments)]
pub fn render_topic(
    topic: TopicView,
    subscriptions: Vec<SubscriptionView>,
    messages: Vec<MessageView>,
    dead_letters: Vec<DeadLetterView>,
    snippets: Snippets,
    now: Millis,
    protected: bool,
    app_names: Vec<String>,
) -> Result<String> {
    ENVIRONMENT
        .get_template("topic.html")
        .context("the topic template is missing")?
        .render(minijinja::context! {
            topic => topic,
            subscriptions => subscriptions,
            messages => messages,
            dead_letters => dead_letters,
            snippets => snippets,
            now => now,
            protected => protected,
            app_names => app_names,
            active_nav => "topics",
            reveal_seconds => crate::config::REVEAL_SECONDS,
            assets => asset_version(),
        })
        .context("cannot render the topic page")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn p7_a_character_split_by_the_display_cap_does_not_look_like_binary() {
        // 4095 ASCII bytes then a two-byte character: the cut lands in the
        // middle of it. Reported as binary, a perfectly ordinary Dutch
        // message would be unreadable on the page.
        let mut payload = "x".repeat(PAYLOAD_DISPLAY_LIMIT - 1).into_bytes();
        payload.extend_from_slice("é".as_bytes());

        let view = PayloadView::of(&payload);

        assert!(
            !view.binary,
            "text must stay text even when truncation lands mid-character"
        );
        assert!(view.truncated);
        assert_eq!(
            view.text.len(),
            PAYLOAD_DISPLAY_LIMIT - 1,
            "the display backs off to the last whole character"
        );
        assert_eq!(view.bytes, PAYLOAD_DISPLAY_LIMIT + 1);
    }

    #[test]
    fn p7_genuinely_invalid_bytes_are_still_reported_as_binary() {
        // The fix for the boundary case must not turn every binary payload
        // into a half-rendered string.
        let payload = vec![0x41, 0x42, 0xff, 0xfe];
        let view = PayloadView::of(&payload);
        assert!(view.binary, "invalid bytes in the middle are not text");
    }

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
            None,
            None,
        );

        assert!(snippets.publish.full.contains("notify.kenny"));
        assert!(
            snippets
                .publish
                .full
                .contains(r#"{"title":"Backup klaar"}"#)
        );
        assert!(snippets.receive_raw.full.contains("as=printer"));
        assert!(snippets.receive_envelope.full.contains("envelope=json"));
        assert!(snippets.ack.full.contains("as=printer"));
        assert!(snippets.bootstrap_note.is_none());
    }

    #[test]
    fn l7_a_topic_without_consumers_explains_the_bootstrap_order() {
        let snippets = Snippets::build(
            "http://hub.lan",
            "notify.kenny",
            None,
            None,
            None,
            None,
            None,
        );
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
        let snippets = Snippets::build(
            "http://hub.lan",
            "t",
            Some("s"),
            Some(&payload),
            None,
            None,
            None,
        );
        assert!(
            snippets.publish.full.contains(r"'\''"),
            "a single quote must be escaped for the shell: {}",
            snippets.publish.full
        );
    }
}
