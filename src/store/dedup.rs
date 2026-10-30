//! The SQL behind publish damping (schema 8): remembered dedup keys, their
//! cap and pruning, and the per-topic count of suppressed repeats.
//!
//! Kept apart from `queries.rs` so the feature lives in one place. Same
//! rules as there: every statement is parameterized, and functions take the
//! caller's [`Transaction`] so the engine decides what is atomic.

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, Transaction};

use crate::engine::clock::Millis;

/// Most remembered keys at once, across all topics. Past it a publish evicts
/// the oldest keys, which only means that a very old repeat may pass: damping
/// degrades towards today's behaviour, it never refuses a publish.
pub const MAX_ROWS: i64 = 10_000;

/// The message a still-open window points at, if any.
pub fn find_live(
    tx: &Transaction,
    topic_id: i64,
    key: &str,
    now: Millis,
) -> Result<Option<String>> {
    tx.query_row(
        "SELECT message_id FROM dedup_keys
          WHERE topic_id = ?1 AND key = ?2 AND expires_at > ?3",
        (topic_id, key, now),
        |row| row.get(0),
    )
    .optional()
    .context("cannot look up the dedup key")
}

/// Remembers `key` for `window_ms` (replacing an expired row of the same
/// key) and evicts the oldest rows past [`MAX_ROWS`].
pub fn remember(
    tx: &Transaction,
    topic_id: i64,
    key: &str,
    message_id: &str,
    now: Millis,
    window_ms: Millis,
) -> Result<()> {
    tx.execute(
        "INSERT INTO dedup_keys (topic_id, key, message_id, published_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (topic_id, key) DO UPDATE SET
             message_id = excluded.message_id,
             published_at = excluded.published_at,
             expires_at = excluded.expires_at",
        (
            topic_id,
            key,
            message_id,
            now,
            now.saturating_add(window_ms),
        ),
    )
    .context("cannot remember the dedup key")?;
    tx.execute(
        "DELETE FROM dedup_keys WHERE rowid IN (
             SELECT rowid FROM dedup_keys ORDER BY published_at ASC
              LIMIT max(0, (SELECT count(*) FROM dedup_keys) - ?1))",
        [MAX_ROWS],
    )
    .context("cannot enforce the dedup key cap")?;
    Ok(())
}

/// Counts one suppressed repeat on the topic's hourly statistics.
pub fn count_suppressed(tx: &Transaction, topic: &str, now: Millis) -> Result<()> {
    tx.execute(
        "INSERT INTO stats_hourly (topic, hour, published, dead, deduplicated)
         VALUES (?1, ?2, 0, 0, 1)
         ON CONFLICT (topic, hour) DO UPDATE SET deduplicated = deduplicated + 1",
        (topic, crate::engine::days::hour_of(now)),
    )
    .context("cannot count the suppressed duplicate")?;
    Ok(())
}

/// The sweeper's share: removes expired keys and keys of topics that are
/// gone, at most `limit` rows per kind. Returns how many rows went.
pub fn prune(tx: &Transaction, now: Millis, limit: usize) -> Result<usize> {
    let expired = tx
        .execute(
            "DELETE FROM dedup_keys WHERE rowid IN (
                 SELECT rowid FROM dedup_keys WHERE expires_at <= ?1 LIMIT ?2)",
            (now, limit as i64),
        )
        .context("cannot prune expired dedup keys")?;
    let orphaned = tx
        .execute(
            "DELETE FROM dedup_keys WHERE rowid IN (
                 SELECT rowid FROM dedup_keys
                  WHERE topic_id NOT IN (SELECT id FROM topics) LIMIT ?1)",
            [limit as i64],
        )
        .context("cannot prune dedup keys of deleted topics")?;
    Ok(expired + orphaned)
}

/// Suppressed repeats per topic over the retained hourly statistics, for
/// the metrics scrape.
pub fn suppressed_by_topic(conn: &Connection) -> Result<Vec<(String, i64)>> {
    let mut statement = conn
        .prepare(
            "SELECT topic, sum(deduplicated) FROM stats_hourly
              GROUP BY topic HAVING sum(deduplicated) > 0 ORDER BY topic",
        )
        .context("cannot read the suppressed counts")?;
    let rows = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .context("cannot read the suppressed counts")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("cannot read the suppressed counts")?;
    Ok(rows)
}

/// Keys currently remembered, for the metrics scrape.
pub fn live_keys(conn: &Connection) -> Result<i64> {
    conn.query_row("SELECT count(*) FROM dedup_keys", [], |row| row.get(0))
        .context("cannot count the dedup keys")
}
