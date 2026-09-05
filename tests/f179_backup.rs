//! [F179] The backup helper, against a real hub.
//!
//! `deploy/kyu-backup` used to hardcode `/etc/kyu/kyu.env` and
//! `/var/lib/kyu`. The homelab's adoption of LXC 109 moved both, and every
//! nightly run from 2026-09-01 failed with
//! `grep: /etc/kyu/kyu.env: No such file or directory` — for two nights,
//! while `kyu-backup.timer` went on reporting that it had fired. The script
//! lived only on that machine, in no repository, so nothing could ever have
//! caught it.
//!
//! These tests exist so that cannot happen twice: the script is in the repo,
//! it takes every path from its environment, and it refuses loudly rather
//! than guessing when something is missing (standing rules 12 and 28).

use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command, Output};
use std::time::Duration;

struct Hub {
    process: Child,
    port: u16,
}

impl Drop for Hub {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

fn free_port() -> u16 {
    use std::sync::atomic::{AtomicU16, Ordering};
    static NEXT: AtomicU16 = AtomicU16::new(63_000);
    for _ in 0..500 {
        let candidate = NEXT.fetch_add(1, Ordering::Relaxed);
        if TcpListener::bind(("127.0.0.1", candidate)).is_ok() {
            return candidate;
        }
    }
    panic!("no free port in the test range — is something holding 63000+?");
}

const TOKEN: &str = "a-token-long-enough-for-the-door";
/// 32 bytes of hex: the key app tokens are encrypted with.
const KEY: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

async fn start(data_dir: &Path) -> Hub {
    let port = free_port();
    let process = Command::new(env!("CARGO_BIN_EXE_kyu"))
        .env("KYU_STATE_DIR", data_dir)
        .env("KYU_LISTEN", format!("127.0.0.1:{port}"))
        .env("KYU_TOKEN", TOKEN)
        .env("KYU_SECRET_KEY", KEY)
        .env("KYU_LOG", "warn")
        .spawn()
        .expect("the kyu binary must start");
    let hub = Hub { process, port };
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        if let Ok(response) = reqwest::get(format!("http://127.0.0.1:{}/healthz", hub.port)).await
            && response.status() == 200
        {
            return hub;
        }
    }
    panic!("the hub never became healthy");
}

/// Runs the shipped script exactly as the unit would, with the environment
/// systemd's `EnvironmentFile=` supplies — and nothing else.
fn run_backup(hub: &Hub, data_dir: &Path, extra: &[(&str, &str)]) -> Output {
    let mut command = Command::new("bash");
    command
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/deploy/kyu-backup"))
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("KYU_TOKEN", TOKEN)
        .env("KYU_LISTEN", format!("127.0.0.1:{}", hub.port))
        .env("KYU_STATE_DIR", data_dir);
    for (name, value) in extra {
        if value.is_empty() {
            command.env_remove(name);
        } else {
            command.env(name, value);
        }
    }
    command.output().expect("the script must run")
}

fn backups(dir: &Path) -> Vec<String> {
    let mut found: Vec<String> = std::fs::read_dir(dir)
        .expect("the data dir must exist")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|name| name.starts_with("kyu.backup-") && name.ends_with(".db"))
        .collect();
    found.sort();
    found
}

#[tokio::test]
async fn f179_the_script_takes_its_paths_from_the_environment_and_writes_a_backup() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let hub = start(dir.path()).await;

    let output = run_backup(&hub, dir.path(), &[]);
    assert!(
        output.status.success(),
        "the backup must succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        backups(dir.path()).len(),
        1,
        "exactly one backup file must appear in KYU_STATE_DIR"
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("backup written"),
        "and it must say so: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[tokio::test]
async fn f179_a_missing_variable_is_refused_loudly_and_never_guessed() {
    // The fault itself: the script assumed a path, the deployment moved, and
    // the assumption failed in a way nobody read. Refusing by name is what
    // makes the next move survivable.
    let dir = tempfile::tempdir().expect("a temp dir");
    let hub = start(dir.path()).await;

    for missing in ["KYU_TOKEN", "KYU_LISTEN", "KYU_STATE_DIR"] {
        let output = run_backup(&hub, dir.path(), &[(missing, "")]);
        assert!(
            !output.status.success(),
            "without {missing} the script must refuse, not improvise"
        );
        let complaint = String::from_utf8_lossy(&output.stderr);
        assert!(
            complaint.contains(missing),
            "and the complaint must name {missing}: {complaint}"
        );
    }
    assert!(
        backups(dir.path()).is_empty(),
        "a refused run must not leave half a backup behind"
    );
}

#[tokio::test]
async fn f179_a_wrong_token_fails_instead_of_reporting_success() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let hub = start(dir.path()).await;

    let output = run_backup(
        &hub,
        dir.path(),
        &[("KYU_TOKEN", "not-the-right-token-at-all")],
    );
    assert!(
        !output.status.success(),
        "a refused backup must be a failed run — this is what makes OnFailure fire"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("401"),
        "and it must name the status it got: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn f179_pruning_keeps_the_newest_and_is_configurable() {
    // Pruning by count, not by age: a week of failed runs must not be able to
    // delete the last good copy. The count itself is a knob (rule 27).
    let dir = tempfile::tempdir().expect("a temp dir");
    let hub = start(dir.path()).await;

    for run in 0..4 {
        let output = run_backup(&hub, dir.path(), &[("KYU_BACKUP_KEEP", "2")]);
        assert!(
            output.status.success(),
            "run {run} must succeed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        // The filename carries a millisecond stamp; give it a distinct one.
        tokio::time::sleep(Duration::from_millis(1100)).await;
    }

    let kept = backups(dir.path());
    assert_eq!(
        kept.len(),
        2,
        "KYU_BACKUP_KEEP=2 must leave two copies, not four: {kept:?}"
    );
}
