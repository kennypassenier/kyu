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

    pub fn for_subscription(&self, topic: &str, subscription: &str) -> Arc<Notify> {
        let key = (topic.to_string(), subscription.to_string());
        self.waiters
            .lock()
            .expect("the notifier lock is never poisoned")
            .entry(key)
            .or_default()
            .clone()
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
