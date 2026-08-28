//! Delivery semantics (AR1, AR9): publish, claim, ack, nack,
//! redelivery, dead-lettering, TTL, retention, idle lifecycle.
//!
//! Free of tokio and HTTP, and it never reads the wall clock: time arrives
//! through a [`clock::Clock`] and storage through [`crate::store`], which
//! is what makes the mocked-clock suites (K5, K7, K9, K11) possible.
//!
//! L2 lands the three verbs. Leases only *start* here — expiry,
//! redelivery, dead-lettering and TTL are L4, and until then an unacked
//! message stays claimed.

pub mod clock;
pub mod ids;
pub mod names;
pub mod policy;

use std::sync::{Arc, Mutex};

use crate::events::{self, Event};
use crate::store::Store;
use crate::store::queries::{
    self, AckOutcome, ClaimedMessage, DeadLetter, NackOutcome, Overdue, RequeueOutcome,
    StoredPolicy,
};

use clock::{Clock, Millis};
use ids::MessageIds;
use policy::Policy;

/// Everything that can go wrong in a way the caller can fix. Each variant
/// carries a remedy, because an error without one just moves the problem
/// (AR4, standing rule 11).
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("{kind} name {name:?} is not allowed")]
    InvalidName { kind: &'static str, name: String },

    #[error("topic {topic:?} is reserved for mailbox's own events")]
    ReservedTopic { topic: String },

    #[error("topic {topic:?} does not exist")]
    UnknownTopic {
        topic: String,
        existing: Vec<String>,
    },

    #[error("subscription {subscription:?} does not exist on topic {topic:?}")]
    UnknownSubscription {
        topic: String,
        subscription: String,
        existing: Vec<String>,
    },

    #[error("message {id:?} was never delivered to subscription {subscription:?}")]
    NoSuchDelivery { id: String, subscription: String },

    #[error("message {id:?} was already acknowledged by {subscription:?}")]
    AlreadyAcked { id: String, subscription: String },

    #[error("message {id:?} is {state}, not claimed by {subscription:?}")]
    NotClaimed {
        id: String,
        subscription: String,
        state: String,
    },

    #[error("message {id:?} is {state}, not in the dead-letter list of {subscription:?}")]
    NotDead {
        id: String,
        subscription: String,
        state: String,
    },

    #[error("{field} is not a usable value: {reason}")]
    InvalidPolicy { field: &'static str, reason: String },

    #[error("subscription {subscription:?} on topic {topic:?} is archived")]
    SubscriptionArchived { topic: String, subscription: String },

    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl EngineError {
    /// What the caller should do about it. Rendered into every error
    /// response body (AR4).
    pub fn remedy(&self) -> String {
        match self {
            Self::InvalidName { kind, .. } => format!(
                "{kind} names may hold up to {} characters from a-z, 0-9, '.', '_' and '-', \
                 using dots to namespace (for example notify.kenny). Uppercase letters, \
                 spaces and slashes are not allowed.",
                names::MAX_NAME_LEN
            ),
            Self::ReservedTopic { .. } => format!(
                "the {:?} prefix carries mailbox's own events, so only mailbox publishes \
                 there. Pick another topic name; you can still subscribe to mailbox topics \
                 to consume those events.",
                names::RESERVED_PREFIX
            ),
            Self::UnknownTopic { existing, .. } => format!(
                "a topic starts existing when something publishes to it, so publish first, \
                 then poll. {}",
                describe_existing("topics", existing)
            ),
            Self::UnknownSubscription {
                topic, existing, ..
            } => format!(
                "subscriptions are created by polling, so this name has never polled \
                 {topic:?}. Check the spelling. {}",
                describe_existing("subscriptions on this topic", existing)
            ),
            Self::NoSuchDelivery { subscription, .. } => format!(
                "this message id was not delivered to {subscription:?}. Acknowledge with the \
                 same subscription name that received it, and use the id from that response."
            ),
            Self::AlreadyAcked { .. } => {
                "the message is already settled, so there is nothing left to acknowledge. \
                 Acknowledging twice is harmless; if you expected a new message, poll again."
                    .to_string()
            }
            Self::NotClaimed { state, .. } => format!(
                "the delivery is {state}: acknowledging only settles a message this \
                 subscription currently holds. Poll for it, handle it, then acknowledge the \
                 id that poll returned."
            ),
            Self::NotDead { state, .. } => format!(
                "only a dead-lettered message can be requeued, and this one is {state}. \
                 List the dead letters for this subscription to see what is actually \
                 waiting there."
            ),
            Self::InvalidPolicy { .. } => {
                "correct that field and send the policy again. A policy write replaces \
                 every field, so omitting one resets it to its default rather than \
                 leaving the previous value in place."
                    .to_string()
            }
            Self::SubscriptionArchived {
                topic,
                subscription,
            } => format!(
                "this subscription was archived after going unpolled for too long, and \
                 the messages it was holding were settled as lapsed. Unarchive it with \
                 POST /api/t/{topic}/subs/{subscription}/unarchive to start receiving \
                 again — from that moment on, or add ?from=beginning to a poll to pick up \
                 what the topic still retains."
            ),
            Self::Internal(_) => {
                "this is a fault in mailbox rather than in the request. Check the hub's logs \
                 for the matching error line and the dashboard for the store's health."
                    .to_string()
            }
        }
    }
}

fn describe_existing(what: &str, existing: &[String]) -> String {
    if existing.is_empty() {
        format!("There are no {what} yet.")
    } else {
        format!("Existing {what}: {}.", existing.join(", "))
    }
}

type Result<T> = std::result::Result<T, EngineError>;

/// Rows per transaction while replaying a topic into a subscription (AR5).
const BACKFILL_BATCH: usize = 500;

/// What a publish did, so the caller can wake exactly the subscriptions
/// that gained a message. The engine stays free of tokio by reporting them
/// instead of notifying them itself (AR1, AR5).
#[derive(Debug, Clone)]
pub struct Published {
    pub id: String,
    pub delivered_to: Vec<String>,
}

/// A subscription that this very poll brought into existence, with the
/// number of messages the topic already held. Reported so the caller can
/// say so out loud: a fresh subscription's first poll finds nothing by
/// design (G7), and an unexplained 204 there looks exactly like a lost
/// message (G8).
#[derive(Debug, Clone)]
pub struct NewSubscription {
    pub name: String,
    pub retained_before: i64,
}

/// The outcome of a poll: what was claimed, and whether the subscription
/// was born in the process.
#[derive(Debug, Clone)]
pub struct Received {
    pub claimed: Option<Claimed>,
    pub created: Option<NewSubscription>,
    /// How many retained messages a `from=beginning` poll pulled in (K8).
    pub backfilled: usize,
}

/// A claimed message and where it came from.
#[derive(Debug, Clone)]
pub struct Claimed {
    pub topic: String,
    pub message: ClaimedMessage,
}

impl Claimed {
    /// Which attempt this delivery is. `attempts` counts the ones that
    /// already failed (AR9), so the one in flight is the next number.
    pub fn attempt(&self) -> i64 {
        self.message.attempts + 1
    }
}

/// Hub-wide defaults that are not per-subscription policy (AR6): they are
/// read from the environment once and apply wherever nothing more specific
/// is set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Defaults {
    /// How long a topic keeps messages nobody still needs (K9).
    pub retention_ms: Option<Millis>,
    /// Idle time before a subscription is flagged on the dashboard (K11).
    pub idle_flag_ms: Millis,
    /// Idle time before it is archived and stops accumulating (K11).
    pub idle_archive_ms: Millis,
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            retention_ms: Some(7 * 24 * 60 * 60 * 1_000),
            idle_flag_ms: 7 * 24 * 60 * 60 * 1_000,
            idle_archive_ms: 30 * 24 * 60 * 60 * 1_000,
        }
    }
}

