//! Subscription lifecycle (K11): state changes, idle detection for flagging and archiving, and lapsing outstanding deliveries.
//!
//! Part of the SQL layer (AR1, AR3); statements are parameterized (AR11).
use anyhow::{Context, Result};
use rusqlite::Transaction;

use super::close_subscription_attempts;
use crate::engine::clock::Millis;

#[derive(Debug, Clone)]
pub struct SubscriptionRef {
    pub id: i64,
    pub topic: String,
    pub name: String,
    /// The threshold that actually applied, so the event can say which.
    pub idle_ms: i64,
}

/// Subscriptions idle longer than their own threshold, or the hub's default
/// when they have not set one.
fn idle_subscriptions(
    tx: &Transaction,
    sql: &'static str,
    now: Millis,
    default_ms: Millis,
    limit: usize,
) -> Result<Vec<SubscriptionRef>> {
    let mut statement = tx
        .prepare(sql)
        .context("cannot scan for idle subscriptions")?;
    let rows = statement
        .query_map((now, default_ms, limit as i64), |row| {
            Ok(SubscriptionRef {
                id: row.get(0)?,
                topic: row.get(1)?,
                name: row.get(2)?,
                idle_ms: row.get(3)?,
            })
        })
        .context("cannot scan for idle subscriptions")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("cannot read the idle subscriptions")?;
    Ok(rows)
}

/// K11 · active subscriptions nobody has polled for a while. `created_at`
/// stands in for a subscription that has never polled at all.
pub fn subscriptions_to_flag(
    tx: &Transaction,
    now: Millis,
    default_ms: Millis,
    limit: usize,
) -> Result<Vec<SubscriptionRef>> {
    idle_subscriptions(
        tx,
        "SELECT s.id, t.name, s.name, COALESCE(s.idle_flag_ms, ?2)
           FROM subscriptions s JOIN topics t ON t.id = s.topic_id
          WHERE s.state = 'active'
            AND COALESCE(s.last_poll_at, s.created_at) <= ?1 - COALESCE(s.idle_flag_ms, ?2)
          ORDER BY s.id LIMIT ?3",
        now,
        default_ms,
        limit,
    )
}

/// K11 · subscriptions idle long enough to stop accumulating messages.
pub fn subscriptions_to_archive(
    tx: &Transaction,
    now: Millis,
    default_ms: Millis,
    limit: usize,
) -> Result<Vec<SubscriptionRef>> {
    idle_subscriptions(
        tx,
        "SELECT s.id, t.name, s.name, COALESCE(s.idle_archive_ms, ?2)
           FROM subscriptions s JOIN topics t ON t.id = s.topic_id
          WHERE s.state IN ('active', 'flagged')
            AND COALESCE(s.last_poll_at, s.created_at) <= ?1 - COALESCE(s.idle_archive_ms, ?2)
          ORDER BY s.id LIMIT ?3",
        now,
        default_ms,
        limit,
    )
}

pub fn set_subscription_state(tx: &Transaction, sub_id: i64, state: &str) -> Result<()> {
    tx.execute(
        "UPDATE subscriptions SET state = ?2 WHERE id = ?1",
        (sub_id, state),
    )
    .context("cannot change the subscription state")?;
    Ok(())
}

pub fn subscription_state(tx: &Transaction, sub_id: i64) -> Result<String> {
    tx.query_row(
        "SELECT state FROM subscriptions WHERE id = ?1",
        [sub_id],
        |row| row.get(0),
    )
    .context("cannot read the subscription state")
}

/// K11 · settles what an archived subscription was still holding.
///
/// `lapsed` rather than silent deletion: the count is reported, the state is
/// visible, and the messages themselves stay on the topic until retention
/// collects them (G8).
pub fn lapse_outstanding(tx: &Transaction, sub_id: i64, now: Millis) -> Result<usize> {
    close_subscription_attempts(tx, sub_id, now, "expired", "the subscription was archived")?;
    let lapsed = tx
        .execute(
            "UPDATE deliveries
                SET state = 'lapsed', lease_expires_at = NULL, next_attempt_at = NULL
              WHERE sub_id = ?1 AND state IN ('pending', 'claimed')",
            [sub_id],
        )
        .context("cannot lapse the outstanding deliveries")?;
    Ok(lapsed)
}
