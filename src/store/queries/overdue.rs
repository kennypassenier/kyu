//! Deliveries that need a decision (lease ran out, TTL passed) and the transitions that follow: repend, dead, expired.
//!
//! Part of the SQL layer (AR1, AR3); statements are parameterized (AR11).
use anyhow::{Context, Result};
use rusqlite::Transaction;

use super::StoredPolicy;
use super::rows::policy_from_row;
use crate::engine::clock::Millis;

/// A claimed delivery whose lease ran out, with everything the engine needs
/// to decide where it goes next (AR9) without a second query per row.
#[derive(Debug, Clone)]
pub struct Overdue {
    pub msg_seq: i64,
    pub sub_id: i64,
    pub topic: String,
    pub subscription: String,
    pub attempts: i64,
    pub published_at: Millis,
    pub policy: StoredPolicy,
}

/// Claimed deliveries whose lease has run out. Bounded by `limit` (AR5):
/// the writer connection must never be held for a sweep over a huge table.
pub fn overdue_claims(tx: &Transaction, now: Millis, limit: usize) -> Result<Vec<Overdue>> {
    let mut statement = tx
        .prepare(
            "SELECT d.msg_seq, d.sub_id, t.name, s.name, d.attempts, m.published_at,
                    s.lease_ms, s.max_attempts, s.backoff_ms, s.ttl_ms,
                    s.idle_flag_ms, s.idle_archive_ms
               FROM deliveries d
               JOIN subscriptions s ON s.id = d.sub_id
               JOIN topics t ON t.id = s.topic_id
               JOIN messages m ON m.seq = d.msg_seq
              WHERE d.state = 'claimed' AND d.lease_expires_at <= ?1
              ORDER BY d.lease_expires_at
              LIMIT ?2",
        )
        .context("cannot scan for overdue claims")?;
    let rows = statement
        .query_map((now, limit as i64), |row| {
            Ok(Overdue {
                msg_seq: row.get(0)?,
                sub_id: row.get(1)?,
                topic: row.get(2)?,
                subscription: row.get(3)?,
                attempts: row.get(4)?,
                published_at: row.get(5)?,
                policy: policy_from_row(row, 6)?,
            })
        })
        .context("cannot scan for overdue claims")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("cannot read the overdue claims")?;
    Ok(rows)
}

/// Pending deliveries whose message has outlived its subscription's TTL.
pub fn pending_past_ttl(tx: &Transaction, now: Millis, limit: usize) -> Result<Vec<Overdue>> {
    let mut statement = tx
        .prepare(
            "SELECT d.msg_seq, d.sub_id, t.name, s.name, d.attempts, m.published_at,
                    s.lease_ms, s.max_attempts, s.backoff_ms, s.ttl_ms,
                    s.idle_flag_ms, s.idle_archive_ms
               FROM deliveries d
               JOIN subscriptions s ON s.id = d.sub_id
               JOIN topics t ON t.id = s.topic_id
               JOIN messages m ON m.seq = d.msg_seq
              WHERE d.state = 'pending'
                AND s.ttl_ms IS NOT NULL
                AND m.published_at + s.ttl_ms <= ?1
              ORDER BY d.msg_seq
              LIMIT ?2",
        )
        .context("cannot scan for expired messages")?;
    let rows = statement
        .query_map((now, limit as i64), |row| {
            Ok(Overdue {
                msg_seq: row.get(0)?,
                sub_id: row.get(1)?,
                topic: row.get(2)?,
                subscription: row.get(3)?,
                attempts: row.get(4)?,
                published_at: row.get(5)?,
                policy: policy_from_row(row, 6)?,
            })
        })
        .context("cannot scan for expired messages")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("cannot read the expired messages")?;
    Ok(rows)
}

/// `claimed -> pending` (AR9). State-guarded, so a delivery that was acked
/// between the scan and this update stays acked.
pub fn repend(
    tx: &Transaction,
    msg_seq: i64,
    sub_id: i64,
    attempts: i64,
    next_attempt_at: Millis,
) -> Result<bool> {
    let updated = tx
        .execute(
            "UPDATE deliveries
                SET state = 'pending', attempts = ?3, next_attempt_at = ?4,
                    lease_expires_at = NULL
              WHERE msg_seq = ?1 AND sub_id = ?2 AND state = 'claimed'",
            (msg_seq, sub_id, attempts, next_attempt_at),
        )
        .context("cannot return the delivery to pending")?;
    Ok(updated > 0)
}

/// `-> dead` (K6). Accepts both `claimed` (a poison pill nacked straight to
/// the dead list) and `pending` (retries exhausted).
pub fn mark_dead(
    tx: &Transaction,
    msg_seq: i64,
    sub_id: i64,
    attempts: i64,
    now: Millis,
) -> Result<bool> {
    let updated = tx
        .execute(
            "UPDATE deliveries
                SET state = 'dead', attempts = ?3, dead_at = ?4,
                    lease_expires_at = NULL, next_attempt_at = NULL
              WHERE msg_seq = ?1 AND sub_id = ?2 AND state IN ('claimed', 'pending')",
            (msg_seq, sub_id, attempts, now),
        )
        .context("cannot dead-letter the delivery")?;
    Ok(updated > 0)
}

/// `-> expired` (K7). A message that is past its TTL is settled, recorded
/// with the moment it lapsed rather than quietly deleted (G8).
pub fn mark_expired(tx: &Transaction, msg_seq: i64, sub_id: i64, now: Millis) -> Result<bool> {
    let updated = tx
        .execute(
            "UPDATE deliveries
                SET state = 'expired', expired_at = ?3,
                    lease_expires_at = NULL, next_attempt_at = NULL
              WHERE msg_seq = ?1 AND sub_id = ?2 AND state IN ('claimed', 'pending')",
            (msg_seq, sub_id, now),
        )
        .context("cannot expire the delivery")?;
    Ok(updated > 0)
}
