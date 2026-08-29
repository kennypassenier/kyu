//! [L1] Store foundation: schema, migrations, pragmas and the ordering
//! guarantee. Real database files in temporary directories (standing rule
//! 9) — an in-memory database cannot prove anything about WAL.

use std::path::Path;

use kyu::engine::ids::MessageIds;
use kyu::store::{STORE_FILE_NAME, Store, migrations};
use rusqlite::Connection;

fn table_names(conn: &Connection) -> Vec<String> {
    let mut statement = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .expect("querying the schema must work");
    statement
        .query_map([], |row| row.get::<_, String>(0))
        .expect("the schema query must run")
        .map(|row| row.expect("a row"))
        .collect()
}

fn schema_version(conn: &Connection) -> u32 {
    conn.query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("reading user_version must work")
}

#[test]
fn l1_migration_creates_the_schema_in_an_empty_directory() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let store = Store::open(dir.path()).expect("opening a fresh store must succeed");

    // Not a literal: the schema grows, and this test is about the migration
    // runner reaching the current version rather than about which one it is.
    assert_eq!(
        store.with_conn(schema_version),
        migrations::MIGRATIONS.len() as u32
    );

    let tables = store.with_conn(table_names);
    for expected in ["topics", "subscriptions", "messages", "deliveries"] {
        assert!(
            tables.iter().any(|name| name == expected),
            "table {expected} is missing from {tables:?}"
        );
    }
    assert!(dir.path().join(STORE_FILE_NAME).exists());
}

#[test]
fn l1_opening_an_existing_store_is_idempotent() {
    let dir = tempfile::tempdir().expect("a temp dir");

    {
        let store = Store::open(dir.path()).expect("first open");
        store
            .with_conn(|conn| {
                conn.execute(
                    "INSERT INTO topics (name, retention_ms, created_at) VALUES (?1, ?2, ?3)",
                    ("notify.kenny", Option::<i64>::None, 1_700_000_000_000i64),
                )
            })
            .expect("a topic row");
    }

    let store = Store::open(dir.path()).expect("second open must not migrate again");
    assert_eq!(
        store.with_conn(schema_version),
        migrations::MIGRATIONS.len() as u32
    );

    let topics: i64 = store
        .with_conn(|conn| {
            conn.query_row(
                "SELECT count(*) FROM topics WHERE name = 'notify.kenny'",
                [],
                |row| row.get(0),
            )
        })
        .expect("counting topics");
    assert_eq!(topics, 1, "reopening must not disturb existing rows");
}

#[test]
fn l1_pragmas_are_set_explicitly_not_assumed() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let store = Store::open(dir.path()).expect("opening the store");
    store.with_conn(|conn| {
        let journal: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .expect("journal_mode");
        assert!(
            journal.eq_ignore_ascii_case("wal"),
            "expected WAL, got {journal:?}"
        );

        // 2 is FULL. Anything less and a confirmed publish could vanish in a
        // power cut, which is exactly what K12 promises it will not do.
        let synchronous: i64 = conn
            .query_row("PRAGMA synchronous", [], |row| row.get(0))
            .expect("synchronous");
        assert_eq!(synchronous, 2, "synchronous must be FULL");

        let foreign_keys: i64 = conn
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .expect("foreign_keys");
        assert_eq!(foreign_keys, 1, "foreign keys must be enforced");
    });
}

#[test]
fn l1_foreign_keys_actually_reject_an_orphan_delivery() {
    let store = Store::open_in_memory().expect("an in-memory store");

    let result = store.with_conn(|conn| {
        conn.execute(
            "INSERT INTO deliveries (msg_seq, sub_id, state) VALUES (?1, ?2, 'pending')",
            (9_999i64, 9_999i64),
        )
    });

    assert!(
        result.is_err(),
        "a delivery for a message that does not exist must be refused"
    );
}

#[test]
fn l1_a_newer_schema_is_refused_with_a_remedy() {
    let dir = tempfile::tempdir().expect("a temp dir");
    {
        let _ = Store::open(dir.path()).expect("create the store");
    }

    // Pretend a future kyu has been here.
    {
        let conn = Connection::open(dir.path().join(STORE_FILE_NAME)).expect("reopen");
        conn.pragma_update(None, "user_version", 99)
            .expect("bump the version");
    }

    let error = Store::open(dir.path()).expect_err("a newer schema must not be opened");
    let message = format!("{error:#}");
    assert!(message.contains("newer kyu"), "explains what happened");
    assert!(
        message.contains("kyu.pre-v") || message.contains("Roll back"),
        "carries a remedy: {message}"
    );
}

