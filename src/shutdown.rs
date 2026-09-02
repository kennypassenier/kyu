//! W12 · stopping on purpose.
//!
//! Until 2.1.0 kyu caught no signals at all, so `systemctl stop kyu` sent
//! SIGTERM and the process vanished where it stood. That was never a data
//! risk — the store runs WAL with `synchronous=FULL` and `tests/l5_crash.rs`
//! proves ten SIGKILLs in a row need no manual repair — but it cost three
//! things worth having: in-flight requests were cut off mid-response,
//! systemd recorded every stop as "killed by signal" rather than a clean
//! exit, and the files on disk never stood still, so a file-level backup
//! taken by the homelab caught `kyu.db-wal` mid-write and failed.
//!
//! Kenny made a graceful stop the norm for every Rust service in this
//! ecosystem on 2026-09-02, which is why kyu now has one.

use std::sync::Arc;
use std::time::Duration;

use crate::store::Store;

/// Resolves when the process is asked to stop.
///
/// Both signals, because they arrive from different places and mean the
/// same thing: SIGTERM from systemd or `docker stop`, Ctrl-C from someone
/// running the hub in a terminal.
///
/// Installing a handler replaces the default disposition for the whole
/// process, which is what makes a second SIGTERM during shutdown harmless:
/// it lands in a stream nobody is reading rather than killing the process
/// halfway through a checkpoint. The escape hatch stays systemd's
/// `TimeoutStopSec`, which is deliberately set above this hub's own budget.
pub async fn requested() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        let mut terminate = match signal(SignalKind::terminate()) {
            Ok(stream) => stream,
            Err(error) => {
                // Refusing to serve because a signal handler would not
                // install would be worse than serving without one.
                tracing::warn!(%error, "cannot listen for SIGTERM; stopping will not be graceful");
                std::future::pending::<()>().await;
                unreachable!()
            }
        };

        tokio::select! {
            _ = terminate.recv() => tracing::info!(signal = "SIGTERM", "stop requested"),
            result = tokio::signal::ctrl_c() => match result {
                Ok(()) => tracing::info!(signal = "SIGINT", "stop requested"),
                Err(error) => tracing::warn!(%error, "cannot listen for Ctrl-C"),
            },
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
        tracing::info!(signal = "SIGINT", "stop requested");
    }
}

/// Settles the store after the server has stopped accepting requests.
///
/// Bounded on purpose. A stop that hangs is worse than a stop that is
/// incomplete: systemd resolves the first one with SIGKILL after a delay
/// nobody remembers configuring, and by then the operator has learned that
/// stopping this service is unpredictable. So the checkpoint gets a budget,
/// and blowing it produces one loud line and an exit anyway — never a
/// failure, because the data is safe either way and an exit code of 1 here
/// would make systemd report a clean stop as a crash.
pub async fn settle(store: Arc<Store>, budget: Duration) {
    let checkpoint = tokio::task::spawn_blocking(move || store.checkpoint());

    match tokio::time::timeout(budget, checkpoint).await {
        Ok(Ok(Ok(()))) => tracing::info!("store checkpointed; stopping"),
        Ok(Ok(Err(error))) => {
            tracing::warn!(%error, "could not checkpoint the store; stopping anyway. The data is intact — WAL and synchronous=FULL do not depend on this — but the write-ahead log is still on disk, so a file-level backup of the data directory may not restore. Take one with POST /api/backup instead.")
        }
        Ok(Err(error)) => tracing::warn!(%error, "the checkpoint task failed; stopping anyway"),
        Err(_) => tracing::warn!(
            budget_ms = budget.as_millis() as u64,
            "the checkpoint did not finish within KYU_SHUTDOWN_TIMEOUT_MS; stopping anyway. Something is holding the store open longer than expected; raise KYU_SHUTDOWN_TIMEOUT_MS if this host has slow storage."
        ),
    }
}
