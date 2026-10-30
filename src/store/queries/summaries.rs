//! Read models for the status page: topic and subscription summaries.
//!
//! Part of the SQL layer (AR1, AR3); statements are parameterized (AR11).

use anyhow::{Context, Result};

use super::StoredPolicy;
use super::rows::policy_from_row;
use crate::engine::clock::Millis;

#[derive(Debug, Clone)]
pub struct TopicSummary {
    pub name: String,
    pub retention_ms: Option<i64>,
    pub messages: i64,
    pub subscriptions: i64,
    pub backlog: i64,
    pub dead: i64,
    pub last_published_at: Option<Millis>,
}

#[derive(Debug, Clone)]
pub struct SubscriptionSummary {
    pub name: String,
    pub state: String,
    pub backlog: i64,
    pub claimed: i64,
    pub dead: i64,
    /// Settled when the subscription was archived (K11). AR3 calls for this
    /// to be visible; a lapsed backlog that nobody can see is the silence
    /// G8 forbids.
    pub lapsed: i64,
    pub last_poll_at: Option<Millis>,
    pub oldest_unacked_at: Option<Millis>,
    pub policy: StoredPolicy,
}

pub fn topic_summaries(conn: &rusqlite::Connection) -> Result<Vec<TopicSummary>> {
    let mut statement = conn
        .prepare(
            "SELECT t.name,
                    t.retention_ms,
                    (SELECT count(*) FROM messages m WHERE m.topic_id = t.id),
                    (SELECT count(*) FROM subscriptions s WHERE s.topic_id = t.id),
                    (SELECT count(*) FROM deliveries d
                       JOIN subscriptions s ON s.id = d.sub_id
                      WHERE s.topic_id = t.id AND d.state = 'pending'),
                    (SELECT count(*) FROM deliveries d
                       JOIN subscriptions s ON s.id = d.sub_id
                      WHERE s.topic_id = t.id AND d.state = 'dead'),
                    (SELECT max(m.published_at) FROM messages m WHERE m.topic_id = t.id)
               FROM topics t
              ORDER BY t.name",
        )
        .context("cannot summarise the topics")?;
    let rows = statement
        .query_map([], |row| {
            Ok(TopicSummary {
                name: row.get(0)?,
                retention_ms: row.get(1)?,
                messages: row.get(2)?,
                subscriptions: row.get(3)?,
                backlog: row.get(4)?,
                dead: row.get(5)?,
                last_published_at: row.get(6)?,
            })
        })
        .context("cannot summarise the topics")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("cannot read the topic summaries")?;
    Ok(rows)
}

pub fn subscription_summaries(
    conn: &rusqlite::Connection,
    topic_id: i64,
) -> Result<Vec<SubscriptionSummary>> {
    let mut statement = conn
        .prepare(
            "SELECT s.name, s.state,
                    (SELECT count(*) FROM deliveries d
                      WHERE d.sub_id = s.id AND d.state = 'pending'),
                    (SELECT count(*) FROM deliveries d
                      WHERE d.sub_id = s.id AND d.state = 'claimed'),
                    (SELECT count(*) FROM deliveries d
                      WHERE d.sub_id = s.id AND d.state = 'dead'),
                    (SELECT count(*) FROM deliveries d
                      WHERE d.sub_id = s.id AND d.state = 'lapsed'),
                    s.last_poll_at,
                    (SELECT min(m.published_at) FROM deliveries d
                       JOIN messages m ON m.seq = d.msg_seq
                      WHERE d.sub_id = s.id AND d.state IN ('pending', 'claimed')),
                    s.lease_ms, s.max_attempts, s.backoff_ms, s.ttl_ms,
                    s.idle_flag_ms, s.idle_archive_ms
               FROM subscriptions s
              WHERE s.topic_id = ?1
              ORDER BY s.name",
        )
        .context("cannot summarise the subscriptions")?;
    let rows = statement
        .query_map([topic_id], |row| {
            Ok(SubscriptionSummary {
                name: row.get(0)?,
                state: row.get(1)?,
                backlog: row.get(2)?,
                claimed: row.get(3)?,
                dead: row.get(4)?,
                lapsed: row.get(5)?,
                last_poll_at: row.get(6)?,
                oldest_unacked_at: row.get(7)?,
                policy: policy_from_row(row, 8)?,
            })
        })
        .context("cannot summarise the subscriptions")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("cannot read the subscription summaries")?;
    Ok(rows)
}
