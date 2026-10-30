//! The view models the dashboard templates render, with every time made human (W14).

use serde::Serialize;

use crate::engine::clock::Millis;
use crate::engine::policy::Policy;
use crate::store::queries::{
    BacklogItem, RecentMessage, StoredPolicy, SubscriptionSummary, TopicDeadLetter, TopicSummary,
};

use super::age::human_age;
use super::payload::PayloadView;

#[derive(Debug, Clone, Serialize)]
pub struct MessageView {
    pub id: String,
    /// Human-readable ("3 minutes ago"), not the raw millisecond stamp —
    /// see `human_age`. kyu's own dashboard was the one place on it that
    /// still printed epoch numbers; found by Kenny reading his own topic
    /// page.
    pub published_at: String,
    pub due_at: Option<String>,
    pub content_type: Option<String>,
    pub payload: PayloadView,
    pub note: Option<String>,
}

impl MessageView {
    pub fn at(message: RecentMessage, now: Millis) -> Self {
        let payload = PayloadView::of(&message.payload);
        let note = payload.note();
        Self {
            id: message.id,
            published_at: human_age(now, message.published_at),
            due_at: message.due_at.map(|at| human_age(now, at)),
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
    /// Human-readable, not the raw millisecond stamp — see `MessageView`.
    pub published_at: String,
    pub dead_at: Option<String>,
    pub attempts: i64,
    pub content_type: Option<String>,
    pub payload: PayloadView,
    pub note: Option<String>,
}

impl DeadLetterView {
    pub fn at(dead: TopicDeadLetter, now: Millis) -> Self {
        let payload = PayloadView::of(&dead.letter.payload);
        let note = payload.note();
        Self {
            subscription: dead.subscription,
            id: dead.letter.id,
            published_at: human_age(now, dead.letter.published_at),
            dead_at: dead.letter.dead_at.map(|at| human_age(now, at)),
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
    /// Human-readable, not the raw millisecond stamp — see `MessageView`.
    pub last_poll_at: Option<String>,
    pub oldest_unacked_at: Option<String>,
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

impl SubscriptionView {
    pub fn at(summary: SubscriptionSummary, now: Millis) -> Self {
        let effective = Policy::effective(summary.policy);
        Self {
            name: summary.name,
            state: summary.state,
            backlog: summary.backlog,
            claimed: summary.claimed,
            dead: summary.dead,
            lapsed: summary.lapsed,
            last_poll_at: summary.last_poll_at.map(|at| human_age(now, at)),
            oldest_unacked_at: summary.oldest_unacked_at.map(|at| human_age(now, at)),
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
    /// Human-readable, not the raw millisecond stamp — see `MessageView`.
    pub last_published_at: Option<String>,
}

impl TopicView {
    pub fn at(summary: TopicSummary, now: Millis) -> Self {
        Self {
            name: summary.name,
            retention_ms: summary.retention_ms,
            messages: summary.messages,
            subscriptions: summary.subscriptions,
            backlog: summary.backlog,
            dead: summary.dead,
            last_published_at: summary.last_published_at.map(|at| human_age(now, at)),
        }
    }
}

/// [W16] One item on a subscription's live backlog, as the dashboard shows
/// it. `state` is `pending` or `claimed` — everything `backlog_for_subscription`
/// can return.
#[derive(Debug, Clone, Serialize)]
pub struct BacklogItemView {
    pub id: String,
    pub state: String,
    /// Human-readable, not the raw millisecond stamp — see `MessageView`.
    pub published_at: String,
    pub attempts: i64,
    pub content_type: Option<String>,
    pub payload: PayloadView,
    pub note: Option<String>,
}

impl BacklogItemView {
    pub fn at(item: BacklogItem, now: Millis) -> Self {
        let payload = PayloadView::of(&item.payload);
        let note = payload.note();
        Self {
            id: item.id,
            state: item.state,
            published_at: human_age(now, item.published_at),
            attempts: item.attempts,
            content_type: item.content_type,
            payload,
            note,
        }
    }
}
