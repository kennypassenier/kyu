//! The background pass: leases, TTL, retention, idle subscriptions.

use crate::events::{self, Event};
use crate::store::dedup as store_dedup;
use crate::store::queries::{self, Overdue};

use super::clock::Millis;
use super::ids::MessageIds;
use super::settle::{Cause, Settled, apply};
use super::{Defaults, Engine, Result};

impl Engine {
    /// K5, K6, K7. One pass of the background work: expired leases return
    /// to the queue, exhausted deliveries are dead-lettered, and messages
    /// past their TTL are settled.
    ///
    /// Bounded by `batch_limit` (AR5): the writer connection is held for one
    /// batch, never for a whole table, so a publish never waits seconds
    /// behind a sweep and a hard kill mid-sweep has little to roll back.
    pub fn sweep(&self, batch_limit: usize) -> Result<SweepReport> {
        let now = self.clock.now_ms();

        self.write(|tx| -> Result<SweepReport> {
            // Locking the id generator inside the store transaction is safe
            // in one direction only: publish takes the generator and releases
            // it *before* the store, so the two can never wait on each other.
            let mut ids = self.ids.lock().expect("the id lock is never poisoned");
            let mut pass = Pass {
                tx,
                ids: &mut ids,
                now,
                limit: batch_limit,
                defaults: self.defaults,
                report: SweepReport::default(),
            };
            pass.settle_leases()?;
            pass.announce_expiries()?;
            pass.idle_lifecycle()?;
            // K9 · retention, last: everything above may have settled a
            // delivery that was the only reason to keep a message.
            pass.collect_retained()?;
            pass.prune_history()?;

            let mut report = pass.report;
            report.wake.sort();
            report.wake.dedup();
            Ok(report)
        })
    }
}

/// One sweep in progress: the transaction, the clock reading and the
/// report the steps fill in.
struct Pass<'a> {
    tx: &'a rusqlite::Transaction<'a>,
    ids: &'a mut MessageIds,
    now: Millis,
    limit: usize,
    defaults: Defaults,
    report: SweepReport,
}

impl Pass<'_> {
    /// Writes one event to `kyu.events` and remembers who to wake.
    fn emit(&mut self, event: &Event) -> Result<()> {
        let woken = events::emit(self.tx, self.ids, self.now, event)?;
        self.report.wake_events(woken);
        Ok(())
    }

    /// A batch was filled, so another pass should follow immediately.
    fn note_batch(&mut self, rows: usize) {
        self.report.more_work |= rows >= self.limit;
    }

    /// Overdue leases go back, on, or out (AR9); pending messages past their
    /// TTL expire.
    fn settle_leases(&mut self) -> Result<()> {
        let overdue = queries::overdue_claims(self.tx, self.now, self.limit)?;
        let stale = queries::pending_past_ttl(self.tx, self.now, self.limit)?;
        self.note_batch(overdue.len());
        self.note_batch(stale.len());

        for delivery in overdue {
            self.settle_overdue(&delivery)?;
        }
        // A pending message past its TTL never had a chance to fail, so
        // its attempt count stays as it is.
        for delivery in stale {
            if queries::mark_expired(self.tx, delivery.msg_seq, delivery.sub_id, self.now)? {
                self.report.expired += 1;
                queries::note_expiry(self.tx, delivery.sub_id, 1)?;
            }
        }
        Ok(())
    }

    fn settle_overdue(&mut self, delivery: &Overdue) -> Result<()> {
        match apply(self.tx, delivery, self.now, Cause::LeaseExpired)? {
            Settled::Redelivered => {
                self.report.redelivered += 1;
                self.report
                    .wake
                    .push((delivery.topic.clone(), delivery.subscription.clone()));
            }
            Settled::DeadLettered => {
                self.report.dead_lettered += 1;
                self.emit(&Event::DeadLettered {
                    topic: delivery.topic.clone(),
                    subscription: delivery.subscription.clone(),
                    message_id: queries::message_id_of(self.tx, delivery.msg_seq)?,
                    attempts: delivery.attempts + 1,
                })?;
            }
            Settled::Expired => {
                self.report.expired += 1;
                queries::note_expiry(self.tx, delivery.sub_id, 1)?;
            }
            Settled::Unchanged => {}
        }
        Ok(())
    }

    /// One event per subscription per WINDOW, carrying the count since the
    /// last one (W11, amended 2026-09-18). It used to be one per
    /// subscription per sweep, which was the right idea on the wrong time
    /// scale: a subscription nobody polls expires its messages one sweep
    /// apart, so the CT 109 store held 27,991 events of which 27,969
    /// carried count 1 — each one a Home Assistant notification, each of
    /// those a new message on the same topic. The first expiry after a
    /// quiet spell still goes out at once.
    fn announce_expiries(&mut self) -> Result<()> {
        let window = self.defaults.expired_event_window_ms;
        for due in queries::expiry_announcements_due(self.tx, self.now, window, self.limit)? {
            queries::mark_expiries_announced(self.tx, due.sub_id, self.now)?;
            self.emit(&Event::Expired {
                topic: due.topic,
                subscription: due.subscription,
                count: due.count,
            })?;
        }
        Ok(())
    }

    /// K11 · idle subscriptions are flagged first and archived later.
    fn idle_lifecycle(&mut self) -> Result<()> {
        let (tx, now, limit) = (self.tx, self.now, self.limit);
        for subscription in
            queries::subscriptions_to_flag(tx, now, self.defaults.idle_flag_ms, limit)?
        {
            queries::set_subscription_state(tx, subscription.id, "flagged")?;
            self.report.flagged += 1;
            self.emit(&Event::SubscriptionFlagged {
                topic: subscription.topic,
                subscription: subscription.name,
                idle_ms: subscription.idle_ms,
            })?;
        }
        for subscription in
            queries::subscriptions_to_archive(tx, now, self.defaults.idle_archive_ms, limit)?
        {
            queries::set_subscription_state(tx, subscription.id, "archived")?;
            let lapsed = queries::lapse_outstanding(tx, subscription.id, now)?;
            self.report.archived += 1;
            self.report.lapsed += lapsed;
            self.emit(&Event::SubscriptionArchived {
                topic: subscription.topic,
                subscription: subscription.name,
                lapsed,
            })?;
        }
        let (flagged, archived) = (self.report.flagged, self.report.archived);
        self.note_batch(flagged);
        self.note_batch(archived);
        Ok(())
    }

    fn collect_retained(&mut self) -> Result<()> {
        let collected =
            queries::collect_retained(self.tx, self.now, self.defaults.retention_ms, self.limit)?;
        self.note_batch(collected);
        if collected > 0 {
            self.report.collected = collected;
            // Logged rather than published, deliberately. Retention is the
            // hub's own housekeeping, and an event about it lands on
            // kyu.events — where it becomes a message that retention must
            // eventually collect, emitting another. The topic would never
            // reach quiescence. The count is on the dashboard and in the
            // metrics; this is the loud-enough place for it.
            tracing::info!(collected, "retention collected messages");
        }
        Ok(())
    }

    /// History: a totals sample every few minutes, and the caps on
    /// everything the dashboard's statistics keep.
    fn prune_history(&mut self) -> Result<()> {
        queries::sample_totals_if_due(self.tx, self.now)?;
        let pruned = queries::prune_history(self.tx, self.now, self.limit)?;
        let pruned_keys = store_dedup::prune(self.tx, self.now, self.limit)?;
        self.note_batch(pruned);
        self.note_batch(pruned_keys);
        Ok(())
    }
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
