//! The schema as a list, one entry per version (AR3). Append only: a
//! published entry is never edited, or databases in the field diverge from
//! new ones.
//!
//! The kit's runner applies the list (chassis-rs `Db::open_with`): each
//! step in its own transaction together with the `PRAGMA user_version`
//! bump, so the schema needs no bookkeeping table of its own. Rules the
//! runner keeps:
//!
//! - A database older than the binary is migrated forward automatically.
//! - A database *newer* than the binary is refused with a remedy, because
//!   guessing at a schema we do not know is how data gets mangled.
//! - Before any migration runs on a populated database, the store is
//!   copied to `kyu.pre-v<N>.db` (AR10, `store::options`). Without that, a
//!   bad image that migrates and then misbehaves cannot be rolled back: the
//!   older binary would refuse the newer schema and there would be nothing
//!   to return to.

use chassis::storage::Migration;

mod schema;
use schema::*;

/// The schema, one entry per version.
pub const MIGRATIONS: &[&str] = &[
    MIGRATION_1_INITIAL_SCHEMA,
    MIGRATION_2_EVENTS_TOPIC,
    MIGRATION_3_PER_SUBSCRIPTION_IDLE,
    MIGRATION_4_APPS,
    MIGRATION_5_EXPIRY_ANNOUNCEMENTS,
    MIGRATION_6_REMOVED_DELIVERIES,
    MIGRATION_7_ATTEMPTS_AND_STATS,
    MIGRATION_8_DEDUP_KEYS,
];

/// [`MIGRATIONS`] as the kit's migration list: the same SQL, one step per
/// version.
pub fn migrations() -> Vec<Migration> {
    MIGRATIONS.iter().map(|sql| Migration::sql(sql)).collect()
}
