//! The pragmas every connection gets, set explicitly rather than inherited (K12).

use anyhow::{Context, Result};
use rusqlite::Connection;

use super::BUSY_TIMEOUT_MS;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Journal {
    Wal,
    Memory,
    /// A second connection to a database whose journal mode is already set.
    Existing,
}

/// Sets every pragma the durability and correctness promises depend on.
/// Their effective values are asserted by tests rather than assumed.
pub(super) fn apply_pragmas(conn: &Connection, journal: Journal) -> Result<()> {
    conn.busy_timeout(std::time::Duration::from_millis(BUSY_TIMEOUT_MS.into()))
        .context("cannot set busy_timeout")?;

    // Referential integrity is off by default in SQLite, and the delivery
    // rows of AR3 lean on it: a delivery must never outlive its message.
    conn.pragma_update(None, "foreign_keys", "ON")
        .context("cannot enable foreign_keys")?;

    if journal == Journal::Existing {
        return Ok(());
    }

    if journal == Journal::Wal {
        // journal_mode returns the mode it settled on, so read it back
        // rather than trusting the write.
        let mode: String = conn
            .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
            .context("cannot switch the journal to WAL")?;
        anyhow::ensure!(
            mode.eq_ignore_ascii_case("wal"),
            "SQLite refused write-ahead logging and settled on {mode:?}. The store \
             must be on a filesystem that supports it — a local volume, not a \
             network mount — because K12's durability promise depends on WAL plus \
             synchronous=FULL."
        );
    }

    // The K12 contract: a confirmed publish has been fsynced. NORMAL would
    // let a committed transaction vanish in a power cut.
    conn.pragma_update(None, "synchronous", "FULL")
        .context("cannot set synchronous=FULL")?;

    Ok(())
}
