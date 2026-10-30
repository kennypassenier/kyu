//! Delivery rows on the administrative path: backfilling for new subscriptions and removing deliveries.
//!
//! Part of the SQL layer (AR1, AR3); statements are parameterized (AR11).
use anyhow::{Context, Result};
use rusqlite::Transaction;

use super::{close_attempt, message_seq_of_id};
use crate::engine::clock::Millis;

/// [W15, W16] Removes one delivery for good, whatever state it is in —
/// dead, pending or claimed. Two dashboard buttons share this: Delete on a
/// dead letter (W15) and Delete on a still-live backlog item (W16, the
/// per-subscription view). One mechanism rather than two nearly-identical
/// ones, because "remove this from this subscription's queue" does not
/// care what state it removes it from.
///
/// Only this subscription's delivery changes; `messages` and every other
/// subscription's delivery for the same message are untouched, since the
/// payload lives on the message, not on this row (AR2 — one message, fanned
/// out to N deliveries).
///
/// The row is marked `removed` rather than deleted (migration 6): K8's
/// replay creates a delivery for every retained message a subscription has
/// no row for, so a deleted row came back at the next `from=beginning`
/// poll — the CT 109 dead letter Kenny deleted twice (2026-09-30).
pub fn delete_delivery(
    tx: &Transaction,
    message_id: &str,
    sub_id: i64,
    now: Millis,
) -> Result<DeleteOutcome> {
    if let Some(msg_seq) = message_seq_of_id(tx, message_id)? {
        close_attempt(
            tx,
            msg_seq,
            sub_id,
            now,
            "nacked",
            "deleted from the dashboard",
        )?;
    }
    let deleted = tx
        .execute(
            "UPDATE deliveries
                SET state = 'removed', lease_expires_at = NULL, next_attempt_at = NULL
              WHERE sub_id = ?2
                AND state <> 'removed'
                AND msg_seq = (SELECT seq FROM messages WHERE id = ?1)",
            (message_id, sub_id),
        )
        .context("cannot delete the delivery")?;

    Ok(if deleted > 0 {
        DeleteOutcome::Deleted
    } else {
        DeleteOutcome::NoSuchDelivery
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeleteOutcome {
    Deleted,
    NoSuchDelivery,
}

/// K8 · gives a subscription a delivery row for every retained message it
/// does not already have.
///
/// Idempotent by construction, so replaying twice cannot duplicate work,
/// and bounded so a seven-day topic does not hold the writer for one poll.
/// The retention sweep cannot delete a message mid-backfill: both run on
/// the single writer connection (AR5), so they are serialised rather than
/// racing.
pub fn backfill_deliveries(
    tx: &Transaction,
    topic_id: i64,
    sub_id: i64,
    limit: usize,
) -> Result<usize> {
    let inserted = tx
        .execute(
            "INSERT INTO deliveries (msg_seq, sub_id, state, attempts)
             SELECT m.seq, ?2, 'pending', 0
               FROM messages m
              WHERE m.topic_id = ?1
                AND NOT EXISTS (
                      SELECT 1 FROM deliveries d
                       WHERE d.msg_seq = m.seq AND d.sub_id = ?2)
              ORDER BY m.seq
              LIMIT ?3",
            (topic_id, sub_id, limit as i64),
        )
        .context("cannot backfill the subscription")?;
    Ok(inserted)
}
