//! The cheap path of a poll: look on a reader before taking the writer.
//!
//! Most polls find nothing. Sending every one of them through the single
//! writer made an idle consumer cost a write transaction per poll; now an
//! empty poll is a read, and the subscription's poll time is written at most
//! once per [`TOUCH_INTERVAL_MS`].

use crate::store::queries;

use super::clock::Millis;
use super::{Engine, Received, Result};

/// How stale `last_poll_at` may get: the idle thresholds are days, and the
/// dashboard's "last read" only needs to be right to the half minute.
pub(super) const TOUCH_INTERVAL_MS: Millis = 30_000;

impl Engine {
    /// [`Self::claim_next_as`], but a poll that has nothing to claim on an
    /// existing active subscription never takes the writer, except for the
    /// periodic poll-time write.
    pub fn poll_next_as(
        &self,
        topic: &str,
        subscription: &str,
        from_beginning: bool,
        app: Option<&str>,
    ) -> Result<Received> {
        let now = self.clock.now_ms();
        let idle = !from_beginning
            && self.read(|conn| queries::is_idle_and_active(conn, topic, subscription, now))?;
        if !idle {
            return self.claim_next_as(topic, subscription, from_beginning, app);
        }
        if self.due_to_touch(topic, subscription, now) {
            self.write(|tx| {
                let sub_id = self.resolve_subscription(tx, topic, subscription)?;
                queries::touch_subscription_poll(tx, sub_id, now)?;
                Ok(())
            })?;
        }
        Ok(Received {
            claimed: None,
            created: None,
            backfilled: 0,
        })
    }

    /// Whether this subscription's poll time is old enough to write again;
    /// answering yes records `now` as the time it was written.
    pub(super) fn due_to_touch(&self, topic: &str, subscription: &str, now: Millis) -> bool {
        let mut touched = self
            .poll_touched
            .lock()
            .expect("the poll-touch lock is never poisoned");
        let key = (topic.to_string(), subscription.to_string());
        match touched.get(&key) {
            Some(last) if now >= *last && now - *last < TOUCH_INTERVAL_MS => false,
            _ => {
                touched.insert(key, now);
                true
            }
        }
    }
}
