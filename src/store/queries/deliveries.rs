//! Delivery rows on the consumer path: claiming the next message, acking and nacking.
//!
//! Part of the SQL layer (AR1, AR3); statements are parameterized (AR11).
use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, Transaction};

use super::Overdue;
use super::rows::policy_from_row;
use crate::engine::clock::Millis;

/// A message as stored, with the delivery counters of the subscription
/// that claimed it.
#[derive(Debug, Clone)]
pub struct ClaimedMessage {
    pub seq: i64,
    pub id: String,
    pub payload: Vec<u8>,
    pub content_type: Option<String>,
    pub published_at: Millis,
    /// Failed attempts so far; the attempt now in flight is this plus one.
    pub attempts: i64,
}

/// What acking found, so the caller can explain precisely what went wrong
/// instead of a bare 404 (AR4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AckOutcome {
    Acked,
    NoSuchDelivery,
    AlreadyAcked,
    NotClaimed { state: String },
}

pub fn insert_pending_delivery(tx: &Transaction, msg_seq: i64, sub_id: i64) -> Result<()> {
    tx.execute(
        "INSERT INTO deliveries (msg_seq, sub_id, state, attempts) VALUES (?1, ?2, 'pending', 0)",
        (msg_seq, sub_id),
    )
    .context("cannot create the delivery row")?;
    Ok(())
}

/// Claims the oldest deliverable message for one subscription.
///
/// Ordering is by `msg_seq`, the insertion sequence — never by the ULID,
/// because a clock that steps backwards after a power cut would otherwise
/// reorder the queue (AR7).
pub fn claim_next(
    tx: &Transaction,
    sub_id: i64,
    now: Millis,
    lease_ms: i64,
) -> Result<Option<ClaimedMessage>> {
    let claimed: Option<(i64, i64)> = tx
        .query_row(
            "UPDATE deliveries
                SET state = 'claimed', lease_expires_at = ?2 + ?3
              WHERE (msg_seq, sub_id) IN (
                    SELECT d.msg_seq, d.sub_id
                      FROM deliveries d
                      JOIN messages m ON m.seq = d.msg_seq
                     WHERE d.sub_id = ?1
                       AND d.state = 'pending'
                       AND (d.next_attempt_at IS NULL OR d.next_attempt_at <= ?2)
                       AND (m.due_at IS NULL OR m.due_at <= ?2)
                     ORDER BY d.msg_seq
                     LIMIT 1)
              RETURNING msg_seq, attempts",
            (sub_id, now, lease_ms),
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .context("cannot claim the next message")?;

    let Some((msg_seq, attempts)) = claimed else {
        return Ok(None);
    };

    let message = tx
        .query_row(
            "SELECT id, payload, content_type, published_at FROM messages WHERE seq = ?1",
            [msg_seq],
            |row| {
                Ok(ClaimedMessage {
                    seq: msg_seq,
                    id: row.get(0)?,
                    payload: row.get(1)?,
                    content_type: row.get(2)?,
                    published_at: row.get(3)?,
                    attempts,
                })
            },
        )
        .context("cannot read the claimed message")?;

    Ok(Some(message))
}

/// Settles a claimed delivery. The UPDATE is guarded by the current state,
/// so an illegal transition cannot happen (AR9); when it matches nothing,
/// a second query explains why.
pub fn ack(tx: &Transaction, message_id: &str, sub_id: i64) -> Result<AckOutcome> {
    let updated = tx
        .execute(
            "UPDATE deliveries
                SET state = 'acked', lease_expires_at = NULL
              WHERE sub_id = ?2
                AND state = 'claimed'
                AND msg_seq = (SELECT seq FROM messages WHERE id = ?1)",
            (message_id, sub_id),
        )
        .context("cannot acknowledge the message")?;

    if updated > 0 {
        return Ok(AckOutcome::Acked);
    }

    let state: Option<String> = tx
        .query_row(
            "SELECT d.state
               FROM deliveries d
               JOIN messages m ON m.seq = d.msg_seq
              WHERE m.id = ?1 AND d.sub_id = ?2",
            (message_id, sub_id),
            |row| row.get(0),
        )
        .optional()
        .context("cannot inspect the delivery state")?;

    Ok(match state.as_deref() {
        None => AckOutcome::NoSuchDelivery,
        Some("acked") => AckOutcome::AlreadyAcked,
        Some(other) => AckOutcome::NotClaimed {
            state: other.to_string(),
        },
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NackOutcome {
    Nacked,
    NoSuchDelivery,
    NotClaimed { state: String },
}

/// W5 · `claimed -> pending` on the consumer's own say-so, without waiting
/// out the lease. Returns the attempt count so the engine can apply the
/// same dead-letter and TTL rules as a lease expiry.
pub fn nack_claimed(tx: &Transaction, message_id: &str, sub_id: i64) -> Result<NackOutcome> {
    let found: Option<(i64, i64, String)> = tx
        .query_row(
            "SELECT d.msg_seq, d.attempts, d.state
               FROM deliveries d
               JOIN messages m ON m.seq = d.msg_seq
              WHERE m.id = ?1 AND d.sub_id = ?2",
            (message_id, sub_id),
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .context("cannot inspect the delivery")?;

    let Some((_, _, state)) = found else {
        return Ok(NackOutcome::NoSuchDelivery);
    };
    if state != "claimed" {
        return Ok(NackOutcome::NotClaimed { state });
    }
    Ok(NackOutcome::Nacked)
}

/// The delivery row behind a message id, for a subscription.
pub fn delivery_of(tx: &Transaction, message_id: &str, sub_id: i64) -> Result<Option<Overdue>> {
    tx.query_row(
        "SELECT d.msg_seq, d.sub_id, t.name, s.name, d.attempts, m.published_at,
                s.lease_ms, s.max_attempts, s.backoff_ms, s.ttl_ms,
                s.idle_flag_ms, s.idle_archive_ms
           FROM deliveries d
           JOIN subscriptions s ON s.id = d.sub_id
           JOIN topics t ON t.id = s.topic_id
           JOIN messages m ON m.seq = d.msg_seq
          WHERE m.id = ?1 AND d.sub_id = ?2",
        (message_id, sub_id),
        |row| {
            Ok(Overdue {
                msg_seq: row.get(0)?,
                sub_id: row.get(1)?,
                topic: row.get(2)?,
                subscription: row.get(3)?,
                attempts: row.get(4)?,
                published_at: row.get(5)?,
                policy: policy_from_row(row, 6)?,
            })
        },
    )
    .optional()
    .context("cannot look up the delivery")
}
