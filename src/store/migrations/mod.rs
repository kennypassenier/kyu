//! Forward-only schema migrations (AR3) with a snapshot taken first
//! (AR10).
//!
//! The version lives in `PRAGMA user_version`, so the schema needs no
//! bookkeeping table of its own. Rules:
//!
//! - A database older than the binary is migrated forward automatically.
//! - A database *newer* than the binary is refused with a remedy, because
//!   guessing at a schema we do not know is how data gets mangled.
//! - Before any migration runs on a populated database, the store is
//!   snapshotted. Without that, a bad image that migrates and then
//!   misbehaves cannot be rolled back: the older binary would refuse the
//!   newer schema and there would be nothing to return to.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use rusqlite::Connection;

mod schema;
use schema::*;

/// Snapshots kept before migrations; older ones are removed.
const SNAPSHOTS_KEPT: usize = 2;

/// The schema, one entry per version. Append only — never edit a
/// published entry, or databases in the field diverge from new ones.
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

/// Where a store stands against this binary: its schema version and the
/// one the binary knows. Read-only; a store written by a newer kyu is
/// refused here with the same remedy `migrate` gives (fix-check-1: this is
/// what `--check` asks, and it must not be the migrating path).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pending {
    pub current: u32,
    pub target: u32,
}

impl Pending {
    pub fn is_behind(&self) -> bool {
        self.current < self.target
    }
}

pub fn pending(conn: &Connection) -> Result<Pending> {
    let current: u32 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .context("cannot read the schema version (PRAGMA user_version)")?;
    let target = MIGRATIONS.len() as u32;
    if current > target {
        bail!("{}", newer_store_message(current, target));
    }
    Ok(Pending { current, target })
}

fn newer_store_message(current: u32, target: u32) -> String {
    format!(
        "this store was written by a newer kyu: schema version {current}, \
         but this binary knows version {target}. Roll back to the newer image, \
         or restore the snapshot this version's migration wrote \
         (kyu.pre-v*.db in the data directory). kyu never downgrades a \
         schema, because guessing at an unknown layout risks the messages in it."
    )
}

/// Brings `conn` up to the current schema version, returning it.
pub fn migrate(conn: &mut Connection, snapshot_dir: Option<&Path>) -> Result<u32> {
    migrate_with(conn, MIGRATIONS, snapshot_dir)
}

/// The body of [`migrate`], with the migration list as a parameter so that
/// tests can exercise upgrade paths (and the snapshot rule) without
/// waiting for the schema to grow a second version.
pub fn migrate_with(
    conn: &mut Connection,
    migrations: &[&str],
    snapshot_dir: Option<&Path>,
) -> Result<u32> {
    let current: u32 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .context("cannot read the schema version (PRAGMA user_version)")?;
    let target = migrations.len() as u32;

    if current > target {
        bail!("{}", newer_store_message(current, target));
    }

    if current == target {
        return Ok(current);
    }

    // Nothing to lose on a fresh database, so no snapshot for 0 -> 1.
    if let Some(dir) = snapshot_dir.filter(|_| current > 0) {
        snapshot(conn, dir, current)?;
    }

    for (index, sql) in migrations.iter().enumerate().skip(current as usize) {
        let version = index as u32 + 1;
        let tx = conn
            .transaction()
            .with_context(|| format!("cannot start the transaction for migration {version}"))?;
        tx.execute_batch(sql)
            .with_context(|| format!("migration {version} failed; the store is unchanged"))?;
        // PRAGMA statements take no parameters, so the version has to be
        // interpolated. It comes from this list's own index, never from input.
        tx.pragma_update(None, "user_version", version)
            .with_context(|| format!("cannot record schema version {version}"))?;
        tx.commit()
            .with_context(|| format!("cannot commit migration {version}"))?;
        tracing::info!(version, "schema migrated");
    }

    Ok(target)
}

/// Copies the live database with `VACUUM INTO`, which is consistent
/// without stopping traffic. Runs outside a transaction by necessity.
fn snapshot(conn: &Connection, dir: &Path, from_version: u32) -> Result<()> {
    let path = dir.join(format!("kyu.pre-v{from_version}.db"));
    if path.exists() {
        fs::remove_file(&path).with_context(|| {
            format!(
                "cannot replace the previous snapshot at {}. Remove it by hand, \
                 or free space in the data directory.",
                path.display()
            )
        })?;
    }

    conn.execute("VACUUM INTO ?1", [path.to_string_lossy().as_ref()])
        .with_context(|| {
            format!(
                "cannot write the pre-migration snapshot to {}. Check free space and \
                 that the data directory is writable; kyu refuses to migrate \
                 without one, so that a bad upgrade stays reversible.",
                path.display()
            )
        })?;

    tracing::info!(snapshot = %path.display(), "pre-migration snapshot written");
    prune_snapshots(dir)?;
    Ok(())
}

/// Keeps the newest [`SNAPSHOTS_KEPT`] snapshots. Deletions are logged
/// with their count — no silent cleanup (standing rule 12).
fn prune_snapshots(dir: &Path) -> Result<()> {
    let mut snapshots: Vec<_> = fs::read_dir(dir)
        .with_context(|| format!("cannot list the data directory {}", dir.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("kyu.pre-v") && name.ends_with(".db"))
        })
        .collect();

    if snapshots.len() <= SNAPSHOTS_KEPT {
        return Ok(());
    }

    snapshots.sort();
    let removable = snapshots.len() - SNAPSHOTS_KEPT;
    for path in snapshots.iter().take(removable) {
        fs::remove_file(path)
            .with_context(|| format!("cannot remove the old snapshot {}", path.display()))?;
    }
    tracing::info!(
        removed = removable,
        kept = SNAPSHOTS_KEPT,
        "pruned old pre-migration snapshots"
    );

    Ok(())
}
