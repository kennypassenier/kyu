//! [W12] Stopping on purpose, against the real binary.
//!
//! Until 2.1.0 kyu caught no signals: `systemctl stop kyu` sent SIGTERM and
//! the process disappeared where it stood. The data was never at risk — that
//! is what `l5_crash.rs` proves with ten SIGKILLs — but the files on disk
//! never stood still, so the homelab's file-level backup of CT 109 caught
//! `kyu.db-wal` mid-write and failed with `file changed as we read it`.
//!
//! These tests use real signals against a real process (standing rule 9).
//! SIGTERM is sent with `kill` rather than through a crate, because one
//! command already on every Linux box beats a dependency in the tree.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

struct Hub {
    process: Child,
    port: u16,
    data_dir: PathBuf,
}

impl Hub {
    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    /// The polite stop: exactly what `systemctl stop` sends.
    fn terminate(&self) {
        let status = Command::new("kill")
            .arg("-TERM")
            .arg(self.process.id().to_string())
            .status()
            .expect("kill must run");
        assert!(status.success(), "SIGTERM must be deliverable");
    }

    /// Waits for the process to exit and returns its exit code, failing the
    /// test rather than hanging if the stop never completes.
    fn wait_for_exit(&mut self, within: Duration) -> Option<i32> {
        let deadline = Instant::now() + within;
        loop {
            match self.process.try_wait().expect("waiting must work") {
                Some(status) => return status.code(),
                None if Instant::now() >= deadline => {
                    panic!("the hub did not exit within {within:?} of SIGTERM")
                }
                None => std::thread::sleep(Duration::from_millis(25)),
            }
        }
    }

    fn wal_len(&self) -> u64 {
        std::fs::metadata(self.data_dir.join("kyu.db-wal"))
            .map(|meta| meta.len())
            .unwrap_or(0)
    }
}

impl Drop for Hub {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

/// A port no other test in this binary will pick. Same reasoning as
/// `l5_crash.rs`: this suite spawns the real binary, so it cannot hand a
/// bound listener over and must not race a sibling for a number.
fn free_port() -> u16 {
    use std::sync::atomic::{AtomicU16, Ordering};
    static NEXT: AtomicU16 = AtomicU16::new(62_000);

    for _ in 0..500 {
        let candidate = NEXT.fetch_add(1, Ordering::Relaxed);
        if TcpListener::bind(("127.0.0.1", candidate)).is_ok() {
            return candidate;
        }
    }
    panic!("no free port in the test range — is something holding 62000+?");
}

async fn start(data_dir: &Path, port: u16) -> Hub {
    let process = Command::new(env!("CARGO_BIN_EXE_kyu"))
        .env("KYU_DATA_DIR", data_dir)
        .env("KYU_LISTEN", format!("127.0.0.1:{port}"))
        .env("KYU_LOG", "warn")
        .spawn()
        .expect("the kyu binary must start");

    let hub = Hub {
        process,
        port,
        data_dir: data_dir.to_path_buf(),
    };

    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        if let Ok(response) = reqwest::get(hub.url("/healthz")).await
            && response.status() == 200
        {
            return hub;
        }
    }
    panic!("the hub never became healthy");
}

async fn publish(hub: &Hub, topic: &str, body: &str) {
    let response = reqwest::Client::new()
        .post(hub.url(&format!("/t/{topic}")))
        .body(body.to_string())
        .send()
        .await
        .expect("publish must reach the hub");
    assert_eq!(response.status(), 201, "publish must be accepted");
}

#[tokio::test]
async fn w12_sigterm_exits_cleanly_and_leaves_the_files_standing_still() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut hub = start(dir.path(), free_port()).await;

    // Create a subscription first, then publish, so there is genuinely
    // something in the write-ahead log to fold back.
    let _ = reqwest::get(hub.url("/t/stop.drill/next?as=w12&wait=0")).await;
    for n in 0..20 {
        publish(&hub, "stop.drill", &format!("message {n}")).await;
    }
    assert!(
        hub.wal_len() > 0,
        "the test is meaningless unless the log actually holds something"
    );

    hub.terminate();
    let code = hub.wait_for_exit(Duration::from_secs(15));

    assert_eq!(
        code,
        Some(0),
        "a polite stop must be a clean exit, not 'killed by signal' — \
         systemd reports anything else as a failed unit"
    );
    assert_eq!(
        hub.wal_len(),
        0,
        "after a graceful stop the write-ahead log must be folded back and \
         truncated, so a file-level backup copies files nothing is rewriting"
    );
}