pub struct Engine {
    store: Arc<Store>,
    clock: Arc<dyn Clock>,
    ids: Mutex<MessageIds>,
    defaults: Defaults,
}

impl Engine {
    pub fn new(store: Arc<Store>, clock: Arc<dyn Clock>) -> Self {
        Self::with_defaults(store, clock, Defaults::default())
    }

    pub fn with_defaults(store: Arc<Store>, clock: Arc<dyn Clock>, defaults: Defaults) -> Self {
        Self {
            store,
            clock,
            ids: Mutex::new(MessageIds::new()),
            defaults,
        }
    }

    pub fn defaults(&self) -> Defaults {
        self.defaults
    }

    pub fn store(&self) -> &Arc<Store> {
        &self.store
    }

    /// K1. Stores the payload verbatim and creates one delivery row per
    /// active subscription, all in one transaction: a message that exists
    /// without its delivery rows would be a message nobody can receive.
    pub fn publish(
        &self,
        topic: &str,
        payload: &[u8],
        content_type: Option<&str>,
    ) -> Result<Published> {
        if !names::is_valid(topic) {
            return Err(EngineError::InvalidName {
                kind: "topic",
                name: topic.to_string(),
            });
        }
        if names::is_reserved(topic) {
            return Err(EngineError::ReservedTopic {
                topic: topic.to_string(),
            });
        }

        let now = self.clock.now_ms();
        let id = self
            .ids
            .lock()
            .expect("the id lock is never poisoned")
            .next(now)
            .to_string();

        let delivered_to = self.store.write(|tx| -> Result<Vec<String>> {
            let topic_id = match queries::topic_id_by_name(tx, topic)? {
                Some(id) => id,
                None => queries::create_topic(tx, topic, now)?,
            };

            let seq = queries::insert_message(tx, &id, topic_id, payload, content_type, now, None)?;

            let subscriptions = queries::active_subscriptions(tx, topic_id)?;
            for subscription in &subscriptions {
                queries::insert_pending_delivery(tx, seq, subscription.id)?;
            }

            Ok(subscriptions
                .into_iter()
                .map(|subscription| subscription.name)
                .collect::<Vec<_>>())
        })?;

        Ok(Published { id, delivered_to })
    }

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

