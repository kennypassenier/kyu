//! Dead letters and backlogs: listing, requeueing, pruning, per topic and per subscription.
//!
//! Part of the SQL layer (AR1, AR3); statements are parameterized (AR11).
use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, Transaction};

use crate::engine::clock::Millis;

#[derive(Debug, Clone)]
pub struct DeadLetter {
    pub id: String,
    pub published_at: Millis,
    pub dead_at: Option<Millis>,
    pub attempts: i64,
    pub content_type: Option<String>,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequeueOutcome {
    Requeued,
    NoSuchDelivery,
    NotDead { state: String },
}

pub fn dead_letters(tx: &Transaction, sub_id: i64, limit: usize) -> Result<Vec<DeadLetter>> {
    let mut statement = tx
        .prepare(
            "SELECT m.id, m.published_at, d.dead_at, d.attempts, m.content_type, m.payload
               FROM deliveries d
               JOIN messages m ON m.seq = d.msg_seq
              WHERE d.sub_id = ?1 AND d.state = 'dead'
              ORDER BY d.dead_at, m.seq
              LIMIT ?2",
        )
        .context("cannot list the dead letters")?;
    let rows = statement
        .query_map((sub_id, limit as i64), |row| {
            Ok(DeadLetter {
                id: row.get(0)?,
                published_at: row.get(1)?,
                dead_at: row.get(2)?,
                attempts: row.get(3)?,
                content_type: row.get(4)?,
                payload: row.get(5)?,
            })
        })
        .context("cannot list the dead letters")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("cannot read the dead letters")?;
    Ok(rows)
}

/// `dead -> pending` with the attempt count reset (AR9): a requeued message
/// gets a full set of retries, because the reason it failed has usually been
/// fixed by hand in between.
pub fn requeue_dead(tx: &Transaction, message_id: &str, sub_id: i64) -> Result<RequeueOutcome> {
    let updated = tx
        .execute(
            "UPDATE deliveries
                SET state = 'pending', attempts = 0, dead_at = NULL,
                    next_attempt_at = NULL, lease_expires_at = NULL
              WHERE sub_id = ?2
                AND state = 'dead'
                AND msg_seq = (SELECT seq FROM messages WHERE id = ?1)",
            (message_id, sub_id),
        )
        .context("cannot requeue the dead letter")?;

    if updated > 0 {
        return Ok(RequeueOutcome::Requeued);
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

    Ok(match state {
        None => RequeueOutcome::NoSuchDelivery,
        Some(state) => RequeueOutcome::NotDead { state },
    })
}

/// [K-actions] Removes every dead letter on the hub, across every topic and
/// subscription, in one statement (as `removed` rows, for the same reason
/// as `delete_delivery`). The status-page counterpart to
/// `delete_delivery`'s one-at-a-time removal (W15): after a storm of
/// failures whose cause is fixed, clearing the backlog of them one topic
/// page at a time does not scale.
pub fn prune_dead_letters(tx: &Transaction) -> Result<usize> {
    let deleted = tx
        .execute(
            "UPDATE deliveries SET state = 'removed' WHERE state = 'dead'",
            [],
        )
        .context("cannot prune the dead letters")?;
    Ok(deleted)
}

/// A dead letter with the subscription it belongs to, for the dashboard's
/// dead-letter section (K6, K10).
#[derive(Debug, Clone)]
pub struct TopicDeadLetter {
    pub subscription: String,
    pub letter: DeadLetter,
}

pub fn dead_letters_for_topic(
    conn: &rusqlite::Connection,
    topic_id: i64,
    limit: usize,
) -> Result<Vec<TopicDeadLetter>> {
    let mut statement = conn
        .prepare(
            "SELECT s.name, m.id, m.published_at, d.dead_at, d.attempts, m.content_type, m.payload
               FROM deliveries d
               JOIN subscriptions s ON s.id = d.sub_id
               JOIN messages m ON m.seq = d.msg_seq
              WHERE s.topic_id = ?1 AND d.state = 'dead'
              ORDER BY d.dead_at DESC, m.seq DESC
              LIMIT ?2",
        )
        .context("cannot list the topic's dead letters")?;
    let rows = statement
        .query_map((topic_id, limit as i64), |row| {
            Ok(TopicDeadLetter {
                subscription: row.get(0)?,
                letter: DeadLetter {
                    id: row.get(1)?,
                    published_at: row.get(2)?,
                    dead_at: row.get(3)?,
                    attempts: row.get(4)?,
                    content_type: row.get(5)?,
                    payload: row.get(6)?,
                },
            })
        })
        .context("cannot list the topic's dead letters")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("cannot read the topic's dead letters")?;
    Ok(rows)
}

/// [W16] One item on a subscription's live backlog: still pending, or
/// already claimed by a consumer. Everything a dead letter shows except
/// `dead_at`, since it has none yet.
#[derive(Debug, Clone)]
pub struct BacklogItem {
    pub id: String,
    pub state: String,
    pub published_at: Millis,
    pub attempts: i64,
    pub content_type: Option<String>,
    pub payload: Vec<u8>,
}

/// [W16] What a subscription is still holding, oldest first — the same
/// question "Subscriptions" answers with a count (`backlog`, `claimed`),
/// individually. Bounded for the same reason `dead_letters_for_topic` is:
/// a broken consumer can leave a great many, and a page that will not load
/// is no use on the night you need it.
pub fn backlog_for_subscription(
    conn: &rusqlite::Connection,
    sub_id: i64,
    limit: usize,
) -> Result<Vec<BacklogItem>> {
    let mut statement = conn
        .prepare(
            "SELECT m.id, d.state, m.published_at, d.attempts, m.content_type, m.payload
               FROM deliveries d
               JOIN messages m ON m.seq = d.msg_seq
              WHERE d.sub_id = ?1 AND d.state IN ('pending', 'claimed')
              ORDER BY m.seq
              LIMIT ?2",
        )
        .context("cannot list the subscription's backlog")?;
    let rows = statement
        .query_map((sub_id, limit as i64), |row| {
            Ok(BacklogItem {
                id: row.get(0)?,
                state: row.get(1)?,
                published_at: row.get(2)?,
                attempts: row.get(3)?,
                content_type: row.get(4)?,
                payload: row.get(5)?,
            })
        })
        .context("cannot list the subscription's backlog")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("cannot read the subscription's backlog")?;
    Ok(rows)
}
