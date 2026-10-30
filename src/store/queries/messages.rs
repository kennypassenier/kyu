//! Message rows: insertion, id/sequence lookups, replay, recent-message listing and retention collection.
//!
//! Part of the SQL layer (AR1, AR3); statements are parameterized (AR11).
use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, Transaction};

use crate::engine::clock::Millis;

pub fn insert_message(
    tx: &Transaction,
    id: &str,
    topic_id: i64,
    payload: &[u8],
    content_type: Option<&str>,
    published_at: Millis,
    due_at: Option<Millis>,
) -> Result<i64> {
    tx.execute(
        "INSERT INTO messages (id, topic_id, payload, content_type, published_at, due_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        (id, topic_id, payload, content_type, published_at, due_at),
    )
    .context("cannot store the message")?;
    Ok(tx.last_insert_rowid())
}

/// K9 · deletes messages past their topic's retention — but only those no
/// *active* subscription still needs (AR3's "backlogs win" rule).
///
/// A message still pending or claimed on an active or flagged subscription
/// is never collected, however old: a consumer that has been offline for a
/// fortnight comes back to a complete backlog. Bounded by `limit` (AR5);
/// delivery rows follow through the foreign key's cascade.
pub fn collect_retained(
    tx: &Transaction,
    now: Millis,
    default_retention_ms: Option<i64>,
    limit: usize,
) -> Result<usize> {
    let deleted = tx
        .execute(
            "DELETE FROM messages
              WHERE seq IN (
                    SELECT m.seq
                      FROM messages m
                      JOIN topics t ON t.id = m.topic_id
                     WHERE COALESCE(t.retention_ms, ?2) IS NOT NULL
                       AND m.published_at + COALESCE(t.retention_ms, ?2) <= ?1
                       AND NOT EXISTS (
                             SELECT 1
                               FROM deliveries d
                               JOIN subscriptions s ON s.id = d.sub_id
                              WHERE d.msg_seq = m.seq
                                AND d.state IN ('pending', 'claimed')
                                AND s.state IN ('active', 'flagged'))
                     ORDER BY m.seq
                     LIMIT ?3)",
            (now, default_retention_ms, limit as i64),
        )
        .context("cannot collect retained messages")?;
    Ok(deleted)
}

pub fn message_id_of(tx: &Transaction, msg_seq: i64) -> Result<String> {
    tx.query_row("SELECT id FROM messages WHERE seq = ?1", [msg_seq], |row| {
        row.get(0)
    })
    .context("cannot read the message id")
}

#[derive(Debug, Clone)]
pub struct RecentMessage {
    pub id: String,
    pub published_at: Millis,
    pub due_at: Option<Millis>,
    pub content_type: Option<String>,
    pub payload: Vec<u8>,
}

pub fn recent_messages(
    conn: &rusqlite::Connection,
    topic_id: i64,
    limit: usize,
) -> Result<Vec<RecentMessage>> {
    let mut statement = conn
        .prepare(
            "SELECT id, published_at, due_at, content_type, payload
               FROM messages
              WHERE topic_id = ?1
              ORDER BY seq DESC
              LIMIT ?2",
        )
        .context("cannot list the recent messages")?;
    let rows = statement
        .query_map((topic_id, limit as i64), |row| {
            Ok(RecentMessage {
                id: row.get(0)?,
                published_at: row.get(1)?,
                due_at: row.get(2)?,
                content_type: row.get(3)?,
                payload: row.get(4)?,
            })
        })
        .context("cannot list the recent messages")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("cannot read the recent messages")?;
    Ok(rows)
}

pub fn message_seq_of_id(tx: &Transaction, message_id: &str) -> Result<Option<i64>> {
    tx.query_row(
        "SELECT seq FROM messages WHERE id = ?1",
        [message_id],
        |row| row.get(0),
    )
    .optional()
    .context("cannot look up the message")
}

/// A stored message's payload and content type, for replay.
pub fn message_for_replay(
    conn: &rusqlite::Connection,
    topic_id: i64,
    message_id: &str,
) -> Result<Option<(Vec<u8>, Option<String>)>> {
    conn.query_row(
        "SELECT payload, content_type FROM messages WHERE id = ?1 AND topic_id = ?2",
        (message_id, topic_id),
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
    .context("cannot read the message to replay")
}
