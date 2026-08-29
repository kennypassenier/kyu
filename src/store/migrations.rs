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

/// Snapshots kept before migrations; older ones are removed.
const SNAPSHOTS_KEPT: usize = 2;

/// The schema, one entry per version. Append only — never edit a
/// published entry, or databases in the field diverge from new ones.
pub const MIGRATIONS: &[&str] = &[
    MIGRATION_1_INITIAL_SCHEMA,
    MIGRATION_2_EVENTS_TOPIC,
    MIGRATION_3_PER_SUBSCRIPTION_IDLE,
    MIGRATION_4_APPS,
];

/// AR3's four tables. Times are integer milliseconds since the Unix epoch
/// (AR7). `STRICT` makes SQLite enforce the column types instead of
/// coercing them.
const MIGRATION_1_INITIAL_SCHEMA: &str = r#"
CREATE TABLE topics (
    id           INTEGER PRIMARY KEY,
    name         TEXT    NOT NULL UNIQUE,
    retention_ms INTEGER,
    created_at   INTEGER NOT NULL
) STRICT;

CREATE TABLE subscriptions (
    id           INTEGER PRIMARY KEY,
    topic_id     INTEGER NOT NULL REFERENCES topics(id) ON DELETE CASCADE,
    name         TEXT    NOT NULL,
    state        TEXT    NOT NULL CHECK (state IN ('active', 'flagged', 'archived')),
    lease_ms     INTEGER,
    max_attempts INTEGER,
    backoff_ms   INTEGER,
    ttl_ms       INTEGER,
    created_at   INTEGER NOT NULL,
    last_poll_at INTEGER,
    UNIQUE (topic_id, name)
) STRICT;

-- `seq` is the rowid: insertion order, which is what delivery order is
-- based on (AR7). `id` is the public ULID and is never used for ordering.
CREATE TABLE messages (
    seq          INTEGER PRIMARY KEY AUTOINCREMENT,
    id           TEXT    NOT NULL UNIQUE,
    topic_id     INTEGER NOT NULL REFERENCES topics(id) ON DELETE CASCADE,
    payload      BLOB    NOT NULL,
    content_type TEXT,
    published_at INTEGER NOT NULL,
    due_at       INTEGER
) STRICT;

-- Fan-out is materialized: one row per (message, subscription), created in
-- the same transaction as the message (AR3). Every delivery state
-- transition is then a single guarded UPDATE (AR9).
CREATE TABLE deliveries (
    msg_seq          INTEGER NOT NULL REFERENCES messages(seq) ON DELETE CASCADE,
    sub_id           INTEGER NOT NULL REFERENCES subscriptions(id) ON DELETE CASCADE,
    state            TEXT    NOT NULL CHECK (state IN
                         ('pending', 'claimed', 'acked', 'dead', 'expired', 'lapsed')),
    attempts         INTEGER NOT NULL DEFAULT 0,
    lease_expires_at INTEGER,
    next_attempt_at  INTEGER,
    dead_at          INTEGER,
    expired_at       INTEGER,
    PRIMARY KEY (msg_seq, sub_id)
) STRICT;

-- The claim query: oldest deliverable row for one subscription.
CREATE INDEX deliveries_claimable
    ON deliveries (sub_id, state, next_attempt_at, msg_seq);

-- The sweeper's scans: expiring leases and settling states.
CREATE INDEX deliveries_by_state
    ON deliveries (state, lease_expires_at);

-- Retention and replay walk a topic in insertion order.
CREATE INDEX messages_by_topic
    ON messages (topic_id, seq);
"#;

/// The hub's own event topic (W11) exists from the start, so that something
/// can subscribe to it *before* the first event happens. Creating it lazily
/// would mean the only way to start listening was to wait for a failure.
///
/// `created_at` is 0: it has been there since the schema was.
const MIGRATION_2_EVENTS_TOPIC: &str = r#"
INSERT INTO topics (name, retention_ms, created_at) VALUES ('kyu.events', NULL, 0);
"#;

/// K11's idle thresholds start as hub-wide defaults, but the knowledge that
/// a consumer only polls monthly lives with that consumer — so they are
/// overridable per subscription, like every other policy field (K7).
const MIGRATION_3_PER_SUBSCRIPTION_IDLE: &str = r#"
ALTER TABLE subscriptions ADD COLUMN idle_flag_ms INTEGER;
ALTER TABLE subscriptions ADD COLUMN idle_archive_ms INTEGER;
"#;

/// W2's registered apps. `token` holds the ciphertext from
/// `crypto::SecretKey::seal`, never the token itself — the column is a BLOB
/// because that is what it is, and calling it anything friendlier would
/// invite someone to read it.
///
/// Revoking keeps the row and stamps `revoked_at`, rather than deleting it:
/// "this app used to exist and I turned it off" is the thing you want to
/// see six months later, and a deleted row cannot tell you that. The unique
/// index therefore covers live apps only, so a revoked name can be reused.
const MIGRATION_4_APPS: &str = r#"
CREATE TABLE apps (
    id         INTEGER PRIMARY KEY,
    name       TEXT    NOT NULL,
    token      BLOB    NOT NULL,
    created_at INTEGER NOT NULL,
    revoked_at INTEGER
) STRICT;

CREATE UNIQUE INDEX apps_live_name ON apps (name) WHERE revoked_at IS NULL;
"#;

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
        bail!(
            "this store was written by a newer kyu: schema version {current}, \
             but this binary knows version {target}. Roll back to the newer image, \
             or restore the snapshot this version's migration wrote \
             (kyu.pre-v*.db in the data directory). kyu never downgrades a \
             schema, because guessing at an unknown layout risks the messages in it."
        );
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
