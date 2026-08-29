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

pub mod migrations;
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

impl Store {
    /// Opens the store in `data_dir`, creating the directory and the
    /// database if needed, and migrates the schema forward.
    pub fn open(data_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(data_dir).with_context(|| {
            format!(
                "cannot create the data directory {}. Check that the volume is \
                 mounted and writable by this user, or point KYU_DATA_DIR \
                 somewhere else.",
                data_dir.display()
            )
        })?;

        let path = data_dir.join(STORE_FILE_NAME);
        let mut conn = Connection::open(&path).with_context(|| {
            format!(
                "cannot open the store at {}. Check file permissions, or point \
                 KYU_DATA_DIR at a writable directory.",
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

    /// W8 · writes a consistent copy of the store while the hub keeps
    /// running.
    ///
    /// `VACUUM INTO` produces a complete database file, not a snapshot of
    /// bytes mid-write, so a backup taken under load is restorable. Copying
    /// the file by hand is the thing this exists to stop anyone doing.
    pub fn backup_to(&self, path: &Path) -> Result<u64> {
        if path.exists() {
            anyhow::bail!(
                "{} already exists. VACUUM INTO refuses to overwrite, so choose \
                 another name or remove the old backup first.",
                path.display()
            );
        }
        let conn = self
            .writer
            .lock()
            .expect("the writer lock is never poisoned");
        conn.execute("VACUUM INTO ?1", [path.to_string_lossy().as_ref()])
            .with_context(|| {
                format!(
                    "cannot write the backup to {}. Check free space and that the \
                     directory is writable by this user.",
                    path.display()
                )
            })?;
        let bytes = std::fs::metadata(path)
            .with_context(|| format!("cannot stat the backup at {}", path.display()))?
            .len();

        // Open what was just written and ask SQLite whether it is intact. A
        // backup is only a backup if it restores; reporting success for a
        // file that merely exists is how a truncated copy gets discovered
        // on the day it is needed.
        let verify = Connection::open(path)
            .with_context(|| format!("the backup at {} cannot be opened", path.display()))?;
        let integrity: String = verify
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .with_context(|| format!("the backup at {} cannot be checked", path.display()))?;
        anyhow::ensure!(
            integrity == "ok",
            "the backup at {} is not a sound database ({integrity}). It has been left \
             in place for inspection; take another one, and check free space first.",
            path.display()
        );

        Ok(bytes)
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
    /// A second connection to a database whose journal mode is already set.
    Existing,
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
