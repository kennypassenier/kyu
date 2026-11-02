//! The read-only look a long poll takes before it asks for the writer.
//!
//! Part of the SQL layer (AR1, AR3); statements are parameterized (AR11).
use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension};

use crate::engine::clock::Millis;

/// True when `subscription` exists on `topic`, is plainly `active`, and has
/// nothing deliverable at `now`: a poll like that changes nothing but its
/// own timestamp. False for everything else (unknown, flagged or archived,
/// or a message waiting), which the claiming path handles.
pub fn is_idle_and_active(
    conn: &Connection,
    topic: &str,
    subscription: &str,
    now: Millis,
) -> Result<bool> {
    let idle: Option<bool> = conn
        .query_row(
            "SELECT s.state = 'active' AND NOT EXISTS (
                    SELECT 1
                      FROM deliveries d
                      JOIN messages m ON m.seq = d.msg_seq
                     WHERE d.sub_id = s.id
                       AND d.state = 'pending'
                       AND (d.next_attempt_at IS NULL OR d.next_attempt_at <= ?3)
                       AND (m.due_at IS NULL OR m.due_at <= ?3))
               FROM subscriptions s
               JOIN topics t ON t.id = s.topic_id
              WHERE t.name = ?1 AND s.name = ?2",
            (topic, subscription, now),
            |row| row.get(0),
        )
        .optional()
        .context("cannot look for a claimable message")?;
    Ok(idle.unwrap_or(false))
}