        self.store.write(|tx| -> Result<Received> {
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

        self.store.write(|tx| -> Result<()> {
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
                AckOutcome::Acked => Ok(()),
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

    /// K11 · brings an archived subscription back. Deliberately explicit:
    /// a poll will not do it, so nobody resumes without seeing that the
    /// backlog was lapsed.
    pub fn unarchive(&self, topic: &str, subscription: &str) -> Result<()> {
        let now = self.clock.now_ms();
        self.store.write(|tx| -> Result<()> {
            let sub_id = self.resolve_subscription(tx, topic, subscription)?;
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
        self.store.write(|tx| -> Result<()> {
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

    pub fn now_ms(&self) -> Millis {
        self.clock.now_ms()
    }

    // ─── L4 · reliability semantics ─────────────────────────────────────

    /// K7. The policy in force for a subscription, alongside the raw stored
    /// values so a caller can tell which fields are explicit.
    pub fn policy(&self, topic: &str, subscription: &str) -> Result<(Policy, StoredPolicy)> {
        self.store.write(|tx| -> Result<(Policy, StoredPolicy)> {
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
        self.store.write(|tx| -> Result<Policy> {
            let sub_id = self.resolve_subscription(tx, topic, subscription)?;
            queries::set_subscription_policy(tx, sub_id, stored)?;
            Ok(Policy::effective(stored))
        })
    }

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

        self.store.write(|tx| -> Result<Settled> {
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

            if dead {
                queries::mark_dead(tx, delivery.msg_seq, sub_id, delivery.attempts + 1, now)?;
                return Ok(Settled::DeadLettered);
            }

            Ok(apply(tx, &delivery, now)?)
        })
    }

    /// K6. What is waiting in a subscription's dead-letter list.
    pub fn dead_letters(
        &self,
        topic: &str,
        subscription: &str,
        limit: usize,
    ) -> Result<Vec<DeadLetter>> {
        self.store.write(|tx| -> Result<Vec<DeadLetter>> {
            let sub_id = self.resolve_subscription(tx, topic, subscription)?;
            Ok(queries::dead_letters(tx, sub_id, limit)?)
        })
    }

    /// K6. Puts a dead letter back in the queue with a fresh set of
    /// attempts.
    pub fn requeue_dead(&self, topic: &str, subscription: &str, message_id: &str) -> Result<()> {
        self.store.write(|tx| -> Result<()> {
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

    /// K5, K6, K7. One pass of the background work: expired leases return
    /// to the queue, exhausted deliveries are dead-lettered, and messages
    /// past their TTL are settled.
    ///
    /// Bounded by `batch_limit` (AR5): the writer connection is held for one
    /// batch, never for a whole table, so a publish never waits seconds
    /// behind a sweep and a hard kill mid-sweep has little to roll back.
    pub fn sweep(&self, batch_limit: usize) -> Result<SweepReport> {
        let now = self.clock.now_ms();

        self.store.write(|tx| -> Result<SweepReport> {
            let mut report = SweepReport::default();

            let overdue = queries::overdue_claims(tx, now, batch_limit)?;
            let stale = queries::pending_past_ttl(tx, now, batch_limit)?;
            report.more_work = overdue.len() >= batch_limit || stale.len() >= batch_limit;

            // Locking the id generator inside the store transaction is safe
            // in one direction only: publish takes the generator and releases
            // it *before* the store, so the two can never wait on each other.
            let mut ids = self.ids.lock().expect("the id lock is never poisoned");

            for delivery in overdue {
                let settled = apply(tx, &delivery, now)?;
                match settled {
                    Settled::Redelivered => {
                        report.redelivered += 1;
                        report
                            .wake
                            .push((delivery.topic.clone(), delivery.subscription.clone()));
                    }
                    Settled::DeadLettered => {
                        report.dead_lettered += 1;
                        let woken = events::emit(
                            tx,
                            &mut ids,
                            now,
                            &Event::DeadLettered {
                                topic: delivery.topic.clone(),
                                subscription: delivery.subscription.clone(),
                                message_id: queries::message_id_of(tx, delivery.msg_seq)?,
                                attempts: delivery.attempts + 1,
                            },
                        )?;
                        report.wake_events(woken);
                    }
                    Settled::Expired => report.expired += 1,
                    Settled::Unchanged => {}
                }
            }

            // A pending message past its TTL never had a chance to fail, so
            // its attempt count stays as it is.
            let mut expired_by_subscription: Vec<(String, String)> = Vec::new();
            for delivery in stale {
                if queries::mark_expired(tx, delivery.msg_seq, delivery.sub_id, now)? {
                    report.expired += 1;
                    expired_by_subscription
                        .push((delivery.topic.clone(), delivery.subscription.clone()));
                }
            }
            // One event per subscription rather than per message: a TTL sweep
            // can settle hundreds at once, and a flood of events is its own
            // kind of silence.
            expired_by_subscription.sort();
            let mut counted: Vec<(String, String, usize)> = Vec::new();
            for pair in expired_by_subscription {
                match counted.last_mut() {
                    Some(last) if last.0 == pair.0 && last.1 == pair.1 => last.2 += 1,
                    _ => counted.push((pair.0, pair.1, 1)),
                }
            }
            for (topic, subscription, count) in counted {
                let woken = events::emit(
                    tx,
                    &mut ids,
                    now,
                    &Event::Expired {
                        topic,
                        subscription,
                        count,
                    },
                )?;
                report.wake_events(woken);
            }

            // K11 · idle lifecycle.
            let flag_cutoff = now.saturating_sub(self.defaults.idle_flag_ms);
            for subscription in queries::subscriptions_to_flag(tx, flag_cutoff, batch_limit)? {
                queries::set_subscription_state(tx, subscription.id, "flagged")?;
                report.flagged += 1;
                let woken = events::emit(
                    tx,
                    &mut ids,
                    now,
                    &Event::SubscriptionFlagged {
                        topic: subscription.topic.clone(),
                        subscription: subscription.name.clone(),
                        idle_ms: self.defaults.idle_flag_ms,
                    },
                )?;
                report.wake_events(woken);
            }

            let archive_cutoff = now.saturating_sub(self.defaults.idle_archive_ms);
            for subscription in queries::subscriptions_to_archive(tx, archive_cutoff, batch_limit)?
            {
                queries::set_subscription_state(tx, subscription.id, "archived")?;
                let lapsed = queries::lapse_outstanding(tx, subscription.id)?;
                report.archived += 1;
                report.lapsed += lapsed;
                let woken = events::emit(
                    tx,
                    &mut ids,
                    now,
                    &Event::SubscriptionArchived {
                        topic: subscription.topic.clone(),
                        subscription: subscription.name.clone(),
                        lapsed,
                    },
                )?;
                report.wake_events(woken);
            }

            // K9 · retention, last: everything above may have settled a
            // delivery that was the only reason to keep a message.
            let collected =
                queries::collect_retained(tx, now, self.defaults.retention_ms, batch_limit)?;
            if collected > 0 {
                report.collected = collected;
                let woken = events::emit(
                    tx,
                    &mut ids,
                    now,
                    &Event::MessagesCollected { count: collected },
                )?;
                report.wake_events(woken);
            }
            report.more_work = report.more_work
                || collected >= batch_limit
                || report.flagged >= batch_limit
                || report.archived >= batch_limit;

            report.wake.sort();
            report.wake.dedup();
            Ok(report)
        })
    }

    fn resolve_subscription(
        &self,
        tx: &rusqlite::Transaction,
        topic: &str,
        subscription: &str,
    ) -> Result<i64> {
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
        Ok(sub_id)
    }
}

/// Where a delivery ended up after failing (AR9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settled {
    Redelivered,
    DeadLettered,
    Expired,
    /// Someone acked it between the scan and the update; the guard held.
    Unchanged,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SweepReport {
    pub redelivered: usize,
    pub dead_lettered: usize,
    pub expired: usize,
    /// Messages retention collected (K9).
    pub collected: usize,
    /// Subscriptions newly flagged as idle (K11).
    pub flagged: usize,
    /// Subscriptions archived, and the deliveries that lapsed with them.
    pub archived: usize,
    pub lapsed: usize,
    /// Subscriptions with a message waiting again, so their waiting pollers
    /// can be woken instead of sitting out their timeout (AR5).
    pub wake: Vec<(String, String)>,
    /// A batch was filled, so another pass should follow immediately.
    pub more_work: bool,
}

impl SweepReport {
    /// Records subscriptions of the events topic that gained a message.
    fn wake_events(&mut self, subscriptions: Vec<String>) {
        for subscription in subscriptions {
            self.wake
                .push((crate::events::EVENTS_TOPIC.to_string(), subscription));
        }
    }

    pub fn changed(&self) -> usize {
        self.redelivered
            + self.dead_lettered
            + self.expired
            + self.collected
            + self.flagged
            + self.archived
    }
}

/// The one place a failed delivery's fate is decided (AR9), shared by lease
/// expiry and nack so the two can never diverge.
fn apply(tx: &rusqlite::Transaction, delivery: &Overdue, now: Millis) -> anyhow::Result<Settled> {
    let policy = Policy::effective(delivery.policy);
    let attempts = delivery.attempts + 1;

    // TTL is re-checked here, not only while a message waits: a consumer
    // that claims a message and then hangs for half an hour must not be
    // able to resurrect it past its deadline (AR9, the stale-doorbell case).
    if policy.is_past_ttl(delivery.published_at, now) {
        return Ok(
            if queries::mark_expired(tx, delivery.msg_seq, delivery.sub_id, now)? {
                Settled::Expired
            } else {
                Settled::Unchanged
            },
        );
    }

    if attempts >= policy.max_attempts {
        return Ok(
            if queries::mark_dead(tx, delivery.msg_seq, delivery.sub_id, attempts, now)? {
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
            Settled::Redelivered
        } else {
            Settled::Unchanged
        },
    )
}
