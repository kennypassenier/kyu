//! Failure paths a caller drives: nack (W5) and the dead-letter list (K6).

use crate::store::queries::{
    self, DeadLetter, DeleteOutcome, NackOutcome, Overdue, RequeueOutcome,
};

use super::clock::Millis;
use super::settle::{Cause, Settled, announce, apply, record_dead};
use super::{Engine, EngineError, Result};

/// The straight-to-dead path of a nack: no retry, whatever attempts remain.
fn nack_to_dead(
    tx: &rusqlite::Transaction,
    delivery: &Overdue,
    now: Millis,
) -> anyhow::Result<Settled> {
    let attempts = delivery.attempts + 1;
    if !queries::mark_dead(tx, delivery.msg_seq, delivery.sub_id, attempts, now)? {
        return Ok(Settled::Unchanged);
    }
    record_dead(tx, delivery, now, "nacked straight to the dead letters")?;
    Ok(Settled::DeadLettered)
}

impl Engine {
    /// W5. The consumer says it failed, instead of leaving the lease to run
    /// out. `dead` sends the message straight to the dead-letter list: a
    /// payload that can never work should not consume four more attempts.
    pub fn nack(
        &self,
        topic: &str,
        subscription: &str,
        message_id: &str,
        dead: bool,
    ) -> Result<Settled> {
        let now = self.clock.now_ms();

        self.write(|tx| -> Result<Settled> {
            let sub_id = self.resolve_subscription(tx, topic, subscription)?;

            match queries::nack_claimed(tx, message_id, sub_id)? {
                NackOutcome::NoSuchDelivery => {
                    return Err(EngineError::NoSuchDelivery {
                        id: message_id.to_string(),
                        subscription: subscription.to_string(),
                    });
                }
                NackOutcome::NotClaimed { state } => {
                    return Err(EngineError::NotClaimed {
                        id: message_id.to_string(),
                        subscription: subscription.to_string(),
                        state,
                    });
                }
                NackOutcome::Nacked => {}
            }

            let delivery = queries::delivery_of(tx, message_id, sub_id)?
                .ok_or_else(|| anyhow::anyhow!("the delivery vanished mid-transaction"))?;

            let settled = if dead {
                nack_to_dead(tx, &delivery, now)?
            } else {
                apply(tx, &delivery, now, Cause::Nack)?
            };
            // Same announcement as the sweeper makes for a lease that ran out.
            let mut ids = self.lock_ids();
            announce(tx, &mut ids, now, &delivery, settled)?;
            Ok(settled)
        })
    }

    /// K6. What is waiting in a subscription's dead-letter list.
    pub fn dead_letters(
        &self,
        topic: &str,
        subscription: &str,
        limit: usize,
    ) -> Result<Vec<DeadLetter>> {
        self.write(|tx| -> Result<Vec<DeadLetter>> {
            let sub_id = self.resolve_subscription(tx, topic, subscription)?;
            Ok(queries::dead_letters(tx, sub_id, limit)?)
        })
    }

    /// K6. Puts a dead letter back in the queue with a fresh set of
    /// attempts.
    pub fn requeue_dead(&self, topic: &str, subscription: &str, message_id: &str) -> Result<()> {
        self.write(|tx| -> Result<()> {
            let sub_id = self.resolve_subscription(tx, topic, subscription)?;
            match queries::requeue_dead(tx, message_id, sub_id)? {
                RequeueOutcome::Requeued => Ok(()),
                RequeueOutcome::NoSuchDelivery => Err(EngineError::NoSuchDelivery {
                    id: message_id.to_string(),
                    subscription: subscription.to_string(),
                }),
                RequeueOutcome::NotDead { state } => Err(EngineError::NotDead {
                    id: message_id.to_string(),
                    subscription: subscription.to_string(),
                    state,
                }),
            }
        })
    }

    /// [W15, W16] Removes one delivery for good, whatever state it is in —
    /// a dead letter someone decided is not worth requeuing (W15), or a
    /// still-pending or claimed item on the per-subscription backlog view
    /// (W16). Only this subscription's delivery goes; the message and
    /// every other subscription's copy of it are untouched.
    pub fn delete_delivery(&self, topic: &str, subscription: &str, message_id: &str) -> Result<()> {
        let now = self.clock.now_ms();
        self.write(|tx| -> Result<()> {
            let sub_id = self.resolve_subscription(tx, topic, subscription)?;
            match queries::delete_delivery(tx, message_id, sub_id, now)? {
                DeleteOutcome::Deleted => Ok(()),
                DeleteOutcome::NoSuchDelivery => Err(EngineError::NoSuchDelivery {
                    id: message_id.to_string(),
                    subscription: subscription.to_string(),
                }),
            }
        })
    }

    /// [K-actions] Removes every dead letter across every topic and
    /// subscription in one confirmed action, for the status page's "Prune
    /// every dead letter" button. Returns how many were removed.
    pub fn prune_dead_letters(&self) -> Result<usize> {
        self.write(|tx| -> Result<usize> { Ok(queries::prune_dead_letters(tx)?) })
    }
}
