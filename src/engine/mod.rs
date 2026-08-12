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

use std::sync::{Arc, Mutex};

use crate::store::queries::{self, AckOutcome, ClaimedMessage};
use crate::store::{DEFAULT_LEASE_MS, Store};

use clock::{Clock, Millis};
use ids::MessageIds;

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

    #[error("replaying a topic from the beginning is not implemented in this build")]
    ReplayUnsupported,

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
            Self::ReplayUnsupported => {
                "poll without from=beginning to receive messages published from now on. \
                 Replaying retained history arrives with the retention work (K8) and is \
                 refused here rather than silently ignored."
                    .to_string()
            }
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

pub struct Engine {
    store: Arc<Store>,
    clock: Arc<dyn Clock>,
    ids: Mutex<MessageIds>,
}

impl Engine {
    pub fn new(store: Arc<Store>, clock: Arc<dyn Clock>) -> Self {
        Self {
            store,
            clock,
            ids: Mutex::new(MessageIds::new()),
        }
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
        if from_beginning {
            return Err(EngineError::ReplayUnsupported);
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
            queries::touch_subscription_poll(tx, sub_id, now)?;

            let claimed = queries::claim_next(tx, sub_id, now, DEFAULT_LEASE_MS)?;

            Ok(Received {
                claimed: claimed.map(|message| Claimed {
                    topic: topic.to_string(),
                    message,
                }),
                created,
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

    pub fn now_ms(&self) -> Millis {
        self.clock.now_ms()
    }
}
