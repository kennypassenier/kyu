//! Every SQL statement in the project lives here (AR1, AR3).
//!
//! Functions take a [`Transaction`] rather than opening one, so the engine
//! decides what is atomic: publishing a message and creating its delivery
//! rows must be one transaction, or a crash between them would leave a
//! message nobody is subscribed to (AR3).
//!
//! All statements are parameterized (AR11) — there is a commit gate that
//! refuses string-built SQL.

use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, Transaction};

use crate::engine::clock::Millis;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subscription {
    pub id: i64,
    pub name: String,
}

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

pub fn subscription_id_by_name(tx: &Transaction, topic_id: i64, name: &str) -> Result<Option<i64>> {
    tx.query_row(
        "SELECT id FROM subscriptions WHERE topic_id = ?1 AND name = ?2",
        (topic_id, name),
        |row| row.get(0),
    )
    .optional()
    .with_context(|| format!("cannot look up subscription {name:?}"))
}

pub fn create_subscription(
    tx: &Transaction,
    topic_id: i64,
    name: &str,
    now: Millis,
) -> Result<i64> {
    // Policy columns stay NULL: a fresh subscription runs on the defaults
    // until someone sets them (K7, L4).
    tx.execute(
        "INSERT INTO subscriptions (topic_id, name, state, created_at, last_poll_at)
         VALUES (?1, ?2, 'active', ?3, ?3)",
        (topic_id, name, now),
    )
    .with_context(|| format!("cannot create subscription {name:?}"))?;
    Ok(tx.last_insert_rowid())
}

pub fn subscription_names(tx: &Transaction, topic_id: i64) -> Result<Vec<String>> {
    let mut statement = tx
        .prepare("SELECT name FROM subscriptions WHERE topic_id = ?1 ORDER BY name")
        .context("cannot list subscriptions")?;
    let names = statement
        .query_map([topic_id], |row| row.get::<_, String>(0))
        .context("cannot list subscriptions")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("cannot read the subscription list")?;
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

pub fn touch_subscription_poll(tx: &Transaction, sub_id: i64, now: Millis) -> Result<()> {
    // Feeds the idle-subscription lifecycle (K11, L6) and the dashboard's
    // alive/idle column.
    tx.execute(
        "UPDATE subscriptions SET last_poll_at = ?2 WHERE id = ?1",
        (sub_id, now),
    )
    .context("cannot record the poll time")?;
    Ok(())
}

/// The subscriptions a new message must be delivered to. Archived ones are
/// excluded, which is what keeps AR3's retention rule safe.
pub fn active_subscriptions(tx: &Transaction, topic_id: i64) -> Result<Vec<Subscription>> {
    let mut statement = tx
        .prepare(
            "SELECT id, name FROM subscriptions
              WHERE topic_id = ?1 AND state IN ('active', 'flagged')
              ORDER BY id",
        )
        .context("cannot list active subscriptions")?;
    let subscriptions = statement
        .query_map([topic_id], |row| {
            Ok(Subscription {
                id: row.get(0)?,
                name: row.get(1)?,
            })
        })
        .context("cannot list active subscriptions")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("cannot read the subscription list")?;
    Ok(subscriptions)
}

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