#[tokio::test]
async fn w12_the_backlog_survives_a_graceful_stop() {
    // The checkpoint moves data between files; the point of the whole
    // exercise is that nothing is lost while it does.
    let dir = tempfile::tempdir().expect("a temp dir");
    let port = free_port();
    let mut hub = start(dir.path(), port).await;

    // Bootstrap order matters here (the G7 trap kyu documents): a poll on a
    // topic that does not exist yet is a 404 and creates nothing, and a
    // subscription only receives what is published after it exists.
    publish(&hub, "stop.survive", "before the subscription existed").await;
    let created = reqwest::get(hub.url("/t/stop.survive/next?as=w12&wait=0"))
        .await
        .expect("the first poll creates the subscription");
    assert_eq!(created.status(), 204, "a fresh subscription starts empty");
    publish(&hub, "stop.survive", "still here afterwards").await;

    hub.terminate();
    assert_eq!(hub.wait_for_exit(Duration::from_secs(15)), Some(0));
    drop(hub);

    let hub = start(dir.path(), port).await;
    let response = reqwest::get(hub.url("/t/stop.survive/next?as=w12&wait=2"))
        .await
        .expect("the restarted hub must answer");
    assert_eq!(response.status(), 200, "the message must still be there");
    assert_eq!(
        response.text().await.unwrap(),
        "still here afterwards",
        "and it must be the same message, byte for byte"
    );
}

#[tokio::test]
async fn w12_a_second_sigterm_during_shutdown_changes_nothing() {
    // Requirement 3 of the ecosystem norm: shutting down is idempotent. An
    // impatient operator pressing Ctrl-C twice must not leave the store
    // half-settled.
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut hub = start(dir.path(), free_port()).await;

    let _ = reqwest::get(hub.url("/t/stop.twice/next?as=w12&wait=0")).await;
    publish(&hub, "stop.twice", "one").await;

    hub.terminate();
    hub.terminate();
    hub.terminate();

    assert_eq!(
        hub.wait_for_exit(Duration::from_secs(15)),
        Some(0),
        "extra signals must not turn a clean stop into a kill"
    );
    assert_eq!(hub.wal_len(), 0, "and the store must still be settled");
}

#[tokio::test]
async fn w12_an_in_flight_long_poll_is_answered_rather_than_cut_off() {
    // A consumer waiting on a long poll is the normal state of this hub, so
    // "graceful" has to mean that request gets an answer — not a reset
    // connection the client has to guess about.
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut hub = start(dir.path(), free_port()).await;
    publish(&hub, "stop.inflight", "creates the topic").await;
    let created = reqwest::get(hub.url("/t/stop.inflight/next?as=w12&wait=0"))
        .await
        .expect("the first poll creates the subscription");
    assert_eq!(created.status(), 204, "a fresh subscription starts empty");

    let url = hub.url("/t/stop.inflight/next?as=w12&wait=3");
    let poll = tokio::spawn(async move { reqwest::get(url).await });

    tokio::time::sleep(Duration::from_millis(300)).await;
    hub.terminate();

    let answered = poll.await.expect("the polling task must not panic");
    assert!(
        answered.is_ok(),
        "the in-flight poll must be answered, not dropped: {:?}",
        answered.err()
    );
    assert_eq!(
        answered.unwrap().status(),
        204,
        "and answered properly: nothing was published, so 204"
    );
    assert_eq!(hub.wait_for_exit(Duration::from_secs(15)), Some(0));
}

#[tokio::test]
async fn w12_the_shutdown_budget_is_configurable_and_refuses_nonsense() {
    // Standing rule of this ecosystem: an operational limit is never a bare
    // number in the source. And per standing rule 12 a typo must be refused
    // rather than quietly replaced by the default.
    use kyu::config::{DEFAULT_SHUTDOWN_TIMEOUT_MS, parse_shutdown_timeout};

    assert_eq!(
        parse_shutdown_timeout(None).unwrap(),
        DEFAULT_SHUTDOWN_TIMEOUT_MS
    );
    assert_eq!(parse_shutdown_timeout(Some("250")).unwrap(), 250);
    assert_eq!(parse_shutdown_timeout(Some(" 250 ")).unwrap(), 250);

    let refused = parse_shutdown_timeout(Some("ten seconds")).expect_err("must be refused");
    assert!(
        refused.to_string().contains("KYU_SHUTDOWN_TIMEOUT_MS"),
        "the error names the variable: {refused}"
    );
    let zero = parse_shutdown_timeout(Some("0")).expect_err("zero must be refused");
    assert!(
        zero.to_string().contains("10000"),
        "and carries the remedy (standing rule 11): {zero}"
    );
}
