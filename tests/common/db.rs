//! Raw access to the store for the tests that assert on the schema or the
//! rows themselves, over the kit's `AppDb` (the store has no `with_conn` of
//! its own any more). Included with `#[path = "common/db.rs"] mod db;` by the
//! files that do not need the whole harness.
#![allow(dead_code)]

use chassis::storage::AppDb;
use rusqlite::Connection;

pub trait RawStore {
    /// Runs `f` on a pooled read connection.
    fn with_conn<T>(&self, f: impl FnOnce(&Connection) -> T) -> T;

    /// [`RawStore::with_conn`] for a closure written against the queries,
    /// which fail with `anyhow` errors.
    fn read_any<T>(&self, f: impl FnOnce(&Connection) -> anyhow::Result<T>) -> anyhow::Result<T> {
        self.with_conn(f)
    }

    /// Runs `f` on the writer, inside one committed transaction: for the
    /// tests that arrange rows or connection state the engine has no verb
    /// for.
    fn with_writer<T>(&self, f: impl FnOnce(&Connection) -> T) -> T;
}

impl RawStore for AppDb {
    fn with_conn<T>(&self, f: impl FnOnce(&Connection) -> T) -> T {
        self.read_blocking(|conn| Ok(f(conn)))
            .expect("the store takes a read")
    }

    fn with_writer<T>(&self, f: impl FnOnce(&Connection) -> T) -> T {
        self.write_blocking(|tx| Ok(f(tx)))
            .expect("the store takes a write")
    }
}

impl RawStore for chassis::storage::Db {
    fn with_conn<T>(&self, f: impl FnOnce(&Connection) -> T) -> T {
        self.read_blocking(|conn| Ok(f(conn)))
            .expect("the store takes a read")
    }

    fn with_writer<T>(&self, f: impl FnOnce(&Connection) -> T) -> T {
        self.write_blocking(|tx| Ok(f(tx)))
            .expect("the store takes a write")
    }
}
