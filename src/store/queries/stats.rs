//! Counters and samples: hourly and daily bumps, totals sampling, delivery counts and ad-hoc scalar queries.
//!
//! Part of the SQL layer (AR1, AR3); statements are parameterized (AR11).
use anyhow::{Context, Result};
use rusqlite::Transaction;

use super::SAMPLE_EVERY_MS;
use crate::engine::clock::Millis;

#[derive(Debug, Clone)]
pub struct DeliveryCount {
    pub topic: String,
    pub subscription: String,
    pub state: String,
    pub count: i64,
}

/// Every (topic, subscription, state) count in one pass, so the metrics
/// endpoint is one query rather than one per series.
pub fn delivery_counts(conn: &rusqlite::Connection) -> Result<Vec<DeliveryCount>> {
    let mut statement = conn
        .prepare(
            "SELECT t.name, s.name, d.state, count(*)
               FROM deliveries d
               JOIN subscriptions s ON s.id = d.sub_id
               JOIN topics t ON t.id = s.topic_id
              GROUP BY t.name, s.name, d.state
              ORDER BY t.name, s.name, d.state",
        )
        .context("cannot count the deliveries")?;
    let rows = statement
        .query_map([], |row| {
            Ok(DeliveryCount {
                topic: row.get(0)?,
                subscription: row.get(1)?,
                state: row.get(2)?,
                count: row.get(3)?,
            })
        })
        .context("cannot count the deliveries")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("cannot read the delivery counts")?;
    Ok(rows)
}

pub fn scalar(conn: &rusqlite::Connection, sql: &str) -> Result<i64> {
    conn.query_row(sql, [], |row| row.get(0))
        .with_context(|| format!("cannot run {sql}"))
}

pub fn bump_published(tx: &Transaction, topic: &str, now: Millis) -> Result<()> {
    tx.execute(
        "INSERT INTO stats_hourly (topic, hour, published, dead) VALUES (?1, ?2, 1, 0)
         ON CONFLICT (topic, hour) DO UPDATE SET published = published + 1",
        (topic, crate::engine::days::hour_of(now)),
    )
    .context("cannot count the published message")?;
    Ok(())
}

pub fn bump_dead(tx: &Transaction, topic: &str, subscription: &str, now: Millis) -> Result<()> {
    tx.execute(
        "INSERT INTO stats_hourly (topic, hour, published, dead) VALUES (?1, ?2, 0, 1)
         ON CONFLICT (topic, hour) DO UPDATE SET dead = dead + 1",
        (topic, crate::engine::days::hour_of(now)),
    )
    .context("cannot count the dead letter per hour")?;
    tx.execute(
        "INSERT INTO stats_daily (topic, subscription, day, delivered, dead) VALUES (?1, ?2, ?3, 0, 1)
         ON CONFLICT (topic, subscription, day) DO UPDATE SET dead = dead + 1",
        (topic, subscription, crate::engine::days::day_of(now)),
    )
    .context("cannot count the dead letter per day")?;
    Ok(())
}

pub fn bump_delivered(
    tx: &Transaction,
    topic: &str,
    subscription: &str,
    now: Millis,
) -> Result<()> {
    tx.execute(
        "INSERT INTO stats_daily (topic, subscription, day, delivered, dead) VALUES (?1, ?2, ?3, 1, 0)
         ON CONFLICT (topic, subscription, day) DO UPDATE SET delivered = delivered + 1",
        (topic, subscription, crate::engine::days::day_of(now)),
    )
    .context("cannot count the delivered message")?;
    Ok(())
}

/// Writes one hub-totals sample when the newest is older than
/// [`SAMPLE_EVERY_MS`]. Returns whether it wrote one.
pub fn sample_totals_if_due(tx: &Transaction, now: Millis) -> Result<bool> {
    let newest: Option<Millis> = tx
        .query_row("SELECT max(at) FROM stats_samples", [], |row| row.get(0))
        .context("cannot read the newest sample")?;
    if newest.is_some_and(|at| now - at < SAMPLE_EVERY_MS) {
        return Ok(false);
    }
    tx.execute(
        "INSERT OR REPLACE INTO stats_samples (at, messages, backlog, dead, in_flight)
         VALUES (?1,
                 (SELECT count(*) FROM messages),
                 (SELECT count(*) FROM deliveries WHERE state = 'pending'),
                 (SELECT count(*) FROM deliveries WHERE state = 'dead'),
                 (SELECT count(*) FROM deliveries WHERE state = 'claimed'))",
        [now],
    )
    .context("cannot sample the hub totals")?;
    Ok(true)
}

/// `(day, delivered, dead)` for a subscription between two day counts,
/// inclusive; days without activity are absent.
pub fn daily_for(
    conn: &rusqlite::Connection,
    topic: &str,
    subscription: &str,
    from_day: i64,
    to_day: i64,
) -> Result<Vec<(i64, i64, i64)>> {
    let mut statement = conn
        .prepare(
            "SELECT day, delivered, dead FROM stats_daily
              WHERE topic = ?1 AND subscription = ?2 AND day BETWEEN ?3 AND ?4
              ORDER BY day",
        )
        .context("cannot read the daily counters")?;
    let rows = statement
        .query_map((topic, subscription, from_day, to_day), |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .context("cannot read the daily counters")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("cannot read the daily counters")?;
    Ok(rows)
}

/// `(topic, hour, published, dead)` for every hour at or after `from_hour`.
pub fn hourly_since(
    conn: &rusqlite::Connection,
    from_hour: Millis,
) -> Result<Vec<(String, i64, i64, i64)>> {
    let mut statement = conn
        .prepare(
            "SELECT topic, hour, published, dead FROM stats_hourly WHERE hour >= ?1 ORDER BY hour",
        )
        .context("cannot read the hourly counters")?;
    let rows = statement
        .query_map([from_hour], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .context("cannot read the hourly counters")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("cannot read the hourly counters")?;
    Ok(rows)
}

/// One sampled set of hub totals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TotalsSample {
    pub at: Millis,
    pub messages: i64,
    pub backlog: i64,
    pub dead: i64,
    pub in_flight: i64,
}

/// The samples since `from`, oldest first.
pub fn samples_since(conn: &rusqlite::Connection, from: Millis) -> Result<Vec<TotalsSample>> {
    let mut statement = conn
        .prepare(
            "SELECT at, messages, backlog, dead, in_flight FROM stats_samples
              WHERE at >= ?1 ORDER BY at",
        )
        .context("cannot read the samples")?;
    let rows = statement
        .query_map([from], |row| {
            Ok(TotalsSample {
                at: row.get(0)?,
                messages: row.get(1)?,
                backlog: row.get(2)?,
                dead: row.get(3)?,
                in_flight: row.get(4)?,
            })
        })
        .context("cannot read the samples")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("cannot read the samples")?;
    Ok(rows)
}
