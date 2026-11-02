//! Long-poll wakeups (AR5).
//!
//! One [`Notify`] per subscription. A publish wakes the subscriptions that
//! gained a delivery row, so a waiting consumer answers in milliseconds
//! instead of at the next poll.
//!
//! Correctness never depends on this: every waiter also re-checks the store
//! on a timer, so a missed wakeup costs latency and nothing else. That is
//! deliberate — a delivery guarantee resting on in-process signalling would
//! be a guarantee resting on nothing.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::Notify;

#[derive(Debug, Default)]
pub struct Notifiers {
    waiters: Mutex<HashMap<(String, String), Arc<Notify>>>,
}

impl Notifiers {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers interest in one subscription. The returned guard holds the
    /// notifier; when the last guard for a subscription drops, the map entry
    /// goes with it, so names nobody is polling cost no memory.
    pub fn for_subscription(self: &Arc<Self>, topic: &str, subscription: &str) -> Waiting {
        let key = (topic.to_string(), subscription.to_string());
        let notify = self
            .waiters
            .lock()
            .expect("the notifier lock is never poisoned")
            .entry(key.clone())
            .or_default()
            .clone();
        Waiting {
            notifiers: self.clone(),
            key,
            notify,
        }
    }

    /// How many subscriptions currently have a registered waiter.
    pub fn len(&self) -> usize {
        self.waiters
            .lock()
            .expect("the notifier lock is never poisoned")
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Wakes one waiter per named subscription. One is right for both
    /// delivery patterns: a lone consumer is woken, and among competing
    /// consumers exactly one goes for the message the others cannot have.
    pub fn wake(&self, topic: &str, subscriptions: &[String]) {
        let waiters = self
            .waiters
            .lock()
            .expect("the notifier lock is never poisoned");
        for subscription in subscriptions {
            if let Some(notify) = waiters.get(&(topic.to_string(), subscription.clone())) {
                notify.notify_one();
            }
        }
    }
}

/// One poll's hold on a subscription's [`Notify`]. Dropping it (on every
/// exit path: answer, error, timeout, client disconnect) removes the map
/// entry when no other poll shares it.
pub struct Waiting {
    notifiers: Arc<Notifiers>,
    key: (String, String),
    notify: Arc<Notify>,
}

impl Waiting {
    pub fn notify(&self) -> &Notify {
        &self.notify
    }
}

impl Drop for Waiting {
    fn drop(&mut self) {
        let mut waiters = self
            .notifiers
            .waiters
            .lock()
            .expect("the notifier lock is never poisoned");
        // Two owners means the map and this guard: nobody else is waiting.
        // Checked under the lock, so a poll registering concurrently either
        // already counts or finds the entry gone and creates a fresh one.
        if waiters
            .get(&self.key)
            .is_some_and(|held| Arc::strong_count(held) == 2)
        {
            waiters.remove(&self.key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_entry_lives_exactly_as_long_as_a_poll_holds_it() {
        let notifiers = Arc::new(Notifiers::new());
        let first = notifiers.for_subscription("t", "s");
        let second = notifiers.for_subscription("t", "s");
        assert_eq!(notifiers.len(), 1);
        drop(first);
        assert_eq!(notifiers.len(), 1, "a second poll still waits on it");
        drop(second);
        assert!(notifiers.is_empty(), "nobody holds it, so it is gone");
        for name in 0..100 {
            drop(notifiers.for_subscription("t", &format!("random-{name}")));
        }
        assert!(notifiers.is_empty(), "random names leave nothing behind");
    }
}
