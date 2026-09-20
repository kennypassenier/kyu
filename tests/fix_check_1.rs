//! [fix-check-1] `kyu --check` reads the store and writes nothing.
//!
//! On CT 109 (2026-09-20) the fix-state-1 drill ran `kyu --check` as root
//! against the live store: it opened the store through the serving path,
//! applied migration 5 and left a root-owned `kyu.pre-v4.db` behind — a
//! check that writes, in the exact place the kit's self-update also runs
//! `<staging> --check` before the swap. This test drives the real binary
//! against a store one schema behind and asserts it is left alone.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use kyu::store::migrations::{MIGRATIONS, migrate_with};
use rusqlite::Connection;

/// Runs the real binary with `--check` against `state_dir`, bounded.
fn check(state_dir: &std::path::Path) -> (Option<i32>, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_kyu"))
        .arg("--check")
        .env("KYU_STATE_DIR", state_dir)
        .env("KYU_TOKEN", "a-login-token-that-is-long-enough")
        .env(
            "KYU_SECRET_KEY",
            "abababababababababababababababababababababababababababababababab",
        )
        .env("KYU_LISTEN", "127.0.0.1:59998")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the binary must start");
    let mut stdout = child.stdout.take().expect("piped");
    let mut stderr = child.stderr.take().expect("piped");
    let out = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stdout.read_to_string(&mut s);
        s
    });
    let err = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        match child.try_wait().expect("waiting must work") {
            Some(status) => break Some(status),
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    };
    let mut output = out.join().unwrap_or_default();
    output.push_str(&err.join().unwrap_or_default());
    (status.and_then(|s| s.code()), output)
}

fn user_version(path: &std::path::Path) -> u32 {
    let conn = Connection::open(path).expect("open");
    conn.query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("user_version")
}

fn snapshots(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("list")
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("kyu.pre-v"))
        .collect();
    names.sort();
    names
}

#[test]
fn fix_check_1_check_reports_a_pending_migration_and_applies_nothing() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let db = dir.path().join("kyu.db");
    // A store one schema behind the binary, with rows in it.
    {
        let mut conn = Connection::open(&db).expect("create");
        let behind = MIGRATIONS.len() - 1;
        migrate_with(&mut conn, &MIGRATIONS[..behind], None).expect("older schema");
        conn.execute(
            "INSERT INTO topics (name, retention_ms, created_at) VALUES ('notify.kenny', NULL, 1)",
            [],
        )
        .expect("a row");
    }
    let before = user_version(&db);
    assert_eq!(before as usize, MIGRATIONS.len() - 1);

    let (code, output) = check(dir.path());
    assert_eq!(
        code,
        Some(0),
        "--check must pass on a readable, older store: {output}"
    );
    assert!(
        output.contains(&format!("schema {before}")) && output.contains("migrat"),
        "--check says a migration is pending, not that it did one: {output}"
    );

    assert_eq!(
        user_version(&db),
        before,
        "--check migrated the store: that is the CT 109 fault (fix-check-1)"
    );
    assert_eq!(
        snapshots(dir.path()),
        Vec::<String>::new(),
        "--check left a pre-migration snapshot behind, so it wrote"
    );
}

#[test]
fn fix_check_1_check_passes_before_the_first_start_without_creating_the_store() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let (code, output) = check(dir.path());
    assert_eq!(
        code,
        Some(0),
        "a state dir with no store yet is fine before the first start: {output}"
    );
    assert!(
        !dir.path().join("kyu.db").exists(),
        "--check created the store; it is created at start, not by a check"
    );
}
