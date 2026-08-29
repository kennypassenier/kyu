//! The hub's own events (W11), published as ordinary messages onto
//! `kyu.*` topics so that consuming them needs no special integration —
//! an HA automation subscribes exactly the way anything else does.
//!
//! One rule is load-bearing (AR1): events *about* a `kyu.*` topic are
//! logged, never republished. Without it a broken consumer of
//! `kyu.events` dead-letters, which emits a dead-letter event onto the
//! same topic, which dead-letters — a self-sustaining message generator.
//!
//! Events are written inside the transaction that caused them, so a hard
//! kill cannot settle a message and lose the announcement of it.

use anyhow::Result;
use rusqlite::Transaction;
use serde_json::json;

use crate::engine::clock::Millis;
use crate::engine::ids::MessageIds;
use crate::engine::names::RESERVED_PREFIX;
use crate::store::queries;

/// The system topic every hub event lands on.
pub const EVENTS_TOPIC: &str = "kyu.events";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    DeadLettered {
        topic: String,
        subscription: String,
        message_id: String,
        attempts: i64,
    },
    Expired {
        topic: String,
        subscription: String,
        count: usize,
    },
    SubscriptionFlagged {
        topic: String,
        subscription: String,
        idle_ms: i64,
    },
    SubscriptionArchived {
        topic: String,
        subscription: String,
        lapsed: usize,
    },
    SubscriptionUnarchived {
        topic: String,
        subscription: String,
    },
}

impl Event {
    /// The topic the event is *about*, which is what the loop-breaker keys
    /// on.
    ///
    /// Every event has one, and that is the point: an event with no subject
    /// could never be suppressed, so it would publish onto `kyu.events`
    /// unconditionally. Retention collecting those events used to do exactly
    /// that, refilling the topic it had just emptied. Housekeeping is logged
    /// instead.
    fn subject_topic(&self) -> Option<&str> {
        match self {
            Self::DeadLettered { topic, .. }
            | Self::Expired { topic, .. }
            | Self::SubscriptionFlagged { topic, .. }
            | Self::SubscriptionArchived { topic, .. }
            | Self::SubscriptionUnarchived { topic, .. } => Some(topic),
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Self::DeadLettered { .. } => "message.dead_lettered",
            Self::Expired { .. } => "message.expired",
            Self::SubscriptionFlagged { .. } => "subscription.flagged",
            Self::SubscriptionArchived { .. } => "subscription.archived",
            Self::SubscriptionUnarchived { .. } => "subscription.unarchived",
        }
    }

    fn payload(&self, now: Millis) -> serde_json::Value {
        let mut body = match self {
            Self::DeadLettered {
                topic,
                subscription,
                message_id,
                attempts,
            } => json!({
                "topic": topic,
                "subscription": subscription,
                "message_id": message_id,
                "attempts": attempts,
            }),
            Self::Expired {
                topic,
                subscription,
                count,
            } => json!({ "topic": topic, "subscription": subscription, "count": count }),
            Self::SubscriptionFlagged {
                topic,
                subscription,
                idle_ms,
            } => json!({ "topic": topic, "subscription": subscription, "idle_ms": idle_ms }),
            Self::SubscriptionArchived {
                topic,
                subscription,
                lapsed,
            } => json!({ "topic": topic, "subscription": subscription, "lapsed": lapsed }),
            Self::SubscriptionUnarchived {
                topic,
                subscription,
            } => json!({ "topic": topic, "subscription": subscription }),
        };
        let map = body.as_object_mut().expect("a JSON object");
        map.insert("event".to_string(), json!(self.kind()));
        map.insert("at".to_string(), json!(now));
        body
    }
}

/// Publishes `event` onto [`EVENTS_TOPIC`] inside the caller's transaction.
///
/// Returns the subscriptions that gained a message, so the caller can wake
/// their waiting polls.
pub fn emit(
    tx: &Transaction,
    ids: &mut MessageIds,
    now: Millis,
    event: &Event,
) -> Result<Vec<String>> {
    // The loop-breaker. An event about a kyu topic is written to the log
    // and stops there.
    if let Some(subject) = event.subject_topic()
        && subject.starts_with(RESERVED_PREFIX)
    {
        tracing::info!(
            event = event.kind(),
            subject,
            "hub event about a kyu topic — logged, not republished"
        );
        return Ok(Vec::new());
    }

    let topic_id = match queries::topic_id_by_name(tx, EVENTS_TOPIC)? {
        Some(id) => id,
        None => queries::create_topic(tx, EVENTS_TOPIC, now)?,
    };

    let payload = serde_json::to_vec(&event.payload(now))?;
    let id = ids.next(now).to_string();
    let seq = queries::insert_message(
        tx,
        &id,
        topic_id,
        &payload,
        Some("application/json"),
        now,
        None,
    )?;

    let subscribers = queries::active_subscriptions(tx, topic_id)?;
    for subscriber in &subscribers {
        queries::insert_pending_delivery(tx, seq, subscriber.id)?;
    }

    tracing::info!(event = event.kind(), id = %id, "hub event published");

    Ok(subscribers
        .into_iter()
        .map(|subscriber| subscriber.name)
        .collect())
}
