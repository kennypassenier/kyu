//! Claiming and acking: K2 poll, K3 ack.

use crate::store::queries::{self, AckOutcome};

use super::policy::Policy;
use super::{Claimed, Engine, EngineError, NewSubscription, Received, Result, names};

/// Rows per transaction while replaying a topic into a subscription (AR5).
const BACKFILL_BATCH: usize = 500;

impl Engine {
    /// K2. Claims the oldest deliverable message for `subscription`,
    /// creating that subscription on first use.
    ///
    /// A brand-new subscription sees messages published from now on (G7):
    /// delivery rows are created at publish time, so there simply are none
    /// for earlier messages. `from=beginning` needs the backfill of L6 and
    /// is refused until then rather than quietly ignored.
    pub fn claim_next(
        &self,
        topic: &str,
        subscription: &str,
        from_beginning: bool,
    ) -> Result<Received> {
        self.claim_next_as(topic, subscription, from_beginning, None)
    }

    /// [`Self::claim_next`] that also records which app polled, for the
    /// attempt history the dashboard shows. `None` for a caller that is not
    /// a registered app (the login token, an open test hub).
    pub fn claim_next_as(
        &self,
        topic: &str,
        subscription: &str,
        from_beginning: bool,
        app: Option<&str>,
    ) -> Result<Received> {
        if !names::is_valid(topic) {
            return Err(EngineError::InvalidName {
                kind: "topic",
                name: topic.to_string(),
            });
        }
        if !names::is_valid(subscription) {
            return Err(EngineError::InvalidName {
                kind: "subscription",
                name: subscription.to_string(),
            });
        }
        let now = self.clock.now_ms();

        self.write(|tx| -> Result<Received> {
            let Some(topic_id) = queries::topic_id_by_name(tx, topic)? else {
                // Auto-creating a topic here would turn a typo into a
                // subscription that waits forever on a topic nobody
                // publishes to. Naming the existing topics is far kinder.
                return Err(EngineError::UnknownTopic {
                    topic: topic.to_string(),
                    existing: queries::topic_names(tx)?,
                });
            };

            let (sub_id, created) =
                match queries::subscription_id_by_name(tx, topic_id, subscription)? {
                    Some(id) => (id, None),
                    None => {
                        let id = queries::create_subscription(tx, topic_id, subscription, now)?;
                        let created = NewSubscription {
                            name: subscription.to_string(),
                            retained_before: queries::count_messages(tx, topic_id)?,
                        };
                        (id, Some(created))
                    }
                };
            // An archived subscription is not quietly revived by a poll: its
            // backlog was lapsed when it was archived, and whoever comes back
            // needs to know that before carrying on (G8, K11).
            let state = queries::subscription_state(tx, sub_id)?;
            if state == "archived" {
                return Err(EngineError::SubscriptionArchived {
                    topic: topic.to_string(),
                    subscription: subscription.to_string(),
                });
            }
            // Polling is the definition of not-idle, so a flag clears itself.
            if state == "flagged" {
                queries::set_subscription_state(tx, sub_id, "active")?;
            }

            queries::touch_subscription_poll(tx, sub_id, now)?;

            // K8 · replay. Idempotent and batched: it hands this subscription
            // a delivery row for every retained message it does not have, so
            // asking twice cannot duplicate anything.
            let mut backfilled = 0;
            if from_beginning {
                loop {
                    let inserted =
                        queries::backfill_deliveries(tx, topic_id, sub_id, BACKFILL_BATCH)?;
                    backfilled += inserted;
                    if inserted < BACKFILL_BATCH {
                        break;
                    }
                }
                if backfilled > 0 {
                    tracing::info!(
                        topic,
                        subscription,
                        backfilled,
                        "replayed retained messages into a subscription"
                    );
                }
            }

            let policy = Policy::effective(queries::subscription_policy(tx, sub_id)?);
            let claimed = queries::claim_next(tx, sub_id, now, policy.lease_ms)?;
            if let Some(message) = &claimed {
                queries::open_attempt(tx, message.seq, sub_id, now, app)?;
            }

            Ok(Received {
                claimed: claimed.map(|message| Claimed {
                    topic: topic.to_string(),
                    message,
                }),
                created,
                backfilled,
            })
        })
    }

    /// K3. Settles a message for one subscription only — an ack speaks for
    /// the consumer that sends it and for nobody else (G3).
    pub fn ack(&self, topic: &str, subscription: &str, message_id: &str) -> Result<()> {
        let now = self.clock.now_ms();
        if !names::is_valid(topic) {
            return Err(EngineError::InvalidName {
                kind: "topic",
                name: topic.to_string(),
            });
        }
        if !names::is_valid(subscription) {
            return Err(EngineError::InvalidName {
                kind: "subscription",
                name: subscription.to_string(),
            });
        }

        self.write(|tx| -> Result<()> {
            let Some(topic_id) = queries::topic_id_by_name(tx, topic)? else {
                return Err(EngineError::UnknownTopic {
                    topic: topic.to_string(),
                    existing: queries::topic_names(tx)?,
                });
            };
            let Some(sub_id) = queries::subscription_id_by_name(tx, topic_id, subscription)? else {
                return Err(EngineError::UnknownSubscription {
                    topic: topic.to_string(),
                    subscription: subscription.to_string(),
                    existing: queries::subscription_names(tx, topic_id)?,
                });
            };

            match queries::ack(tx, message_id, sub_id)? {
                AckOutcome::Acked => {
                    if let Some(msg_seq) = queries::message_seq_of_id(tx, message_id)? {
                        queries::close_attempt(tx, msg_seq, sub_id, now, "acked", "")?;
                    }
                    queries::bump_delivered(tx, topic, subscription, now)?;
                    Ok(())
                }
                AckOutcome::NoSuchDelivery => Err(EngineError::NoSuchDelivery {
                    id: message_id.to_string(),
                    subscription: subscription.to_string(),
                }),
                AckOutcome::AlreadyAcked => Err(EngineError::AlreadyAcked {
                    id: message_id.to_string(),
                    subscription: subscription.to_string(),
                }),
                AckOutcome::NotClaimed { state } => Err(EngineError::NotClaimed {
                    id: message_id.to_string(),
                    subscription: subscription.to_string(),
                    state,
                }),
            }
        })
    }
}
