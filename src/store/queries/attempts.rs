//! Delivery attempt history and the retention windows for history, hourly, daily and sample data.
//!
//! Part of the SQL layer (AR1, AR3); statements are parameterized (AR11).
use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, Transaction};

use crate::engine::clock::Millis;

/// How long an attempt row is kept even when its message is (the sweeper's
/// cap; the cascade already removes rows with their message).
pub const ATTEMPTS_KEPT_MS: Millis = 30 * 24 * 60 * 60 * 1_000;

/// How long hourly counters are kept.
pub const HOURLY_KEPT_MS: Millis = 30 * 24 * 60 * 60 * 1_000;

/// How long daily counters are kept, in days.
pub const DAILY_KEPT_DAYS: i64 = 400;

/// How long hub-total samples are kept.
pub const SAMPLES_KEPT_MS: Millis = 48 * 60 * 60 * 1_000;

/// How often the sweeper samples the hub totals.
pub const SAMPLE_EVERY_MS: Millis = 5 * 60 * 1_000;

/// One delivery attempt as the dashboard shows it.
#[derive(Debug, Clone)]
pub struct AttemptRow {
    pub n: i64,
    pub claimed_at: Millis,
    pub ended_at: Option<Millis>,
    pub outcome: String,
    pub app: Option<String>,
    pub reason: Option<String>,
}

/// Records a claim as attempt number `n` of this delivery. A stray attempt
/// that never ended (it cannot happen through the engine, but a crash is
/// not the engine) is closed first, so a delivery never has two in flight.
pub fn open_attempt(
    tx: &Transaction,
    msg_seq: i64,
    sub_id: i64,
    now: Millis,
    app: Option<&str>,
) -> Result<()> {
    close_attempt(
        tx,
        msg_seq,
        sub_id,
        now,
        "expired",
        "superseded by a new claim",
    )?;
    tx.execute(
        "INSERT INTO attempts (msg_seq, sub_id, n, claimed_at, outcome, app)
         VALUES (?1, ?2,
                 COALESCE((SELECT max(n) FROM attempts WHERE msg_seq = ?1 AND sub_id = ?2), 0) + 1,
                 ?3, 'in_flight', ?4)",
        (msg_seq, sub_id, now, app),
    )
    .context("cannot record the delivery attempt")?;
    Ok(())
}

/// Ends the attempt in flight for a delivery, if there is one.
pub fn close_attempt(
    tx: &Transaction,
    msg_seq: i64,
    sub_id: i64,
    now: Millis,
    outcome: &str,
    reason: &str,
) -> Result<()> {
    tx.execute(
        "UPDATE attempts
            SET ended_at = ?3, outcome = ?4, reason = ?5
          WHERE msg_seq = ?1 AND sub_id = ?2 AND outcome = 'in_flight'",
        (msg_seq, sub_id, now, outcome, reason),
    )
    .context("cannot close the delivery attempt")?;
    Ok(())
}

/// Ends every attempt in flight on one subscription (archiving it).
pub fn close_subscription_attempts(
    tx: &Transaction,
    sub_id: i64,
    now: Millis,
    outcome: &str,
    reason: &str,
) -> Result<()> {
    tx.execute(
        "UPDATE attempts
            SET ended_at = ?2, outcome = ?3, reason = ?4
          WHERE sub_id = ?1 AND outcome = 'in_flight'",
        (sub_id, now, outcome, reason),
    )
    .context("cannot close the subscription's attempts")?;
    Ok(())
}

/// Drops statistics past their caps and returns how many rows went. The
/// attempts are bounded by `limit` like every sweeper batch; the counters
/// are small enough to prune whole.
pub fn prune_history(tx: &Transaction, now: Millis, limit: usize) -> Result<usize> {
    let mut removed = 0;
    removed += tx
        .execute(
            "DELETE FROM attempts WHERE id IN (
                 SELECT id FROM attempts WHERE claimed_at < ?1 ORDER BY claimed_at LIMIT ?2)",
            (now - ATTEMPTS_KEPT_MS, limit as i64),
        )
        .context("cannot prune old attempts")?;
    removed += tx
        .execute(
            "DELETE FROM stats_hourly WHERE hour < ?1",
            [now - HOURLY_KEPT_MS],
        )
        .context("cannot prune hourly counters")?;
    removed += tx
        .execute(
            "DELETE FROM stats_daily WHERE day < ?1",
            [crate::engine::days::day_of(now) - DAILY_KEPT_DAYS],
        )
        .context("cannot prune daily counters")?;
    removed += tx
        .execute(
            "DELETE FROM stats_samples WHERE at < ?1",
            [now - SAMPLES_KEPT_MS],
        )
        .context("cannot prune samples")?;
    Ok(removed)
}

/// The attempts of one message on one subscription, oldest first.
pub fn attempts_for(
    conn: &rusqlite::Connection,
    sub_id: i64,
    message_id: &str,
) -> Result<Option<Vec<AttemptRow>>> {
    let seq: Option<i64> = conn
        .query_row(
            "SELECT d.msg_seq FROM deliveries d JOIN messages m ON m.seq = d.msg_seq
              WHERE m.id = ?1 AND d.sub_id = ?2",
            (message_id, sub_id),
            |row| row.get(0),
        )
        .optional()
        .context("cannot look up the delivery")?;
    let Some(seq) = seq else {
        return Ok(None);
    };
    let mut statement = conn
        .prepare(
            "SELECT n, claimed_at, ended_at, outcome, app, reason
               FROM attempts WHERE msg_seq = ?1 AND sub_id = ?2 ORDER BY id",
        )
        .context("cannot list the attempts")?;
    let rows = statement
        .query_map((seq, sub_id), |row| {
            Ok(AttemptRow {
                n: row.get(0)?,
                claimed_at: row.get(1)?,
                ended_at: row.get(2)?,
                outcome: row.get(3)?,
                app: row.get(4)?,
                reason: row.get(5)?,
            })
        })
        .context("cannot list the attempts")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("cannot read the attempts")?;
    Ok(Some(rows))
}
