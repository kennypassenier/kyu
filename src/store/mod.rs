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
//! Concurrency follows AR5: one writer connection, so delivery transitions
//! serialise without a lock of our own, and a small pool of readers so the
//! dashboard can scan a topic without ever blocking a publish.

mod backup;
pub mod dedup;
mod health;
mod inspect;
pub mod migrations;
mod pragmas;
pub mod queries;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use rusqlite::{Connection, Transaction};

pub const STORE_FILE_NAME: &str = "kyu.db";

/// How long a statement waits for the writer before giving up. AR5 keeps
/// writes on one connection, so contention should be brief; this is the
/// backstop, not the mechanism.
const BUSY_TIMEOUT_MS: u32 = 5_000;

/// Read connections for the dashboard (AR5). WAL allows readers to work
/// while the writer commits, so a dashboard scanning a topic never blocks a
/// publish — which is the whole reason the pool exists.
const READERS: usize = 4;

/// How long a failed write keeps the hub unhealthy. Long enough that a
/// scrape or an uptime check cannot miss it between polls, short enough that
/// the hub recovers by itself once the cause is gone.
const WRITE_FAILURE_WINDOW: std::time::Duration = std::time::Duration::from_secs(60);

#[derive(Debug)]
pub struct Store {
    writer: Mutex<Connection>,
    readers: Vec<Mutex<Connection>>,
    path: Option<PathBuf>,
    /// When a write last failed (W6, closed at the Phase 7 gate).
    ///
    /// The write probe takes SQLite's lock, which succeeds on a disk that is
    /// full but writable — the failure only appears when a commit needs a new
    /// page. Rather than predicting that, the store remembers that a write
    /// actually failed, so health reports what happened instead of what it
    /// guessed. This catches every cause, not only a full disk: a read-only
    /// remount, a permissions change, a corrupt file.
    last_write_failure: Mutex<Option<std::time::Instant>>,
}

pub use inspect::Inspection;
use pragmas::{Journal, apply_pragmas};

impl Store {
    /// Opens the store in `data_dir`, creating the directory and the
    /// database if needed, and migrates the schema forward.
    pub fn open(data_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(data_dir).with_context(|| {
            format!(
                "cannot create the data directory {}. Check that the volume is \
                 mounted and writable by this user, or point KYU_STATE_DIR \
                 somewhere else.",
                data_dir.display()
            )
        })?;

        let path = data_dir.join(STORE_FILE_NAME);
        let mut conn = Connection::open(&path).with_context(|| {
            format!(
                "cannot open the store at {}. Check file permissions, or point \
                 KYU_STATE_DIR at a writable directory.",
                path.display()
            )
        })?;

        apply_pragmas(&conn, Journal::Wal)?;
        migrations::migrate(&mut conn, Some(data_dir))?;

        let mut readers = Vec::with_capacity(READERS);
        for _ in 0..READERS {
            let reader = Connection::open(&path)
                .with_context(|| format!("cannot open a read connection to {}", path.display()))?;
            apply_pragmas(&reader, Journal::Existing)?;
            readers.push(Mutex::new(reader));
        }

        Ok(Self {
            writer: Mutex::new(conn),
            readers,
            path: Some(path),
            last_write_failure: Mutex::new(None),
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
        // An in-memory database is private to its connection, so the pool
        // would be a set of empty databases: reads share the writer instead.
        Ok(Self {
            writer: Mutex::new(conn),
            readers: Vec::new(),
            path: None,
            last_write_failure: Mutex::new(None),
        })
    }

    /// Runs a read against a pooled connection, falling back to the writer
    /// when every reader is busy or when there is no pool (in memory).
    pub fn read<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        for reader in &self.readers {
            if let Ok(conn) = reader.try_lock() {
                return f(&conn);
            }
        }
        if let Some(reader) = self.readers.first() {
            let conn = reader.lock().expect("a reader lock is never poisoned");
            return f(&conn);
        }
        let conn = self
            .writer
            .lock()
            .expect("the writer lock is never poisoned");
        f(&conn)
    }

    /// Folds the write-ahead log back into the database and truncates it
    /// (W12), so the files on disk stand still.
    ///
    /// Nothing about durability depends on this — K12 is proven by killing
    /// the process with SIGKILL and restarting it, and a checkpoint is not
    /// what makes a confirmed publish survive. What it buys is a quiescent
    /// moment: in WAL mode almost everything recent lives in `<name>.db-wal`,
    /// which a running hub keeps rewriting, and a file-level backup that
    /// copies it mid-write gets `file changed as we read it` rather than a
    /// database. After this returns, the log is empty and a plain `tar` or
    /// `cp` of the directory is a restorable copy.
    ///
    /// TRUNCATE rather than PASSIVE on purpose: PASSIVE gives up quietly if
    /// a reader is still holding the log open, which would make this look
    /// like it worked while leaving the very file we came to settle.
    pub fn checkpoint(&self) -> Result<()> {
        let conn = self
            .writer
            .lock()
            .expect("the writer lock is never poisoned");
        conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
            .context("cannot checkpoint the write-ahead log")
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
        let tx = match conn.transaction().context("cannot open a transaction") {
            Ok(tx) => tx,
            Err(error) => {
                self.record_write_failure();
                return Err(E::from(error));
            }
        };
        let value = f(&tx)?;
        if let Err(error) = tx.commit().context("cannot commit the transaction") {
            // The commit is where a full disk finally says so.
            self.record_write_failure();
            return Err(E::from(error));
        }
        Ok(value)
    }

    /// Direct access to the writer connection, for tests that assert on the
    /// schema itself. Ordinary reads go through [`Store::read`].
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
}

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
