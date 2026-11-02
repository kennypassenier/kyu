//! The one place a failed delivery's fate is decided (AR9).

use crate::events::{self, Event};
use crate::store::queries::{self, Overdue};

use super::clock::Millis;
use super::ids::MessageIds;
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
        return expire(tx, delivery, now);
    }
    if attempts >= policy.max_attempts {
        return dead_letter(tx, delivery, attempts, now, cause);
    }
    let next_attempt_at = now.saturating_add(policy.retry_delay_ms(attempts));
    retry(tx, delivery, attempts, next_attempt_at, now, cause)
}

/// `Unchanged` when the guarded update found nothing to change.
fn settled_if(changed: bool, settled: Settled) -> Settled {
    if changed { settled } else { Settled::Unchanged }
}

fn expire(tx: &rusqlite::Transaction, delivery: &Overdue, now: Millis) -> anyhow::Result<Settled> {
    let changed = queries::mark_expired(tx, delivery.msg_seq, delivery.sub_id, now)?;
    if changed {
        queries::close_attempt(
            tx,
            delivery.msg_seq,
            delivery.sub_id,
            now,
            "expired",
            "the message passed its time to live",
        )?;
    }
    Ok(settled_if(changed, Settled::Expired))
}

fn dead_letter(
    tx: &rusqlite::Transaction,
    delivery: &Overdue,
    attempts: i64,
    now: Millis,
    cause: Cause,
) -> anyhow::Result<Settled> {
    let changed = queries::mark_dead(tx, delivery.msg_seq, delivery.sub_id, attempts, now)?;
    if changed {
        record_dead(tx, delivery, now, cause.exhausted())?;
    }
    Ok(settled_if(changed, Settled::DeadLettered))
}

fn retry(
    tx: &rusqlite::Transaction,
    delivery: &Overdue,
    attempts: i64,
    next_attempt_at: Millis,
    now: Millis,
    cause: Cause,
) -> anyhow::Result<Settled> {
    let changed = queries::repend(
        tx,
        delivery.msg_seq,
        delivery.sub_id,
        attempts,
        next_attempt_at,
    )?;
    if changed {
        let (outcome, reason) = cause.retry();
        queries::close_attempt(tx, delivery.msg_seq, delivery.sub_id, now, outcome, reason)?;
    }
    Ok(settled_if(changed, Settled::Redelivered))
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

/// Announces how a failed delivery ended, the same way for every path that
/// settles one (lease expiry in the sweep, a consumer's nack): a dead letter
/// becomes a `message.dead_lettered` event, an expiry is counted towards the
/// next `message.expired` announcement. Returns the subscriptions of the
/// events topic that gained a message.
pub(super) fn announce(
    tx: &rusqlite::Transaction,
    ids: &mut MessageIds,
    now: Millis,
    delivery: &Overdue,
    settled: Settled,
) -> anyhow::Result<Vec<String>> {
    match settled {
        Settled::DeadLettered => events::emit(
            tx,
            ids,
            now,
            &Event::DeadLettered {
                topic: delivery.topic.clone(),
                subscription: delivery.subscription.clone(),
                message_id: queries::message_id_of(tx, delivery.msg_seq)?,
                attempts: delivery.attempts + 1,
            },
        ),
        Settled::Expired => {
            queries::note_expiry(tx, delivery.sub_id, 1)?;
            Ok(Vec::new())
        }
        Settled::Redelivered | Settled::Unchanged => Ok(Vec::new()),
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
