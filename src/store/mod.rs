//! Persistence (AR3): the schema and every SQL statement in the project.
//!
//! All state — messages, subscriptions, delivery rows, dead letters —
//! lives in one SQLite database, so each delivery transition is a single
//! transaction and the invariants are checkable with one `SELECT`.
//!
//! The handle itself is the kit's (chassis-rs `storage::Db`, reached
//! through `AppDb`): it owns the connections, the durability pragmas
//! (K12: WAL plus `synchronous=FULL`, set at every open), the migration
//! runner, the copy before a migration, the write probe, the failure
//! record and the verified backup. What stays here is what only kyu can
//! know: the schema ([`migrations`]), the statements ([`queries`],
//! [`dedup`]) and the pool shape ([`options`]). Ordering rests on
//! `messages.seq`, the rowid, because a clock can move backwards after a
//! power cut (AR7).
//!
//! Concurrency follows AR5: one writer connection, so delivery transitions
//! serialise without a lock of our own, and a small pool of readers so the
//! dashboard can scan a topic without ever blocking a publish.

pub mod dedup;
pub mod migrations;
pub mod queries;

use chassis::storage::DbOptions;

pub const STORE_FILE_NAME: &str = "kyu.db";

/// Read connections for the dashboard (AR5). WAL allows readers to work
/// while the writer commits, so a dashboard scanning a topic never blocks a
/// publish — which is the whole reason the pool exists.
pub const READERS: usize = 4;

/// Pre-migration copies kept (`kyu.pre-v<N>.db`, AR10): the newest two, so
/// the copy of the previous upgrade is still there when the next one fails.
pub const SNAPSHOTS_KEPT: usize = 2;

/// How kyu asks the kit to open its store: [`READERS`] readers and a copy
/// before every schema change.
///
/// The readers stay read-only: every statement that writes takes a
/// `&Transaction` (see `queries`), so nothing a dashboard "read" runs can
/// write, and the kit's default is the stricter one.
pub fn options() -> DbOptions {
    DbOptions::new()
        .readers(READERS)
        .pre_migration_snapshots(SNAPSHOTS_KEPT)
}
