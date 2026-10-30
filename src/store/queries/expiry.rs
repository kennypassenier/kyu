//! Expiry bookkeeping: per-subscription expiry counts and the announcements derived from them.
//!
//! Part of the SQL layer (AR1, AR3); statements are parameterized (AR11).
use anyhow::{Context, Result};
use rusqlite::Transaction;

use crate::engine::clock::Millis;

/// W11 (amended 2026-09-18): expiries are announced per window, not per
/// sweep, so a settled expiry is first counted on its subscription.
pub fn note_expiry(tx: &Transaction, sub_id: i64, count: i64) -> Result<()> {
    tx.execute(
        "UPDATE subscriptions
            SET expired_unannounced = expired_unannounced + ?2
          WHERE id = ?1",
        (sub_id, count),
    )
    .context("cannot count the expired delivery")?;
    Ok(())
}

/// One `message.expired` event waiting to be published.
#[derive(Debug, Clone)]
pub struct ExpiryAnnouncement {
    pub sub_id: i64,
    pub topic: String,
    pub subscription: String,
    pub count: usize,
}

/// Subscriptions holding unannounced expiries whose previous announcement
/// is at least `window_ms` old — or that never announced one, so the first
/// expiry after a quiet spell goes out at once.
pub fn expiry_announcements_due(
    tx: &Transaction,
    now: Millis,
    window_ms: Millis,
    limit: usize,
) -> Result<Vec<ExpiryAnnouncement>> {
    let mut statement = tx
        .prepare(
            "SELECT s.id, t.name, s.name, s.expired_unannounced
               FROM subscriptions s
               JOIN topics t ON t.id = s.topic_id
              WHERE s.expired_unannounced > 0
                AND (s.expired_announced_at IS NULL
                     OR s.expired_announced_at + ?2 <= ?1)
              ORDER BY s.id
              LIMIT ?3",
        )
        .context("cannot scan for expiry announcements")?;
    let rows = statement
        .query_map((now, window_ms, limit as i64), |row| {
            let count: i64 = row.get(3)?;
            Ok(ExpiryAnnouncement {
                sub_id: row.get(0)?,
                topic: row.get(1)?,
                subscription: row.get(2)?,
                count: usize::try_from(count).unwrap_or(0),
            })
        })
        .context("cannot scan for expiry announcements")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("cannot read the expiry announcements")?;
    Ok(rows)
}

/// The announcement went out: stamp the moment and start counting afresh.
pub fn mark_expiries_announced(tx: &Transaction, sub_id: i64, now: Millis) -> Result<()> {
    tx.execute(
        "UPDATE subscriptions
            SET expired_announced_at = ?2, expired_unannounced = 0
          WHERE id = ?1",
        (sub_id, now),
    )
    .context("cannot record the expiry announcement")?;
    Ok(())
}
