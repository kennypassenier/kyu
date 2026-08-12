//! The background sweeper (AR5): one task, a coarse tick, bounded batches.
//!
//! It drives the transitions nobody asks for over HTTP — leases running
//! out, retries becoming dead letters, messages passing their TTL — through
//! the same [`Engine::sweep`] the mocked-clock tests exercise, so the
//! behaviour under test is the behaviour in production.
//!
//! Shell rather than domain: the timer and the runtime live here, the
//! decisions live in the engine (AR1).

use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinHandle;

use crate::engine::Engine;

/// How often the sweeper looks. AR5 caps this at one second: it is the
/// worst-case delay before an expired lease becomes visible again.
pub const TICK: Duration = Duration::from_secs(1);

/// Rows per transaction. Small enough that the single writer connection is
/// never held long, so a publish never queues behind a sweep and a hard kill
/// mid-sweep has almost nothing to roll back (AR5, K12).
pub const BATCH_LIMIT: usize = 500;

/// Runs until the returned handle is dropped or aborted.
///
/// `wake` is called with the subscriptions that gained a message again, so
/// a waiting long poll answers at once instead of sitting out its timeout.
/// It is a callback rather than a direct dependency on the notifier so this
/// module stays independent of the HTTP layer.
pub fn spawn<F>(engine: Arc<Engine>, wake: F) -> JoinHandle<()>
where
    F: Fn(&[(String, String)]) + Send + 'static,
{
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(TICK);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            ticker.tick().await;

            // Keep going while batches come back full: the tick paces idle
            // polling, not the draining of a backlog.
            loop {
                let engine = engine.clone();
                let swept = tokio::task::spawn_blocking(move || engine.sweep(BATCH_LIMIT)).await;

                let report = match swept {
                    Ok(Ok(report)) => report,
                    Ok(Err(error)) => {
                        // A failing sweep must not kill the task: the next
                        // tick tries again, and the error is on the record.
                        tracing::error!(error = ?error, "the sweep failed");
                        break;
                    }
                    Err(join_error) => {
                        tracing::error!(error = %join_error, "the sweep task failed to run");
                        break;
                    }
                };

                if report.changed() > 0 {
                    tracing::info!(
                        redelivered = report.redelivered,
                        dead_lettered = report.dead_lettered,
                        expired = report.expired,
                        "sweep settled overdue deliveries"
                    );
                }

                if !report.wake.is_empty() {
                    wake(&report.wake);
                }

                if !report.more_work {
                    break;
                }
            }
        }
    })
}
