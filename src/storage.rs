//! What kyu declares to the kit's storage layer: its database (the schema,
//! the pool shape, the copy before a migration), its data as kinds on the
//! Storage page, and the browser keys that follow the login.
//!
//! The kit owns the handle (`AppDb`, since chassis-rs 3.11.0): it opens
//! `kyu.db` at start, migrates it, copies it first when it is behind, and
//! adds the budget, the Storage page with its preview and typed-confirm
//! delete, the backup-pause checkpoint and the preferences table
//! `chassis_prefs`. kyu keeps no handle of its own.

use std::path::Path;

use chassis::storage::{AppDb, AppDbSpec, BrowserKeys, DataKind, Filter, TimeFormat};
use chassis::{App, Error};

use crate::store::migrations::migrations;
use crate::store::{STORE_FILE_NAME, options};

/// kyu's database as the kit opens it: the schema, four readers and a copy
/// before every migration.
pub fn spec() -> AppDbSpec {
    AppDbSpec::new(migrations()).with_options(options())
}

/// Opens the store in `data_dir` the way a start does (migrating it
/// forward, copying it first when it is behind), without a running kit.
/// For the one-time token import, which has to read the 2.x `apps` table
/// before the kit opens its client store, and for tests.
pub fn open(data_dir: &Path) -> Result<AppDb, Error> {
    AppDb::open(&data_dir.join(STORE_FILE_NAME), spec(), kinds())
}

/// An in-memory store for fast logic tests.
///
/// Write-ahead logging does not apply to memory databases, so anything
/// asserting durability or crash behaviour must use [`open`] with a real
/// file (standing rule 9).
pub fn open_in_memory() -> Result<AppDb, Error> {
    AppDb::open_in_memory(spec(), kinds())
}

/// The data kinds. None has a "finished by its own definition" rule: kyu's
/// sweeper (`crate::sweeper`) already prunes on its own caps, so the kit
/// removes nothing by itself. These only make the data visible and let Kenny
/// clean it by hand, behind the kit's preview and typed confirmation.
/// `stats_daily` is left out: its `day` column is a day number, which the
/// kit's time formats (seconds, milliseconds, RFC 3339) cannot compare.
pub fn kinds() -> Vec<DataKind> {
    vec![
        DataKind::new("messages", "Messages")
            .description(
                "Published messages with their deliveries and attempts. Deleting a message \
                 deletes its deliveries and attempts with it; subscriptions that had not \
                 received it yet never will.",
            )
            .unit("messages")
            .table("messages")
            .time_column("published_at", TimeFormat::UnixMillis),
        DataKind::new("attempts", "Delivery attempts")
            .description("The attempt history shown on a message: one row per claim.")
            .unit("attempts")
            .table("attempts")
            .time_column("claimed_at", TimeFormat::UnixMillis)
            .filter(Filter::choice(
                "outcome",
                "Outcome",
                "outcome",
                ["in_flight", "acked", "nacked", "expired", "dead"],
            )),
        DataKind::new("dedup_keys", "Dedup keys")
            .description(
                "The dedup keys publishers sent with Kyu-Dedup-Key and the message each \
                 one produced. While a key is remembered, a repeat publish on that topic \
                 is suppressed. Deleting a key lets the next publish with it through.",
            )
            .unit("keys")
            .table("dedup_keys")
            .time_column("published_at", TimeFormat::UnixMillis),
        DataKind::new("stats_hourly", "Hourly statistics")
            .description(
                "Published and dead-letter counters per topic and hour, behind the charts.",
            )
            .unit("hours")
            .table("stats_hourly")
            .time_column("hour", TimeFormat::UnixMillis),
        DataKind::new("stats_samples", "Backlog samples")
            .description("The periodic backlog and in-flight samples behind the status charts.")
            .unit("samples")
            .table("stats_samples")
            .time_column("at", TimeFormat::UnixMillis),
    ]
}

/// The browser keys that sync: every kp-themes `data-kp-remember` choice
/// (table sorts and columns, pane widths, the auto-refresh choice, the
/// publish draft, the tour-done flag), all under `kp-remember:`. kyu's own
/// scripts keep nothing in the browser. The theme key (`theme`) stays in the
/// browser: it is read before login, when no preference can be fetched.
/// Never a secret; the kit refuses a declaration that names one.
pub fn browser_keys() -> BrowserKeys {
    BrowserKeys::new().prefix("kp-remember:")
}

/// Declare everything in one place, for the binary and the test harness.
pub fn declare(app: &mut App) -> Result<AppDb, Error> {
    let db = app.app_db(spec())?;
    for kind in kinds() {
        app.storage_kind(kind);
    }
    app.sync_browser_keys(browser_keys());
    Ok(db)
}
