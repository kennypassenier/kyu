//! Publishing: K1 immediate, W4 scheduled, damped repeats, and replay.

use crate::store::dedup as store_dedup;
use crate::store::queries;

use super::clock::Millis;
use super::error::check_name;
use super::{Engine, EngineError, Published, Result, dedup, names};
impl Engine {
    /// K1. Stores the payload verbatim and creates one delivery row per
    /// active subscription, all in one transaction: a message that exists
    /// without its delivery rows would be a message nobody can receive.
    pub fn publish(
        &self,
        topic: &str,
        payload: &[u8],
        content_type: Option<&str>,
    ) -> Result<Published> {
        self.publish_due(topic, payload, content_type, None)
    }

    /// W4 · publish something that becomes deliverable later.
    ///
    /// The message is stored and durable straight away — only its delivery
    /// waits. A restart cannot lose the schedule, because the due time is a
    /// column rather than a timer in memory.
    pub fn publish_due(
        &self,
        topic: &str,
        payload: &[u8],
        content_type: Option<&str>,
        due_at: Option<Millis>,
    ) -> Result<Published> {
        self.publish_damped(topic, payload, content_type, due_at, None)
    }

    /// [`Self::publish_due`] with optional damping of repeats (`dedup`).
    pub fn publish_damped(
        &self,
        topic: &str,
        payload: &[u8],
        content_type: Option<&str>,
        due_at: Option<Millis>,
        dedup: Option<&dedup::Dedup>,
    ) -> Result<Published> {
        check_name("topic", topic)?;
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

        let message = NewMessage {
            id: &id,
            payload,
            content_type,
            due_at,
        };
        let stored = self.write(|tx| -> Result<std::result::Result<Vec<String>, String>> {
            let topic_id = match queries::topic_id_by_name(tx, topic)? {
                Some(id) => id,
                None => queries::create_topic(tx, topic, now)?,
            };
            if let Some(dedup) = dedup
                && let Some(original) = damp(tx, topic_id, topic, dedup, &id, now)?
            {
                return Ok(Err(original));
            }
            Ok(Ok(store_and_fan_out(tx, topic_id, topic, &message, now)?))
        })?;

        Ok(match stored {
            Ok(delivered_to) => Published {
                id,
                delivered_to,
                due_at,
                duplicate: false,
            },
            Err(original) => Published {
                id: original,
                delivered_to: Vec::new(),
                due_at: None,
                duplicate: true,
            },
        })
    }

    /// Publishes a stored message again as a new one (the dashboard's
    /// replay): same payload and content type, immediate, to every active
    /// subscription. The original stays as it is.
    pub fn replay(&self, topic: &str, message_id: &str) -> Result<Published> {
        let found = self
            .store
            .read(|conn| {
                let Some(topic_id) = queries::topic_id_by_name_conn(conn, topic)? else {
                    return Ok(None);
                };
                queries::message_for_replay(conn, topic_id, message_id)
            })
            .map_err(EngineError::Internal)?;
        let Some((payload, content_type)) = found else {
            return Err(EngineError::NoSuchMessage {
                id: message_id.to_string(),
                topic: topic.to_string(),
            });
        };
        self.publish(topic, &payload, content_type.as_deref())
    }
}

/// A message on its way into the store.
struct NewMessage<'a> {
    id: &'a str,
    payload: &'a [u8],
    content_type: Option<&'a str>,
    due_at: Option<Millis>,
}

/// The original message's id when `dedup` suppresses this publish;
/// otherwise remembers the key for this one and returns `None`.
fn damp(
    tx: &rusqlite::Transaction,
    topic_id: i64,
    topic: &str,
    dedup: &dedup::Dedup,
    id: &str,
    now: Millis,
) -> Result<Option<String>> {
    if let Some(original) = store_dedup::find_live(tx, topic_id, &dedup.key, now)? {
        store_dedup::count_suppressed(tx, topic, now)?;
        return Ok(Some(original));
    }
    store_dedup::remember(tx, topic_id, &dedup.key, id, now, dedup.window_ms)?;
    Ok(None)
}

/// Stores the message and one pending delivery per active subscription,
/// returning the names of the subscriptions it reached.
fn store_and_fan_out(
    tx: &rusqlite::Transaction,
    topic_id: i64,
    topic: &str,
    message: &NewMessage,
    now: Millis,
) -> Result<Vec<String>> {
    let seq = queries::insert_message(
        tx,
        message.id,
        topic_id,
        message.payload,
        message.content_type,
        now,
        message.due_at,
    )?;
    queries::bump_published(tx, topic, now)?;
    let subscriptions = queries::active_subscriptions(tx, topic_id)?;
    for subscription in &subscriptions {
        queries::insert_pending_delivery(tx, seq, subscription.id)?;
    }
    Ok(subscriptions
        .into_iter()
        .map(|subscription| subscription.name)
        .collect())
}
