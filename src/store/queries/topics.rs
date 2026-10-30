//! Topic rows: lookup by name, creation, listing, message counts and per-topic retention.
//!
//! Part of the SQL layer (AR1, AR3); statements are parameterized (AR11).
use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, Transaction};

use crate::engine::clock::Millis;

pub fn topic_id_by_name(tx: &Transaction, name: &str) -> Result<Option<i64>> {
    tx.query_row("SELECT id FROM topics WHERE name = ?1", [name], |row| {
        row.get(0)
    })
    .optional()
    .with_context(|| format!("cannot look up topic {name:?}"))
}

pub fn create_topic(tx: &Transaction, name: &str, now: Millis) -> Result<i64> {
    tx.execute(
        "INSERT INTO topics (name, retention_ms, created_at) VALUES (?1, NULL, ?2)",
        (name, now),
    )
    .with_context(|| format!("cannot create topic {name:?}"))?;
    Ok(tx.last_insert_rowid())
}

pub fn topic_names(tx: &Transaction) -> Result<Vec<String>> {
    let mut statement = tx
        .prepare("SELECT name FROM topics ORDER BY name")
        .context("cannot list topics")?;
    let names = statement
        .query_map([], |row| row.get::<_, String>(0))
        .context("cannot list topics")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("cannot read the topic list")?;
    Ok(names)
}

/// How many messages the topic already holds. Used to explain a fresh
/// subscription's empty first poll instead of leaving it a silent 204 (G8).
pub fn count_messages(tx: &Transaction, topic_id: i64) -> Result<i64> {
    tx.query_row(
        "SELECT count(*) FROM messages WHERE topic_id = ?1",
        [topic_id],
        |row| row.get(0),
    )
    .context("cannot count the messages on this topic")
}

pub fn set_topic_retention(
    tx: &Transaction,
    topic_id: i64,
    retention_ms: Option<i64>,
) -> Result<()> {
    tx.execute(
        "UPDATE topics SET retention_ms = ?2 WHERE id = ?1",
        (topic_id, retention_ms),
    )
    .context("cannot store the topic retention")?;
    Ok(())
}

pub fn topic_retention(tx: &Transaction, topic_id: i64) -> Result<Option<i64>> {
    tx.query_row(
        "SELECT retention_ms FROM topics WHERE id = ?1",
        [topic_id],
        |row| row.get(0),
    )
    .context("cannot read the topic retention")
}

pub fn topic_id_by_name_conn(conn: &rusqlite::Connection, name: &str) -> Result<Option<i64>> {
    conn.query_row("SELECT id FROM topics WHERE name = ?1", [name], |row| {
        row.get(0)
    })
    .optional()
    .with_context(|| format!("cannot look up topic {name:?}"))
}

pub fn topic_names_conn(conn: &rusqlite::Connection) -> Result<Vec<String>> {
    let mut statement = conn
        .prepare("SELECT name FROM topics ORDER BY name")
        .context("cannot list topics")?;
    let names = statement
        .query_map([], |row| row.get::<_, String>(0))
        .context("cannot list topics")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("cannot read the topic list")?;
    Ok(names)
}
