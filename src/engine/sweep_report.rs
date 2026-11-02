//! What one sweep did, for the sweeper's log and its next decision.

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
    /// Why this sweep skipped everything that archives or deletes by age:
    /// the clock jumped, or is behind a message already published.
    pub clock_skew: Option<String>,
}

impl SweepReport {
    /// Records subscriptions of the events topic that gained a message.
    pub(super) fn wake_events(&mut self, subscriptions: Vec<String>) {
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
