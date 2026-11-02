//! Delivery semantics (AR1, AR9): publish, claim, ack, nack,
//! redelivery, dead-lettering, TTL, retention, idle lifecycle.
//!
//! Free of tokio and HTTP, and it never reads the wall clock: time arrives
//! through a [`clock::Clock`] and storage through [`crate::store`], which
//! is what makes the mocked-clock suites (K5, K7, K9, K11) possible.
//!
//! L2 lands the three verbs. Leases only *start* here — expiry,
//! redelivery, dead-lettering and TTL are L4, and until then an unacked
//! message stays claimed.

mod apps;
mod claim;
pub mod clock;
mod clockguard;
pub mod days;
mod dead;
pub mod dedup;
mod error;
pub mod ids;
pub mod names;
pub mod policy;
mod publish;
mod settle;
mod store_access;
mod subscriptions;
mod sweep;
mod sweep_report;

pub use apps::App;
pub use clockguard::{CLOCK_JUMP_LIMIT_MS, ClockSkew};
pub use error::EngineError;
use error::{Result, check_name};
pub use settle::Settled;
pub use sweep_report::SweepReport;

use std::sync::{Arc, Mutex};

use chassis::storage::AppDb;

use crate::store::queries::{self, ClaimedMessage};

use clock::{Clock, Millis};
use ids::MessageIds;

/// What a publish did, so the caller can wake exactly the subscriptions
/// that gained a message. The engine stays free of tokio by reporting them
/// instead of notifying them itself (AR1, AR5).
#[derive(Debug, Clone)]
pub struct Published {
    pub id: String,
    pub delivered_to: Vec<String>,
    /// `Some` when the message is scheduled rather than immediate (W4).
    pub due_at: Option<Millis>,
    /// True when a dedup key suppressed this publish: nothing was stored and
    /// `id` is the original message's.
    pub duplicate: bool,
}

/// A subscription that this very poll brought into existence, with the
/// number of messages the topic already held. Reported so the caller can
/// say so out loud: a fresh subscription's first poll finds nothing by
/// design (G7), and an unexplained 204 there looks exactly like a lost
/// message (G8).
#[derive(Debug, Clone)]
pub struct NewSubscription {
    pub name: String,
    pub retained_before: i64,
}

/// The outcome of a poll: what was claimed, and whether the subscription
/// was born in the process.
#[derive(Debug, Clone)]
pub struct Received {
    pub claimed: Option<Claimed>,
    pub created: Option<NewSubscription>,
    /// How many retained messages a `from=beginning` poll pulled in (K8).
    pub backfilled: usize,
}

/// A claimed message and where it came from.
#[derive(Debug, Clone)]
pub struct Claimed {
    pub topic: String,
    pub message: ClaimedMessage,
}

impl Claimed {
    /// Which attempt this delivery is. `attempts` counts the ones that
    /// already failed (AR9), so the one in flight is the next number.
    pub fn attempt(&self) -> i64 {
        self.message.attempts + 1
    }
}

/// Hub-wide defaults that are not per-subscription policy (AR6): they are
/// read from the environment once and apply wherever nothing more specific
/// is set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Defaults {
    /// How long a topic keeps messages nobody still needs (K9).
    pub retention_ms: Option<Millis>,
    /// Idle time before a subscription is flagged on the dashboard (K11).
    pub idle_flag_ms: Millis,
    /// Idle time before it is archived and stops accumulating (K11).
    pub idle_archive_ms: Millis,
    /// How long one `message.expired` announcement stands for a
    /// subscription before the next one may go out (W11, amended
    /// 2026-09-18). A day: one line with a count says what a subscriber
    /// needs to know, and a subscription nobody polls expires everything.
    pub expired_event_window_ms: Millis,
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            retention_ms: Some(7 * 24 * 60 * 60 * 1_000),
            idle_flag_ms: 7 * 24 * 60 * 60 * 1_000,
            idle_archive_ms: 30 * 24 * 60 * 60 * 1_000,
            expired_event_window_ms: 24 * 60 * 60 * 1_000,
        }
    }
}

pub struct Engine {
    db: AppDb,
    clock: Arc<dyn Clock>,
    ids: Mutex<MessageIds>,
    clock_guard: clockguard::ClockGuard,
    defaults: Defaults,
}

impl Engine {
    pub fn new(db: AppDb, clock: Arc<dyn Clock>) -> Self {
        Self::with_defaults(db, clock, Defaults::default())
    }

    pub fn with_defaults(db: AppDb, clock: Arc<dyn Clock>, defaults: Defaults) -> Self {
        Self {
            db,
            clock,
            ids: Mutex::new(MessageIds::new()),
            clock_guard: clockguard::ClockGuard::default(),
            defaults,
        }
    }

    /// The id generator, recovered if a panic poisoned its lock. The guarded
    /// state is a monotonic ULID generator whose every step leaves it valid,
    /// so a poisoned lock is safe to reuse; failing instead would break
    /// publish and sweep for the rest of the process.
    fn lock_ids(&self) -> std::sync::MutexGuard<'_, MessageIds> {
        self.ids
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn defaults(&self) -> Defaults {
        self.defaults
    }

    pub fn now_ms(&self) -> Millis {
        self.clock.now_ms()
    }

    fn resolve_subscription(
        &self,
        tx: &rusqlite::Transaction,
        topic: &str,
        subscription: &str,
    ) -> Result<i64> {
        check_name("topic", topic)?;
        check_name("subscription", subscription)?;
        let Some(topic_id) = queries::topic_id_by_name(tx, topic)? else {
            return Err(EngineError::UnknownTopic {
                topic: topic.to_string(),
                existing: queries::topic_names(tx)?,
            });
        };
        let Some(sub_id) = queries::subscription_id_by_name(tx, topic_id, subscription)? else {
            return Err(EngineError::UnknownSubscription {
                topic: topic.to_string(),
                subscription: subscription.to_string(),
                existing: queries::subscription_names(tx, topic_id)?,
            });
        };
        Ok(sub_id)
    }
}