#[test]
fn l1_a_snapshot_is_written_before_migrating_a_populated_store() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut conn = Connection::open(dir.path().join(STORE_FILE_NAME)).expect("open");

    // Version 1 with data in it.
    let v1 = ["CREATE TABLE probe (id INTEGER PRIMARY KEY) STRICT;"];
    migrations::migrate_with(&mut conn, &v1, Some(dir.path())).expect("apply v1");
    conn.execute("INSERT INTO probe (id) VALUES (1)", [])
        .expect("a row");
    assert!(
        !snapshot_exists(dir.path()),
        "a fresh store has nothing to snapshot"
    );

    // Now a second version arrives.
    let v2 = [v1[0], "ALTER TABLE probe ADD COLUMN note TEXT;"];
    let version = migrations::migrate_with(&mut conn, &v2, Some(dir.path())).expect("apply v2");

    assert_eq!(version, 2);
    assert!(
        dir.path().join("kyu.pre-v1.db").exists(),
        "migrating a populated store must leave a rollback point"
    );

    // The snapshot must be a usable database holding the pre-migration state.
    let snapshot = Connection::open(dir.path().join("kyu.pre-v1.db")).expect("open snapshot");
    let rows: i64 = snapshot
        .query_row("SELECT count(*) FROM probe", [], |row| row.get(0))
        .expect("the snapshot must be queryable");
    assert_eq!(rows, 1);
    assert_eq!(
        schema_version(&snapshot),
        1,
        "the snapshot holds the schema as it was before the migration"
    );
}

fn snapshot_exists(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .expect("listing the dir")
        .filter_map(Result::ok)
        .any(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with("kyu.pre-v"))
        })
}

#[test]
fn l1_delivery_order_survives_a_clock_that_steps_backwards() {
    let store = Store::open_in_memory().expect("an in-memory store");
    let mut ids = MessageIds::new();
    store.with_conn(|conn| {
        conn.execute(
            "INSERT INTO topics (name, retention_ms, created_at) VALUES ('notify.kenny', NULL, 0)",
            [],
        )
        .expect("a topic");
        let topic_id: i64 = conn
            .query_row(
                "SELECT id FROM topics WHERE name = 'notify.kenny'",
                [],
                |r| r.get(0),
            )
            .expect("the topic id");

        // Published before the outage, then after a reboot whose clock reads an
        // hour earlier — the AR7 scenario.
        let before = ids.next(1_700_000_000_000);
        let after = ids.next(1_700_000_000_000 - 3_600_000);

        for (id, published_at) in [
            (before, 1_700_000_000_000i64),
            (after, 1_700_000_000_000 - 3_600_000),
        ] {
            conn.execute(
                "INSERT INTO messages (id, topic_id, payload, content_type, published_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
                (
                    id.to_string(),
                    topic_id,
                    b"{}".to_vec(),
                    "application/json",
                    published_at,
                ),
            )
            .expect("a message");
        }

        let by_seq: Vec<String> = {
            let mut statement = conn
                .prepare("SELECT id FROM messages ORDER BY seq")
                .expect("prepare");
            statement
                .query_map([], |row| row.get::<_, String>(0))
                .expect("query")
                .map(|row| row.expect("a row"))
                .collect()
        };
        assert_eq!(
            by_seq,
            vec![before.to_string(), after.to_string()],
            "insertion order must be publish order regardless of the clock"
        );

        let by_published_at: Vec<String> = {
            let mut statement = conn
                .prepare("SELECT id FROM messages ORDER BY published_at")
                .expect("prepare");
            statement
                .query_map([], |row| row.get::<_, String>(0))
                .expect("query")
                .map(|row| row.expect("a row"))
                .collect()
        };
        assert_ne!(
            by_published_at, by_seq,
            "this test is only meaningful while the timestamps disagree with \
         insertion order — that disagreement is why AR7 orders by seq"
        );
    });
}

#[test]
fn l1_the_delivery_state_enum_matches_ar9() {
    let store = Store::open_in_memory().expect("an in-memory store");
    store.with_conn(|conn| {
        conn.execute(
            "INSERT INTO topics (name, retention_ms, created_at) VALUES ('t', NULL, 0)",
            [],
        )
        .expect("a topic");
        conn.execute(
            "INSERT INTO subscriptions (topic_id, name, state, created_at)
         VALUES (1, 'sub', 'active', 0)",
            [],
        )
        .expect("a subscription");
        conn.execute(
            "INSERT INTO messages (id, topic_id, payload, published_at)
         VALUES ('01ARZ3NDEKTSV4RRFFQ69G5FAV', 1, x'7b7d', 0)",
            [],
        )
        .expect("a message");

        for state in ["pending", "claimed", "acked", "dead", "expired", "lapsed"] {
            conn.execute(
                "INSERT OR REPLACE INTO deliveries (msg_seq, sub_id, state) VALUES (1, 1, ?1)",
                [state],
            )
            .unwrap_or_else(|error| panic!("AR9 state {state:?} must be storable: {error}"));
        }

        let rejected = conn.execute(
            "INSERT OR REPLACE INTO deliveries (msg_seq, sub_id, state) VALUES (1, 1, 'nonsense')",
            [],
        );
        assert!(
            rejected.is_err(),
            "a state outside AR9 must be refused by the schema"
        );
    });
}
