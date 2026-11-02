//! The background sweeper (AR5): one task, a coarse tick, bounded batches.
//!
//! It drives the transitions nobody asks for over HTTP — leases running
//! out, retries becoming dead letters, messages passing their TTL — through
//! the same [`Engine::sweep`] the mocked-clock tests exercise, so the
//! behaviour under test is the behaviour in production.
//!
//! Shell rather than domain: the timer and the runtime live here, the
//! decisions live in the engine (AR1).

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, AtomicU32, Ordering};
use std::time::Duration;

use tokio::task::JoinHandle;

use crate::engine::Engine;
use crate::engine::clock::Millis;

/// How often the sweeper looks. AR5 caps this at one second: it is the
/// worst-case delay before an expired lease becomes visible again.
pub const TICK: Duration = Duration::from_secs(1);

/// Rows per transaction. Small enough that the single writer connection is
/// never held long, so a publish never queues behind a sweep and a hard kill
/// mid-sweep has almost nothing to roll back (AR5, K12).
pub const BATCH_LIMIT: usize = 500;

/// Back-to-back passes allowed within one tick. A backlog drains across
/// ticks beyond this, so one tick can never monopolise the writer.
pub const MAX_PASSES_PER_TICK: usize = 20;

/// Consecutive failed ticks before the sweeper reports itself failing.
pub const FAILING_AFTER: u32 = 3;

/// When the sweeper last completed a pass successfully, and how it has been
/// failing since.
///
/// Shared with the health endpoint (W6): a hub whose sweeper has stopped
/// still answers every request, but leases stop expiring and messages stop
/// coming back — a failure that is invisible from the outside unless
/// something watches for it.
#[derive(Debug, Clone)]
pub struct Heartbeat(Arc<Beat>);

#[derive(Debug)]
struct Beat {
    last_ms: AtomicI64,
    failures: AtomicU32,
    last_error: Mutex<Option<String>>,
}

impl Heartbeat {
    pub fn starting_at(now_ms: Millis) -> Self {
        Self(Arc::new(Beat {
            last_ms: AtomicI64::new(now_ms),
            failures: AtomicU32::new(0),
            last_error: Mutex::new(None),
        }))
    }

    /// A pass completed: the sweeper is alive and any failure streak ends.
    pub fn beat(&self, now_ms: Millis) {
        self.0.last_ms.store(now_ms, Ordering::Relaxed);
        self.0.failures.store(0, Ordering::Relaxed);
    }

    /// A tick failed. The heartbeat is deliberately not advanced.
    pub fn record_failure(&self, error: String) {
        self.0.failures.fetch_add(1, Ordering::Relaxed);
        *self
            .0
            .last_error
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(error);
    }

    pub fn consecutive_failures(&self) -> u32 {
        self.0.failures.load(Ordering::Relaxed)
    }

    /// The error of the most recent failed tick, if any ever failed.
    pub fn last_error(&self) -> Option<String> {
        self.0
            .last_error
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub fn last_beat_ms(&self) -> Millis {
        self.0.last_ms.load(Ordering::Relaxed)
    }

    /// A sweeper that has missed several ticks is reported unhealthy. The
    /// margin is generous: a busy backlog can make one pass take a while,
    /// and a false alarm every restart would teach you to ignore it.
    pub fn is_alive(&self, now_ms: Millis) -> bool {
        now_ms.saturating_sub(self.last_beat_ms()) < STALE_AFTER_MS
    }
}

/// How far behind the heartbeat may fall before the hub calls itself
/// unhealthy: five ticks.
pub const STALE_AFTER_MS: Millis = 5_000;

/// Runs until the returned handle is dropped or aborted.
///
/// `wake` is called with the subscriptions that gained a message again, so
/// a waiting long poll answers at once instead of sitting out its timeout.
/// It is a callback rather than a direct dependency on the notifier so this
/// module stays independent of the HTTP layer.
pub fn spawn<F>(engine: Arc<Engine>, heartbeat: Heartbeat, wake: F) -> JoinHandle<()>
where
    F: Fn(&[(String, String)]) + Send + Sync + 'static,
{
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(TICK);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            ticker.tick().await;
            // A failed tick must not kill the task: the next tick tries
            // again, the error is on the record, and the heartbeat stays
            // put so health sees the streak.
            match tick(&engine, &wake).await {
                Ok(()) => heartbeat.beat(engine.now_ms()),
                Err(error) => {
                    tracing::error!(error = %error, "the sweep failed");
                    heartbeat.record_failure(error);
                }
            }
        }
    })
}

/// One tick: passes in a row while they keep changing things, at most
/// [`MAX_PASSES_PER_TICK`]. The tick paces idle polling, not the draining
/// of a backlog.
async fn tick<F>(engine: &Arc<Engine>, wake: &F) -> Result<(), String>
where
    F: Fn(&[(String, String)]) + Sync,
{
    for _ in 0..MAX_PASSES_PER_TICK {
        let worker = engine.clone();
        let report = tokio::task::spawn_blocking(move || worker.sweep(BATCH_LIMIT))
            .await
            .map_err(|join| {
                if join.is_panic() {
                    "sweeper tick panicked in the sweep".to_string()
                } else {
                    format!("the sweep task failed to run: {join}")
                }
            })?
            .map_err(|error| format!("{error:?}"))?;

        if report.changed() > 0 {
            tracing::info!(
                redelivered = report.redelivered,
                dead_lettered = report.dead_lettered,
                expired = report.expired,
                "sweep settled overdue deliveries"
            );
        }
        if !report.wake.is_empty() {
            catch_unwind(AssertUnwindSafe(|| wake(&report.wake)))
                .map_err(|_| "sweeper tick panicked in the wake callback".to_string())?;
        }
        // Rows actually changed, not rows scanned: a full batch that settles
        // Unchanged would otherwise spin.
        if !(report.more_work && report.changed() > 0) {
            return Ok(());
        }
    }
    Ok(())
}
