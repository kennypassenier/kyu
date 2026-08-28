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

// ─── L4 · reliability semantics (K5, K6, K7, W5, AR9) ──────────────────────

/// A subscription's policy as stored. `None` means "use the default", which
/// is resolved in the engine so the default lives in exactly one place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StoredPolicy {
    pub lease_ms: Option<i64>,
    pub max_attempts: Option<i64>,
    pub backoff_ms: Option<i64>,
    pub ttl_ms: Option<i64>,
    /// K11 · how long this subscription may go unpolled before it is
    /// flagged, then archived. `None` follows the hub-wide default.
    pub idle_flag_ms: Option<i64>,
    pub idle_archive_ms: Option<i64>,
}

/// A claimed delivery whose lease ran out, with everything the engine needs
/// to decide where it goes next (AR9) without a second query per row.
#[derive(Debug, Clone)]
pub struct Overdue {
    pub msg_seq: i64,
    pub sub_id: i64,
    pub topic: String,
    pub subscription: String,
    pub attempts: i64,
    pub published_at: Millis,
    pub policy: StoredPolicy,
}

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
pub enum NackOutcome {
    Nacked,
    NoSuchDelivery,
    NotClaimed { state: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequeueOutcome {
    Requeued,
    NoSuchDelivery,
    NotDead { state: String },
}

pub fn subscription_policy(tx: &Transaction, sub_id: i64) -> Result<StoredPolicy> {
    tx.query_row(
        "SELECT lease_ms, max_attempts, backoff_ms, ttl_ms, idle_flag_ms, idle_archive_ms
           FROM subscriptions WHERE id = ?1",
        [sub_id],
        |row| {
            Ok(StoredPolicy {
                lease_ms: row.get(0)?,
                max_attempts: row.get(1)?,
                backoff_ms: row.get(2)?,
                ttl_ms: row.get(3)?,
                idle_flag_ms: row.get(4)?,
                idle_archive_ms: row.get(5)?,
            })
        },
    )
    .context("cannot read the subscription policy")
}

/// Replace semantics: every field is written, so an absent field in the
/// request means "back to the default" rather than "leave whatever was
/// there". One rule is easier to remember than two.
pub fn set_subscription_policy(tx: &Transaction, sub_id: i64, policy: StoredPolicy) -> Result<()> {
    tx.execute(
        "UPDATE subscriptions
            SET lease_ms = ?2, max_attempts = ?3, backoff_ms = ?4, ttl_ms = ?5,
                idle_flag_ms = ?6, idle_archive_ms = ?7
          WHERE id = ?1",
        (
            sub_id,
            policy.lease_ms,
            policy.max_attempts,
            policy.backoff_ms,
            policy.ttl_ms,
            policy.idle_flag_ms,
            policy.idle_archive_ms,
        ),
    )
    .context("cannot store the subscription policy")?;
    Ok(())
}

/// Claimed deliveries whose lease has run out. Bounded by `limit` (AR5):
/// the writer connection must never be held for a sweep over a huge table.
pub fn overdue_claims(tx: &Transaction, now: Millis, limit: usize) -> Result<Vec<Overdue>> {
    let mut statement = tx
        .prepare(
            "SELECT d.msg_seq, d.sub_id, t.name, s.name, d.attempts, m.published_at,
                    s.lease_ms, s.max_attempts, s.backoff_ms, s.ttl_ms,
                    s.idle_flag_ms, s.idle_archive_ms
               FROM deliveries d
               JOIN subscriptions s ON s.id = d.sub_id
               JOIN topics t ON t.id = s.topic_id
               JOIN messages m ON m.seq = d.msg_seq
              WHERE d.state = 'claimed' AND d.lease_expires_at <= ?1
              ORDER BY d.lease_expires_at
              LIMIT ?2",
        )
        .context("cannot scan for overdue claims")?;
    let rows = statement
        .query_map((now, limit as i64), |row| {
            Ok(Overdue {
                msg_seq: row.get(0)?,
                sub_id: row.get(1)?,
                topic: row.get(2)?,
                subscription: row.get(3)?,
                attempts: row.get(4)?,
                published_at: row.get(5)?,
                policy: StoredPolicy {
                    lease_ms: row.get(6)?,
                    max_attempts: row.get(7)?,
                    backoff_ms: row.get(8)?,
                    ttl_ms: row.get(9)?,
                    idle_flag_ms: row.get(10)?,
                    idle_archive_ms: row.get(11)?,
                },
            })
        })
        .context("cannot scan for overdue claims")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("cannot read the overdue claims")?;
    Ok(rows)
}

/// Pending deliveries whose message has outlived its subscription's TTL.
pub fn pending_past_ttl(tx: &Transaction, now: Millis, limit: usize) -> Result<Vec<Overdue>> {
    let mut statement = tx
        .prepare(
            "SELECT d.msg_seq, d.sub_id, t.name, s.name, d.attempts, m.published_at,
                    s.lease_ms, s.max_attempts, s.backoff_ms, s.ttl_ms,
                    s.idle_flag_ms, s.idle_archive_ms
               FROM deliveries d
               JOIN subscriptions s ON s.id = d.sub_id
               JOIN topics t ON t.id = s.topic_id
               JOIN messages m ON m.seq = d.msg_seq
              WHERE d.state = 'pending'
                AND s.ttl_ms IS NOT NULL
                AND m.published_at + s.ttl_ms <= ?1
              ORDER BY d.msg_seq
              LIMIT ?2",
        )
        .context("cannot scan for expired messages")?;
    let rows = statement
        .query_map((now, limit as i64), |row| {
            Ok(Overdue {
                msg_seq: row.get(0)?,
                sub_id: row.get(1)?,
                topic: row.get(2)?,
                subscription: row.get(3)?,
                attempts: row.get(4)?,
                published_at: row.get(5)?,
                policy: StoredPolicy {
                    lease_ms: row.get(6)?,
                    max_attempts: row.get(7)?,
                    backoff_ms: row.get(8)?,
                    ttl_ms: row.get(9)?,
                    idle_flag_ms: row.get(10)?,
                    idle_archive_ms: row.get(11)?,
                },
            })
        })
        .context("cannot scan for expired messages")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("cannot read the expired messages")?;
    Ok(rows)
}

/// `claimed -> pending` (AR9). State-guarded, so a delivery that was acked
/// between the scan and this update stays acked.
pub fn repend(
    tx: &Transaction,
    msg_seq: i64,
    sub_id: i64,
    attempts: i64,
    next_attempt_at: Millis,
) -> Result<bool> {
    let updated = tx
        .execute(
            "UPDATE deliveries
                SET state = 'pending', attempts = ?3, next_attempt_at = ?4,
                    lease_expires_at = NULL
              WHERE msg_seq = ?1 AND sub_id = ?2 AND state = 'claimed'",
            (msg_seq, sub_id, attempts, next_attempt_at),
        )
        .context("cannot return the delivery to pending")?;
    Ok(updated > 0)
}

/// `-> dead` (K6). Accepts both `claimed` (a poison pill nacked straight to
/// the dead list) and `pending` (retries exhausted).
pub fn mark_dead(
    tx: &Transaction,
    msg_seq: i64,
    sub_id: i64,
    attempts: i64,
    now: Millis,
) -> Result<bool> {
    let updated = tx
        .execute(
            "UPDATE deliveries
                SET state = 'dead', attempts = ?3, dead_at = ?4,
                    lease_expires_at = NULL, next_attempt_at = NULL
              WHERE msg_seq = ?1 AND sub_id = ?2 AND state IN ('claimed', 'pending')",
            (msg_seq, sub_id, attempts, now),
        )
        .context("cannot dead-letter the delivery")?;
    Ok(updated > 0)
}

/// `-> expired` (K7). A message that is past its TTL is settled, recorded
/// with the moment it lapsed rather than quietly deleted (G8).
pub fn mark_expired(tx: &Transaction, msg_seq: i64, sub_id: i64, now: Millis) -> Result<bool> {
    let updated = tx
        .execute(
            "UPDATE deliveries
                SET state = 'expired', expired_at = ?3,
                    lease_expires_at = NULL, next_attempt_at = NULL
              WHERE msg_seq = ?1 AND sub_id = ?2 AND state IN ('claimed', 'pending')",
            (msg_seq, sub_id, now),
        )
        .context("cannot expire the delivery")?;
    Ok(updated > 0)
}

/// W5 · `claimed -> pending` on the consumer's own say-so, without waiting
/// out the lease. Returns the attempt count so the engine can apply the
/// same dead-letter and TTL rules as a lease expiry.
pub fn nack_claimed(tx: &Transaction, message_id: &str, sub_id: i64) -> Result<NackOutcome> {
    let found: Option<(i64, i64, String)> = tx
        .query_row(
            "SELECT d.msg_seq, d.attempts, d.state
               FROM deliveries d
               JOIN messages m ON m.seq = d.msg_seq
              WHERE m.id = ?1 AND d.sub_id = ?2",
            (message_id, sub_id),
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .context("cannot inspect the delivery")?;

    let Some((_, _, state)) = found else {
        return Ok(NackOutcome::NoSuchDelivery);
    };
    if state != "claimed" {
        return Ok(NackOutcome::NotClaimed { state });
    }
    Ok(NackOutcome::Nacked)
}

/// The delivery row behind a message id, for a subscription.
pub fn delivery_of(tx: &Transaction, message_id: &str, sub_id: i64) -> Result<Option<Overdue>> {
    tx.query_row(
        "SELECT d.msg_seq, d.sub_id, t.name, s.name, d.attempts, m.published_at,
                s.lease_ms, s.max_attempts, s.backoff_ms, s.ttl_ms,
                s.idle_flag_ms, s.idle_archive_ms
           FROM deliveries d
           JOIN subscriptions s ON s.id = d.sub_id
           JOIN topics t ON t.id = s.topic_id
           JOIN messages m ON m.seq = d.msg_seq
          WHERE m.id = ?1 AND d.sub_id = ?2",
        (message_id, sub_id),
        |row| {
            Ok(Overdue {
                msg_seq: row.get(0)?,
                sub_id: row.get(1)?,
                topic: row.get(2)?,
                subscription: row.get(3)?,
                attempts: row.get(4)?,
                published_at: row.get(5)?,
                policy: StoredPolicy {
                    lease_ms: row.get(6)?,
                    max_attempts: row.get(7)?,
                    backoff_ms: row.get(8)?,
                    ttl_ms: row.get(9)?,
                    idle_flag_ms: row.get(10)?,
                    idle_archive_ms: row.get(11)?,
                },
            })
        },
    )
    .optional()
    .context("cannot look up the delivery")
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

// ─── L6 · history and lifecycle (K8, K9, K11, W11) ─────────────────────────

#[derive(Debug, Clone)]
pub struct SubscriptionRef {
    pub id: i64,
    pub topic: String,
    pub name: String,
    /// The threshold that actually applied, so the event can say which.
    pub idle_ms: i64,
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

/// Subscriptions idle longer than their own threshold, or the hub's default
/// when they have not set one.
fn idle_subscriptions(
    tx: &Transaction,
    sql: &'static str,
    now: Millis,
    default_ms: Millis,
    limit: usize,
) -> Result<Vec<SubscriptionRef>> {
    let mut statement = tx
        .prepare(sql)
        .context("cannot scan for idle subscriptions")?;
    let rows = statement
        .query_map((now, default_ms, limit as i64), |row| {
            Ok(SubscriptionRef {
                id: row.get(0)?,
                topic: row.get(1)?,
                name: row.get(2)?,
                idle_ms: row.get(3)?,
            })
        })
        .context("cannot scan for idle subscriptions")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("cannot read the idle subscriptions")?;
    Ok(rows)
}

/// K11 · active subscriptions nobody has polled for a while. `created_at`
/// stands in for a subscription that has never polled at all.
pub fn subscriptions_to_flag(
    tx: &Transaction,
    now: Millis,
    default_ms: Millis,
    limit: usize,
) -> Result<Vec<SubscriptionRef>> {
    idle_subscriptions(
        tx,
        "SELECT s.id, t.name, s.name, COALESCE(s.idle_flag_ms, ?2)
           FROM subscriptions s JOIN topics t ON t.id = s.topic_id
          WHERE s.state = 'active'
            AND COALESCE(s.last_poll_at, s.created_at) <= ?1 - COALESCE(s.idle_flag_ms, ?2)
          ORDER BY s.id LIMIT ?3",
        now,
        default_ms,
        limit,
    )
}

/// K11 · subscriptions idle long enough to stop accumulating messages.
pub fn subscriptions_to_archive(
    tx: &Transaction,
    now: Millis,
    default_ms: Millis,
    limit: usize,
) -> Result<Vec<SubscriptionRef>> {
    idle_subscriptions(
        tx,
        "SELECT s.id, t.name, s.name, COALESCE(s.idle_archive_ms, ?2)
           FROM subscriptions s JOIN topics t ON t.id = s.topic_id
          WHERE s.state IN ('active', 'flagged')
            AND COALESCE(s.last_poll_at, s.created_at) <= ?1 - COALESCE(s.idle_archive_ms, ?2)
          ORDER BY s.id LIMIT ?3",
        now,
        default_ms,
        limit,
    )
}

pub fn set_subscription_state(tx: &Transaction, sub_id: i64, state: &str) -> Result<()> {
    tx.execute(
        "UPDATE subscriptions SET state = ?2 WHERE id = ?1",
        (sub_id, state),
    )
    .context("cannot change the subscription state")?;
    Ok(())
}

pub fn subscription_state(tx: &Transaction, sub_id: i64) -> Result<String> {
    tx.query_row(
        "SELECT state FROM subscriptions WHERE id = ?1",
        [sub_id],
        |row| row.get(0),
    )
    .context("cannot read the subscription state")
}

/// K11 · settles what an archived subscription was still holding.
///
/// `lapsed` rather than silent deletion: the count is reported, the state is
/// visible, and the messages themselves stay on the topic until retention
/// collects them (G8).
pub fn lapse_outstanding(tx: &Transaction, sub_id: i64) -> Result<usize> {
    let lapsed = tx
        .execute(
            "UPDATE deliveries
                SET state = 'lapsed', lease_expires_at = NULL, next_attempt_at = NULL
              WHERE sub_id = ?1 AND state IN ('pending', 'claimed')",
            [sub_id],
        )
        .context("cannot lapse the outstanding deliveries")?;
    Ok(lapsed)
}

pub fn message_id_of(tx: &Transaction, msg_seq: i64) -> Result<String> {
    tx.query_row("SELECT id FROM messages WHERE seq = ?1", [msg_seq], |row| {
        row.get(0)
    })
    .context("cannot read the message id")
}

// ─── L7 · dashboard reads (K10) ────────────────────────────────────────────

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
    pub last_poll_at: Option<Millis>,
    pub oldest_unacked_at: Option<Millis>,
    pub policy: StoredPolicy,
}

#[derive(Debug, Clone)]
pub struct RecentMessage {
    pub id: String,
    pub published_at: Millis,
    pub due_at: Option<Millis>,
    pub content_type: Option<String>,
    pub payload: Vec<u8>,
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
                last_poll_at: row.get(5)?,
                oldest_unacked_at: row.get(6)?,
                policy: StoredPolicy {
                    lease_ms: row.get(7)?,
                    max_attempts: row.get(8)?,
                    backoff_ms: row.get(9)?,
                    ttl_ms: row.get(10)?,
                    idle_flag_ms: row.get(11)?,
                    idle_archive_ms: row.get(12)?,
                },
            })
        })
        .context("cannot summarise the subscriptions")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("cannot read the subscription summaries")?;
    Ok(rows)
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

pub fn topic_id_by_name_conn(conn: &rusqlite::Connection, name: &str) -> Result<Option<i64>> {
    conn.query_row("SELECT id FROM topics WHERE name = ?1", [name], |row| {
        row.get(0)
    })
    .optional()
    .with_context(|| format!("cannot look up topic {name:?}"))
}

// ─── L8 · metrics and backup (W1, W8) ──────────────────────────────────────

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
