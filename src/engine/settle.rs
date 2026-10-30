//! The one place a failed delivery's fate is decided (AR9).

use crate::store::queries::{self, Overdue};

use super::clock::Millis;
use super::policy::Policy;

/// Where a delivery ended up after failing (AR9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settled {
    Redelivered,
    DeadLettered,
    Expired,
    /// Someone acked it between the scan and the update; the guard held.
    Unchanged,
}

/// The one place a failed delivery's fate is decided (AR9), shared by lease
/// expiry and nack so the two can never diverge.
pub(super) fn apply(
    tx: &rusqlite::Transaction,
    delivery: &Overdue,
    now: Millis,
    cause: Cause,
) -> anyhow::Result<Settled> {
    let policy = Policy::effective(delivery.policy);
    let attempts = delivery.attempts + 1;

    // TTL is re-checked here, not only while a message waits: a consumer
    // that claims a message and then hangs for half an hour must not be
    // able to resurrect it past its deadline (AR9, the stale-doorbell case).
    if policy.is_past_ttl(delivery.published_at, now) {
        return Ok(
            if queries::mark_expired(tx, delivery.msg_seq, delivery.sub_id, now)? {
                queries::close_attempt(
                    tx,
                    delivery.msg_seq,
                    delivery.sub_id,
                    now,
                    "expired",
                    "the message passed its time to live",
                )?;
                Settled::Expired
            } else {
                Settled::Unchanged
            },
        );
    }

    if attempts >= policy.max_attempts {
        return Ok(
            if queries::mark_dead(tx, delivery.msg_seq, delivery.sub_id, attempts, now)? {
                record_dead(tx, delivery, now, cause.exhausted())?;
                Settled::DeadLettered
            } else {
                Settled::Unchanged
            },
        );
    }

    let next_attempt_at = now.saturating_add(policy.retry_delay_ms(attempts));
    Ok(
        if queries::repend(
            tx,
            delivery.msg_seq,
            delivery.sub_id,
            attempts,
            next_attempt_at,
        )? {
            let (outcome, reason) = cause.retry();
            queries::close_attempt(tx, delivery.msg_seq, delivery.sub_id, now, outcome, reason)?;
            Settled::Redelivered
        } else {
            Settled::Unchanged
        },
    )
}

/// Why a delivery failed, for the attempt history.
#[derive(Debug, Clone, Copy)]
pub(super) enum Cause {
    Nack,
    LeaseExpired,
}

impl Cause {
    /// The attempt's outcome and reason when another try follows.
    pub(super) fn retry(self) -> (&'static str, &'static str) {
        match self {
            Self::Nack => ("nacked", "the consumer nacked the message"),
            Self::LeaseExpired => ("expired", "the lease ran out before an ack"),
        }
    }

    pub(super) fn exhausted(self) -> &'static str {
        match self {
            Self::Nack => "nacked on the last allowed attempt",
            Self::LeaseExpired => "the lease ran out on the last allowed attempt",
        }
    }
}

/// A delivery just became a dead letter: end its attempt and count it.
pub(super) fn record_dead(
    tx: &rusqlite::Transaction,
    delivery: &Overdue,
    now: Millis,
    reason: &str,
) -> anyhow::Result<()> {
    queries::close_attempt(tx, delivery.msg_seq, delivery.sub_id, now, "dead", reason)?;
    queries::bump_dead(tx, &delivery.topic, &delivery.subscription, now)?;
    Ok(())
}
