//! Subscription lifecycle (K11), retention (K9) and policy (K7).

use crate::events::{self, Event};
use crate::store::queries::{self, StoredPolicy};

use super::clock::Millis;
use super::policy::{self, Policy};
use super::{Engine, EngineError, Result};
impl Engine {
    /// K11 · brings an archived subscription back. Deliberately explicit:
    /// a poll will not do it, so nobody resumes without seeing that the
    /// backlog was lapsed.
    pub fn unarchive(&self, topic: &str, subscription: &str) -> Result<bool> {
        let now = self.clock.now_ms();
        self.write(|tx| -> Result<bool> {
            let sub_id = self.resolve_subscription(tx, topic, subscription)?;

            // Idempotent, but not noisy with it: unarchiving something that
            // was never archived changes nothing, so it announces nothing.
            // A spurious subscription.unarchived event would wake whatever
            // HA automation listens to kyu.events for no reason.
            if queries::subscription_state(tx, sub_id)? != "archived" {
                return Ok(false);
            }

            queries::set_subscription_state(tx, sub_id, "active")?;
            queries::touch_subscription_poll(tx, sub_id, now)?;
            let mut ids = self.ids.lock().expect("the id lock is never poisoned");
            events::emit(
                tx,
                &mut ids,
                now,
                &Event::SubscriptionUnarchived {
                    topic: topic.to_string(),
                    subscription: subscription.to_string(),
                },
            )?;
            Ok(true)
        })
    }

    /// Archives a subscription by hand (Kenny, 2026-09-30): the same state
    /// and the same `subscription.archived` event the idle sweep produces,
    /// with its backlog settled as lapsed. Returns `None` when it already
    /// was archived, so a second click announces nothing.
    pub fn archive(&self, topic: &str, subscription: &str) -> Result<Option<usize>> {
        let now = self.clock.now_ms();
        self.write(|tx| -> Result<Option<usize>> {
            let sub_id = self.resolve_subscription(tx, topic, subscription)?;
            if queries::subscription_state(tx, sub_id)? == "archived" {
                return Ok(None);
            }
            queries::set_subscription_state(tx, sub_id, "archived")?;
            let lapsed = queries::lapse_outstanding(tx, sub_id, now)?;
            let mut ids = self.ids.lock().expect("the id lock is never poisoned");
            events::emit(
                tx,
                &mut ids,
                now,
                &Event::SubscriptionArchived {
                    topic: topic.to_string(),
                    subscription: subscription.to_string(),
                    lapsed,
                },
            )?;
            Ok(Some(lapsed))
        })
    }

    /// Deletes an archived subscription and every delivery row it had
    /// (Kenny, 2026-09-30). Refused for one that is not archived: archiving
    /// first is what settles its backlog visibly. A client that polls the
    /// name again later starts a new subscription from that moment.
    pub fn delete_subscription(&self, topic: &str, subscription: &str) -> Result<()> {
        self.write(|tx| -> Result<()> {
            let sub_id = self.resolve_subscription(tx, topic, subscription)?;
            let state = queries::subscription_state(tx, sub_id)?;
            if state != "archived" {
                return Err(EngineError::NotArchived {
                    topic: topic.to_string(),
                    subscription: subscription.to_string(),
                    state,
                });
            }
            queries::delete_subscription(tx, sub_id)?;
            Ok(())
        })
    }

    /// K9 · a topic's retention, `None` meaning "keep forever".
    pub fn retention(&self, topic: &str) -> Result<(Option<Millis>, Option<Millis>)> {
        self.store
            .write(|tx| -> Result<(Option<Millis>, Option<Millis>)> {
                let Some(topic_id) = queries::topic_id_by_name(tx, topic)? else {
                    return Err(EngineError::UnknownTopic {
                        topic: topic.to_string(),
                        existing: queries::topic_names(tx)?,
                    });
                };
                let explicit = queries::topic_retention(tx, topic_id)?;
                Ok((explicit.or(self.defaults.retention_ms), explicit))
            })
    }

    pub fn set_retention(&self, topic: &str, retention_ms: Option<Millis>) -> Result<()> {
        if let Some(retention) = retention_ms
            && retention <= 0
        {
            return Err(EngineError::InvalidPolicy {
                field: "retention_ms",
                reason: "a retention window must be longer than zero milliseconds; omit it \
                         to fall back to the hub default, or set keep_forever to true"
                    .to_string(),
            });
        }
        self.write(|tx| -> Result<()> {
            let Some(topic_id) = queries::topic_id_by_name(tx, topic)? else {
                return Err(EngineError::UnknownTopic {
                    topic: topic.to_string(),
                    existing: queries::topic_names(tx)?,
                });
            };
            queries::set_topic_retention(tx, topic_id, retention_ms)?;
            Ok(())
        })
    }

    // ─── L4 · reliability semantics ─────────────────────────────────────

    /// K7. The policy in force for a subscription, alongside the raw stored
    /// values so a caller can tell which fields are explicit.
    pub fn policy(&self, topic: &str, subscription: &str) -> Result<(Policy, StoredPolicy)> {
        self.write(|tx| -> Result<(Policy, StoredPolicy)> {
            let sub_id = self.resolve_subscription(tx, topic, subscription)?;
            let stored = queries::subscription_policy(tx, sub_id)?;
            Ok((Policy::effective(stored), stored))
        })
    }

    /// K7. Replaces the whole policy: an omitted field goes back to its
    /// default rather than keeping whatever was there, because two update
    /// rules would be one more thing to remember.
    pub fn set_policy(
        &self,
        topic: &str,
        subscription: &str,
        stored: StoredPolicy,
    ) -> Result<Policy> {
        if let Err((field, reason)) = policy::validate(stored) {
            return Err(EngineError::InvalidPolicy { field, reason });
        }
        self.write(|tx| -> Result<Policy> {
            let sub_id = self.resolve_subscription(tx, topic, subscription)?;
            queries::set_subscription_policy(tx, sub_id, stored)?;
            Ok(Policy::effective(stored))
        })
    }
}
