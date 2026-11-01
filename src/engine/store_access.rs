//! How the engine reaches the store: the kit's `AppDb` behind two small
//! entry points, one for reads and one for writes, each speaking the
//! engine's own error type (AR4).
//!
//! The kit's connections, pragmas, backup ticket and failure record do the
//! work; this file only bridges the error types and decides what counts as
//! a sick store (W6).

use chassis::storage::Db;
use rusqlite::{Connection, Transaction};

use super::error::Result;
use super::{Engine, EngineError};

impl From<chassis::Error> for EngineError {
    fn from(error: chassis::Error) -> Self {
        Self::Store(error)
    }
}

impl Engine {
    /// The kit's database handle. Before the kit has started (it opens the
    /// file at start) this is an error with a remedy, not a panic.
    pub fn db(&self) -> std::result::Result<&Db, chassis::Error> {
        self.db.db()
    }

    /// The `AppDb` the engine runs on, for the shutdown checkpoint and the
    /// backup handler.
    pub fn app_db(&self) -> &chassis::storage::AppDb {
        &self.db
    }

    /// Runs a read against a pooled connection (the writer when every
    /// reader is busy or there is no pool, as in memory).
    ///
    /// Queries fail with `anyhow` errors; the original is handed back
    /// untouched, with its context chain, rather than flattened to text on
    /// the way through the kit's error type.
    pub fn read<T>(&self, f: impl FnOnce(&Connection) -> anyhow::Result<T>) -> anyhow::Result<T> {
        let mut failure = None;
        let outcome = self.db()?.read_blocking(|conn| {
            f(conn).map_err(|error| {
                let shown = format!("{error:#}");
                failure = Some(error);
                chassis::Error::internal(shown, "check the hub's logs for the failed query")
            })
        });
        match (outcome, failure) {
            (Ok(value), _) => Ok(value),
            (Err(_), Some(original)) => Err(original),
            (Err(kit), None) => Err(kit.into()),
        }
    }

    /// Every write goes through here so that a storage failure is recorded
    /// once, in one place (W6, the L1 gap closed at the Phase 7 gate).
    ///
    /// The kit runs `f` in one transaction and records the failures that are
    /// its own (the transaction cannot begin, the commit is refused); a
    /// refusal from `f` such as an unknown topic never marks the store
    /// sick. What the kit cannot see is a storage failure inside `f`, which
    /// surfaces as `Internal`: that one is recorded here. The typed errors
    /// are the caller being wrong, and a hub that called itself unhealthy
    /// over a 404 would be crying wolf. A `Store` error (a pause that held
    /// the write, a refused commit) comes from the kit and is already
    /// accounted for there.
    pub(super) fn write<T>(&self, f: impl FnOnce(&Transaction) -> Result<T>) -> Result<T> {
        let db = self.db()?;
        let result = db.write_blocking_as(f);
        if let Err(EngineError::Internal(error)) = &result {
            tracing::error!(error = ?error, "a store write failed");
            db.note_failure(&chassis::Error::internal(
                format!("{error:#}"),
                "check free space and that the data directory is writable",
            ));
        }
        result
    }
}
