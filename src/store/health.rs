//! The store's own health (W6): recent write failures and the write probe.

use anyhow::{Context, Result};

use super::{Store, WRITE_FAILURE_WINDOW};

impl Store {
    /// Called by the engine when a write failed for a storage reason rather
    /// than a business one — an unknown topic is not a sick store.
    pub fn record_write_failure(&self) {
        *self
            .last_write_failure
            .lock()
            .expect("the failure lock is never poisoned") = Some(std::time::Instant::now());
    }

    /// How long ago a write last failed, if that was recent enough to still
    /// mean something (W6).
    pub fn recent_write_failure(&self) -> Option<std::time::Duration> {
        let last = *self
            .last_write_failure
            .lock()
            .expect("the failure lock is never poisoned");
        last.map(|at| at.elapsed())
            .filter(|elapsed| *elapsed < WRITE_FAILURE_WINDOW)
    }

    /// Probes write access without writing anything (W6).
    ///
    /// `BEGIN IMMEDIATE` takes SQLite's reserved lock, which fails at once
    /// on a read-only store; rolling it straight back leaves no trace. A
    /// health check that only read would keep answering "fine" while every
    /// publish failed.
    ///
    /// What it catches: a store opened read-only, a read-only filesystem, a
    /// missing or unwritable data directory, and a lock another process
    /// refuses to release. What it does not: a disk that is full but
    /// writable — that surfaces at commit, so it shows up as failing
    /// publishes rather than here — and permissions changed *after* the file
    /// was opened, which POSIX checks only at open time.
    pub fn probe_writable(&self) -> Result<()> {
        let mut conn = self
            .writer
            .lock()
            .expect("the writer lock is never poisoned");
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .context("the store is not writable")?;
        tx.rollback().context("cannot release the write probe")?;
        Ok(())
    }
}
