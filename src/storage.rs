//! What kyu declares to the kit's storage layer (chassis-rs 3.11.0 and
//! 3.12.0): its data as kinds on the Storage page, and the browser keys that
//! follow the login.
//!
//! kyu keeps its own store (`crate::store`, `kyu.db`). The kit's
//! `App::app_db` opens the same file under the same migration list, so the
//! schema does not change and nothing moves: `Store::open` has already
//! migrated the file to the current schema before the kit opens it, which
//! leaves the kit with nothing to apply. What the kit adds is the budget,
//! the Storage page with its preview and typed-confirm delete, the backup
//! pause checkpoint, and the preferences table `chassis_prefs`.

use chassis::storage::{AppDb, AppDbSpec, BrowserKeys, DataKind, Filter, Migration, TimeFormat};
use chassis::{App, Error};

use crate::store::migrations::MIGRATIONS;

/// kyu's schema as the kit's migration list: the same SQL, one step per
/// version, so the kit and `Store::open` can never disagree about the
/// version number of the file they share.
pub fn migrations() -> Vec<Migration> {
    MIGRATIONS.iter().map(|sql| Migration::sql(sql)).collect()
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
    let db = app.app_db(AppDbSpec::new(migrations()))?;
    for kind in kinds() {
        app.storage_kind(kind);
    }
    app.sync_browser_keys(browser_keys());
    Ok(db)
}
