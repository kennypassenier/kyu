//! Subscription rows: lookup, creation, listing, poll heartbeat, deletion and the stored delivery policy.
//!
//! Part of the SQL layer (AR1, AR3); statements are parameterized (AR11).
use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, Transaction};

use super::rows::policy_from_row;
use crate::engine::clock::Millis;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subscription {
    pub id: i64,
    pub name: String,
}

pub fn subscription_id_by_name(tx: &Transaction, topic_id: i64, name: &str) -> Result<Option<i64>> {
    tx.query_row(
        "SELECT id FROM subscriptions WHERE topic_id = ?1 AND name = ?2",
        (topic_id, name),
        |row| row.get(0),
    )
    .optional()
    .with_context(|| format!("cannot look up subscription {name:?}"))
}

pub fn create_subscription(
    tx: &Transaction,
    topic_id: i64,
    name: &str,
    now: Millis,
) -> Result<i64> {
    // Policy columns stay NULL: a fresh subscription runs on the defaults
    // until someone sets them (K7, L4).
    tx.execute(
        "INSERT INTO subscriptions (topic_id, name, state, created_at, last_poll_at)
         VALUES (?1, ?2, 'active', ?3, ?3)",
        (topic_id, name, now),
    )
    .with_context(|| format!("cannot create subscription {name:?}"))?;
    Ok(tx.last_insert_rowid())
}

pub fn subscription_names(tx: &Transaction, topic_id: i64) -> Result<Vec<String>> {
    let mut statement = tx
        .prepare("SELECT name FROM subscriptions WHERE topic_id = ?1 ORDER BY name")
        .context("cannot list subscriptions")?;
    let names = statement
        .query_map([topic_id], |row| row.get::<_, String>(0))
        .context("cannot list subscriptions")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("cannot read the subscription list")?;
    Ok(names)
}

pub fn touch_subscription_poll(tx: &Transaction, sub_id: i64, now: Millis) -> Result<()> {
    // Feeds the idle-subscription lifecycle (K11, L6) and the dashboard's
    // alive/idle column.
    tx.execute(
        "UPDATE subscriptions SET last_poll_at = ?2 WHERE id = ?1",
        (sub_id, now),
    )
    .context("cannot record the poll time")?;
    Ok(())
}

/// The subscriptions a new message must be delivered to. Archived ones are
/// excluded, which is what keeps AR3's retention rule safe.
pub fn active_subscriptions(tx: &Transaction, topic_id: i64) -> Result<Vec<Subscription>> {
    let mut statement = tx
        .prepare(
            "SELECT id, name FROM subscriptions
              WHERE topic_id = ?1 AND state IN ('active', 'flagged')
              ORDER BY id",
        )
        .context("cannot list active subscriptions")?;
    let subscriptions = statement
        .query_map([topic_id], |row| {
            Ok(Subscription {
                id: row.get(0)?,
                name: row.get(1)?,
            })
        })
        .context("cannot list active subscriptions")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("cannot read the subscription list")?;
    Ok(subscriptions)
}

/// A subscription's policy as stored. `None` means "use the default", which
/// is resolved in the engine so the default lives in exactly one place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StoredPolicy {
    pub lease_ms: Option<i64>,
    pub max_attempts: Option<i64>,
    pub backoff_ms: Option<i64>,
    pub ttl_ms: Option<i64>,
    /// K11 · how long this subscription may go unpolled before it is
    /// flagged, then archived. `None` follows the hub-wide default.
    pub idle_flag_ms: Option<i64>,
    pub idle_archive_ms: Option<i64>,
}

pub fn subscription_policy(tx: &Transaction, sub_id: i64) -> Result<StoredPolicy> {
    tx.query_row(
        "SELECT lease_ms, max_attempts, backoff_ms, ttl_ms, idle_flag_ms, idle_archive_ms
           FROM subscriptions WHERE id = ?1",
        [sub_id],
        |row| policy_from_row(row, 0),
    )
    .context("cannot read the subscription policy")
}

/// Replace semantics: every field is written, so an absent field in the
/// request means "back to the default" rather than "leave whatever was
/// there". One rule is easier to remember than two.
pub fn set_subscription_policy(tx: &Transaction, sub_id: i64, policy: StoredPolicy) -> Result<()> {
    tx.execute(
        "UPDATE subscriptions
            SET lease_ms = ?2, max_attempts = ?3, backoff_ms = ?4, ttl_ms = ?5,
                idle_flag_ms = ?6, idle_archive_ms = ?7
          WHERE id = ?1",
        (
            sub_id,
            policy.lease_ms,
            policy.max_attempts,
            policy.backoff_ms,
            policy.ttl_ms,
            policy.idle_flag_ms,
            policy.idle_archive_ms,
        ),
    )
    .context("cannot store the subscription policy")?;
    Ok(())
}

/// Removes a subscription; its delivery rows follow through the foreign
/// key's cascade. The engine only calls this for an archived one.
pub fn delete_subscription(tx: &Transaction, sub_id: i64) -> Result<()> {
    tx.execute("DELETE FROM subscriptions WHERE id = ?1", [sub_id])
        .context("cannot delete the subscription")?;
    Ok(())
}

pub fn subscription_id_by_name_conn(
    conn: &rusqlite::Connection,
    topic_id: i64,
    name: &str,
) -> Result<Option<i64>> {
    conn.query_row(
        "SELECT id FROM subscriptions WHERE topic_id = ?1 AND name = ?2",
        (topic_id, name),
        |row| row.get(0),
    )
    .optional()
    .with_context(|| format!("cannot look up subscription {name:?}"))
}
