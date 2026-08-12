//! Persistence (AR3): the schema, its migrations and every SQL statement
//! in the project.
//!
//! All state — messages, subscriptions, delivery rows, dead letters —
//! lives in one SQLite database, so each delivery transition is a single
//! transaction and the invariants are checkable with one `SELECT`.
//!
//! Durability (K12) rests on pragmas that are set explicitly at every
//! open rather than inherited: build defaults for `synchronous` vary, and
//! "confirmed means on disk" cannot depend on how someone compiled
//! SQLite. Ordering rests on `messages.seq`, the rowid, because a clock
//! can move backwards after a power cut (AR7).
//!
//! L1 lands opening, pragmas and migrations. The reader pool and single
//! writer connection of AR5 arrive with the verbs in L2.

pub mod migrations;
pub mod queries;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use rusqlite::{Connection, Transaction};

pub const STORE_FILE_NAME: &str = "mailbox.db";

/// How long a statement waits for the writer before giving up. AR5 keeps
/// writes on one connection, so contention should be brief; this is the
/// backstop, not the mechanism.
const BUSY_TIMEOUT_MS: u32 = 5_000;

/// One writer connection (AR5): SQLite WAL allows many readers but a
/// single writer, and funnelling writes through one connection makes the
/// delivery transitions serialise without any lock of our own.
///
/// The reader pool AR5 also calls for arrives with L7, the first
/// read-heavy consumer; the verbs are all writes.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l5_the_write_probe_passes_on_a_healthy_store() {
        let store = Store::open_in_memory().expect("a store");
        store.probe_writable().expect("a fresh store is writable");
    }

    #[test]
    fn l5_the_write_probe_fails_when_writes_are_refused() {
        let store = Store::open_in_memory().expect("a store");
        // query_only makes SQLite refuse writes on this connection, which is
        // how it behaves on a read-only store.
        store.with_conn(|conn| {
            conn.pragma_update(None, "query_only", "ON")
                .expect("the pragma")
        });

        let error = store
            .probe_writable()
            .expect_err("a store that refuses writes must not pass the probe");
        assert!(
            format!("{error:#}").contains("not writable"),
            "and it must say so plainly: {error:#}"
        );
    }
}

#[derive(Debug)]
pub struct Store {
    writer: Mutex<Connection>,
    path: Option<PathBuf>,
}

impl Store {
    /// Opens the store in `data_dir`, creating the directory and the
    /// database if needed, and migrates the schema forward.
    pub fn open(data_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(data_dir).with_context(|| {
            format!(
                "cannot create the data directory {}. Check that the volume is \
                 mounted and writable by this user, or point MAILBOX_DATA_DIR \
                 somewhere else.",
                data_dir.display()
            )
        })?;

        let path = data_dir.join(STORE_FILE_NAME);
        let mut conn = Connection::open(&path).with_context(|| {
            format!(
                "cannot open the store at {}. Check file permissions, or point \
                 MAILBOX_DATA_DIR at a writable directory.",
                path.display()
            )
        })?;

        apply_pragmas(&conn, Journal::Wal)?;
        migrations::migrate(&mut conn, Some(data_dir))?;

        Ok(Self {
            writer: Mutex::new(conn),
            path: Some(path),
        })
    }

    /// An in-memory store for fast logic tests.
    ///
    /// Write-ahead logging does not apply to memory databases, so anything
    /// asserting durability or crash behaviour must use [`Store::open`]
    /// with a real file (standing rule 9).
    pub fn open_in_memory() -> Result<Self> {
        let mut conn =
            Connection::open_in_memory().context("cannot open an in-memory SQLite database")?;
        apply_pragmas(&conn, Journal::Memory)?;
        migrations::migrate(&mut conn, None)?;
        Ok(Self {
            writer: Mutex::new(conn),
            path: None,
        })
    }

    /// Runs `f` inside one write transaction, committing if it returns
    /// `Ok`. This is the only way to write: it is what makes a delivery
    /// transition atomic (AR3).
    ///
    /// The error type is the caller's, so the engine can return its own
    /// typed errors (AR4) while transaction failures convert in.
    pub fn write<T, E>(
        &self,
        f: impl FnOnce(&Transaction) -> std::result::Result<T, E>,
    ) -> std::result::Result<T, E>
    where
        E: From<anyhow::Error>,
    {
        let mut conn = self
            .writer
            .lock()
            .expect("the writer lock is never poisoned");
        let tx = conn
            .transaction()
            .context("cannot open a transaction")
            .map_err(E::from)?;
        let value = f(&tx)?;
        tx.commit()
            .context("cannot commit the transaction")
            .map_err(E::from)?;
        Ok(value)
    }

    /// Direct connection access, for reads and for tests that assert on the
    /// schema itself. L7 replaces this with a reader pool (AR5).
    pub fn with_conn<T>(&self, f: impl FnOnce(&Connection) -> T) -> T {
        let conn = self
            .writer
            .lock()
            .expect("the writer lock is never poisoned");
        f(&conn)
    }

    /// `None` for an in-memory store.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Journal {
    Wal,
    Memory,
}

/// Sets every pragma the durability and correctness promises depend on.
/// Their effective values are asserted by tests rather than assumed.
fn apply_pragmas(conn: &Connection, journal: Journal) -> Result<()> {
    conn.busy_timeout(std::time::Duration::from_millis(BUSY_TIMEOUT_MS.into()))
        .context("cannot set busy_timeout")?;

    // Referential integrity is off by default in SQLite, and the delivery
    // rows of AR3 lean on it: a delivery must never outlive its message.
    conn.pragma_update(None, "foreign_keys", "ON")
        .context("cannot enable foreign_keys")?;

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
