//! The background pass: leases, TTL, retention, idle subscriptions.

use crate::events::{self, Event};
use crate::store::dedup as store_dedup;
use crate::store::queries;

use super::settle::{Cause, Settled, apply};
use super::{Engine, Result};
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
            let mut report = SweepReport::default();

            let overdue = queries::overdue_claims(tx, now, batch_limit)?;
            let stale = queries::pending_past_ttl(tx, now, batch_limit)?;
            report.more_work = overdue.len() >= batch_limit || stale.len() >= batch_limit;

            // Locking the id generator inside the store transaction is safe
            // in one direction only: publish takes the generator and releases
            // it *before* the store, so the two can never wait on each other.
            let mut ids = self.ids.lock().expect("the id lock is never poisoned");

            for delivery in overdue {
                let settled = apply(tx, &delivery, now, Cause::LeaseExpired)?;
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
                    Settled::Expired => {
                        report.expired += 1;
                        queries::note_expiry(tx, delivery.sub_id, 1)?;
                    }
                    Settled::Unchanged => {}
                }
            }

            // A pending message past its TTL never had a chance to fail, so
            // its attempt count stays as it is.
            for delivery in stale {
                if queries::mark_expired(tx, delivery.msg_seq, delivery.sub_id, now)? {
                    report.expired += 1;
                    queries::note_expiry(tx, delivery.sub_id, 1)?;
                }
            }
            // One event per subscription per WINDOW, carrying the count since
            // the last one (W11, amended 2026-09-18). It used to be one per
            // subscription per sweep, which was the right idea on the wrong
            // time scale: a subscription nobody polls expires its messages
            // one sweep apart, so the CT 109 store held 27,991 events of
            // which 27,969 carried count 1 — each one a Home Assistant
            // notification, each of those a new message on the same topic.
            // The first expiry after a quiet spell still goes out at once.
            for due in queries::expiry_announcements_due(
                tx,
                now,
                self.defaults.expired_event_window_ms,
                batch_limit,
            )? {
                queries::mark_expiries_announced(tx, due.sub_id, now)?;
                let woken = events::emit(
                    tx,
                    &mut ids,
                    now,
                    &Event::Expired {
                        topic: due.topic,
                        subscription: due.subscription,
                        count: due.count,
                    },
                )?;
                report.wake_events(woken);
            }

            // K11 · idle lifecycle.
            for subscription in
                queries::subscriptions_to_flag(tx, now, self.defaults.idle_flag_ms, batch_limit)?
            {
                queries::set_subscription_state(tx, subscription.id, "flagged")?;
                report.flagged += 1;
                let woken = events::emit(
                    tx,
                    &mut ids,
                    now,
                    &Event::SubscriptionFlagged {
                        topic: subscription.topic.clone(),
                        subscription: subscription.name.clone(),
                        idle_ms: subscription.idle_ms,
                    },
                )?;
                report.wake_events(woken);
            }

            for subscription in queries::subscriptions_to_archive(
                tx,
                now,
                self.defaults.idle_archive_ms,
                batch_limit,
            )? {
                queries::set_subscription_state(tx, subscription.id, "archived")?;
                let lapsed = queries::lapse_outstanding(tx, subscription.id, now)?;
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
                // Logged rather than published, deliberately. Retention is
                // the hub's own housekeeping, and an event about it lands on
                // kyu.events — where it becomes a message that retention
                // must eventually collect, emitting another. The topic would
                // never reach quiescence. The count is on the dashboard and
                // in the metrics; this is the loud-enough place for it.
                tracing::info!(collected, "retention collected messages");
            }
            report.more_work = report.more_work
                || collected >= batch_limit
                || report.flagged >= batch_limit
                || report.archived >= batch_limit;

            // History: a totals sample every few minutes, and the caps on
            // everything the dashboard's statistics keep.
            queries::sample_totals_if_due(tx, now)?;
            let pruned = queries::prune_history(tx, now, batch_limit)?;
            let pruned_keys = store_dedup::prune(tx, now, batch_limit)?;
            report.more_work =
                report.more_work || pruned >= batch_limit || pruned_keys >= batch_limit;

            report.wake.sort();
            report.wake.dedup();
            Ok(report)
        })
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
