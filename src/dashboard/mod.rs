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

mod age;
mod payload;
mod snippets;
mod views;

pub use age::human_age;
pub use payload::{PAYLOAD_DISPLAY_LIMIT, PayloadView};
pub use snippets::{Snippet, Snippets, mask_token};
pub use views::{
    BacklogItemView, DeadLetterView, MessageView, StoredPolicyView, SubscriptionView, TopicView,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::clock::Millis;
    use crate::store::queries::{
        RecentMessage, StoredPolicy, SubscriptionSummary, TopicDeadLetter, TopicSummary,
    };

    /// [W14] Every timestamp the dashboard shows is human-readable, not the
    /// raw millisecond stamp `human_age` exists for — found live by Kenny
    /// reading his own topic page, where `oldest_unacked_at`, `dead_at`,
    /// `published_at` and `due_at` were still 13-digit numbers while
    /// `AppView.created_at` had already used `human_age` since W2. One test
    /// per view, each asserting the exact string `human_age` would produce
    /// for the same two timestamps, so a future field that reintroduces a
    /// raw `Millis` fails to compile rather than fails this test.
    #[test]
    fn w14_every_view_reports_human_readable_time_not_raw_milliseconds() {
        let now: Millis = 1_800_000_000_000;
        let five_minutes_ago = now - 5 * 60_000;

        let topic = TopicView::at(
            TopicSummary {
                name: "notify.kenny".to_string(),
                retention_ms: None,
                messages: 1,
                subscriptions: 1,
                backlog: 0,
                dead: 0,
                last_published_at: Some(five_minutes_ago),
            },
            now,
        );
        assert_eq!(topic.last_published_at.as_deref(), Some("5 minutes ago"));

        let subscription = SubscriptionView::at(
            SubscriptionSummary {
                name: "printer".to_string(),
                state: "active".to_string(),
                backlog: 1,
                claimed: 0,
                dead: 0,
                lapsed: 0,
                last_poll_at: Some(five_minutes_ago),
                oldest_unacked_at: Some(five_minutes_ago),
                policy: StoredPolicy {
                    lease_ms: None,
                    max_attempts: None,
                    backoff_ms: None,
                    ttl_ms: None,
                    idle_flag_ms: None,
                    idle_archive_ms: None,
                },
            },
            now,
        );
        assert_eq!(
            subscription.oldest_unacked_at.as_deref(),
            Some("5 minutes ago")
        );
        assert_eq!(subscription.last_poll_at.as_deref(), Some("5 minutes ago"));

        let dead_letter = DeadLetterView::at(
            TopicDeadLetter {
                subscription: "printer".to_string(),
                letter: crate::store::queries::DeadLetter {
                    id: "01M1QJ".to_string(),
                    published_at: five_minutes_ago,
                    dead_at: Some(five_minutes_ago),
                    attempts: 5,
                    content_type: None,
                    payload: b"kapot".to_vec(),
                },
            },
            now,
        );
        assert_eq!(dead_letter.published_at, "5 minutes ago");
        assert_eq!(dead_letter.dead_at.as_deref(), Some("5 minutes ago"));

        let message = MessageView::at(
            RecentMessage {
                id: "01M1QJ".to_string(),
                published_at: five_minutes_ago,
                due_at: Some(five_minutes_ago),
                content_type: None,
                payload: b"hello".to_vec(),
            },
            now,
        );
        assert_eq!(message.published_at, "5 minutes ago");
        assert_eq!(message.due_at.as_deref(), Some("5 minutes ago"));
    }
}
