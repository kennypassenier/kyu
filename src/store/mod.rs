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

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::Connection;

pub const STORE_FILE_NAME: &str = "mailbox.db";

/// How long a statement waits for the writer before giving up. AR5 keeps
/// writes on one connection, so contention should be brief; this is the
/// backstop, not the mechanism.
const BUSY_TIMEOUT_MS: u32 = 5_000;

#[derive(Debug)]
pub struct Store {
    conn: Connection,
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
            conn,
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
        Ok(Self { conn, path: None })
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    pub fn conn_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }

    /// `None` for an in-memory store.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
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
