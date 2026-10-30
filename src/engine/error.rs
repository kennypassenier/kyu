//! The engine's typed errors, each carrying the remedy a caller can act on.

use super::names;

/// Everything that can go wrong in a way the caller can fix. Each variant
/// carries a remedy, because an error without one just moves the problem
/// (AR4, standing rule 11).
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("{kind} name {name:?} is not allowed")]
    InvalidName { kind: &'static str, name: String },

    #[error("topic {topic:?} is reserved for kyu's own events")]
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

    #[error("message {id:?} is not on topic {topic:?}")]
    NoSuchMessage { id: String, topic: String },

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

    #[error("subscription {subscription:?} on topic {topic:?} is {state}, not archived")]
    NotArchived {
        topic: String,
        subscription: String,
        state: String,
    },

    #[error("an app named {name:?} already exists")]
    AppExists { name: String },

    #[error("no app named {name:?} is registered")]
    UnknownApp { name: String },

    #[error("this hub has no token configured, so it cannot manage apps")]
    Unprotected,

    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl EngineError {
    /// What the caller should do about it. Rendered into every error
    /// response body (AR4).
    pub fn remedy(&self) -> String {
        match self {
            Self::AppExists { .. } => "pick another name, or revoke the existing app first. \
                 Revoked names become available again."
                .to_string(),
            Self::UnknownApp { .. } => "register the app on the dashboard's apps page first; \
                 its token is generated there."
                .to_string(),
            Self::Unprotected => "set KYU_TOKEN and KYU_SECRET_KEY in your compose \
                 file and restart the hub. Without a token there is nothing to hand out."
                .to_string(),
            Self::InvalidName { kind, .. } => format!(
                "{kind} names may hold up to {} characters from a-z, 0-9, '.', '_' and '-', \
                 using dots to namespace (for example notify.kenny). Uppercase letters, \
                 spaces and slashes are not allowed.",
                names::MAX_NAME_LEN
            ),
            Self::ReservedTopic { .. } => format!(
                "the {:?} prefix carries kyu's own events, so only kyu publishes \
                 there. Pick another topic name; you can still subscribe to kyu topics \
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
            Self::NoSuchMessage { .. } => "the topic no longer holds that message: retention \
                 collects messages after the topic's retention window. Pick a message from \
                 the topic's recent messages, or publish the payload again."
                .to_string(),
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
            Self::NotArchived { state, .. } => format!(
                "only an archived subscription can be deleted, and this one is {state}. \
                 Archive it first (the Archive button on the topic page): that settles \
                 its backlog as lapsed, so nothing it still holds is lost silently."
            ),
            Self::Internal(_) => {
                "this is a fault in kyu rather than in the request. Check the hub's logs \
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

pub(super) type Result<T> = std::result::Result<T, EngineError>;

/// Refuses a topic, subscription or app name the hub cannot store.
pub(super) fn check_name(kind: &'static str, name: &str) -> Result<()> {
    if names::is_valid(name) {
        Ok(())
    } else {
        Err(EngineError::InvalidName {
            kind,
            name: name.to_string(),
        })
    }
}
