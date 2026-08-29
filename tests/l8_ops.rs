//! [L8] Observability, ops and scheduling (W1, W4, W7, W8).
//!
//! The four features that sit beside the delivery core rather than inside
//! it: metrics for Grafana, delayed delivery, JSON logs, and a backup whose
//! restore is tested rather than described.

use std::net::SocketAddr;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use kyu::engine::clock::{Clock, MockClock, SystemClock};
use kyu::engine::{Defaults, Engine};
use kyu::http::{AppState, Limits, router};
use kyu::store::Store;
use kyu::sweeper::Heartbeat;
use serde_json::Value;

struct Hub {
    addr: SocketAddr,
    server: tokio::task::JoinHandle<()>,
}

impl Hub {
    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    /// Actually stops serving. Dropping the struct would not: the task owns
    /// itself, and a "restart" that left the old server running would prove
    /// nothing about persistence.
    async fn shutdown(self) {
        self.server.abort();
        let _ = self.server.await;
    }
}

async fn spawn_at(dir: &Path) -> Hub {
    let store = Arc::new(Store::open(dir).expect("a store"));
    let engine = Arc::new(Engine::new(store, Arc::new(SystemClock)));
    let state = AppState::new(
        engine,
        Limits {
            max_body_bytes: 1024 * 1024,
            default_wait_s: 1,
            max_wait_s: 300,
            recheck_interval: Duration::from_millis(100),
        },
        Heartbeat::starting_at(SystemClock.now_ms()),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a port");
    let addr = listener.local_addr().expect("an address");
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, router(state)).await;
    });
    Hub { addr, server }
}

async fn spawn() -> (Hub, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let hub = spawn_at(dir.path()).await;
    (hub, dir)
}

async fn publish(hub: &Hub, path: &str, body: &str) -> reqwest::Response {
    reqwest::Client::new()
        .post(hub.url(path))
        .header("content-type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .expect("a response")
}

async fn receive(hub: &Hub, topic: &str, subscription: &str) -> reqwest::Response {
    reqwest::get(hub.url(&format!("/t/{topic}/next?as={subscription}&wait=0")))
        .await
        .expect("a response")
}

async fn body_json(response: reqwest::Response) -> Value {
    serde_json::from_str(&response.text().await.expect("a body")).expect("JSON")
}

// ─── W1 · metrics ───────────────────────────────────────────────────────────

#[tokio::test]
async fn l8_metrics_expose_the_series_that_reveal_a_silent_failure() {
    let (hub, _dir) = spawn().await;
    publish(&hub, "/t/notify.kenny", "{}").await;
    assert_eq!(receive(&hub, "notify.kenny", "printer").await.status(), 204);
    publish(&hub, "/t/notify.kenny", r#"{"n":1}"#).await;
    publish(&hub, "/t/notify.kenny", r#"{"n":2}"#).await;

    let response = reqwest::get(hub.url("/metrics")).await.expect("a response");
    assert_eq!(response.status(), 200);
    assert!(
        response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .starts_with("text/plain"),
        "Prometheus reads text, not JSON"
    );

    let body = response.text().await.expect("a body");
    assert!(body.contains("# TYPE kyu_deliveries gauge"));
    assert!(
        body.contains(
            r#"kyu_deliveries{topic="notify.kenny",subscription="printer",state="pending"} 2"#
        ),
        "the backlog is per topic and subscription, which is what an alert \
         needs to name the broken thing: {body}"
    );
    assert!(body.contains("kyu_store_bytes "));
    assert!(
        body.contains("kyu_sweeper_age_ms "),
        "a stalled sweeper must be alertable, not only visible on /healthz"
    );
}

// ─── W4 · delayed delivery ──────────────────────────────────────────────────

#[test]
fn l8_a_delayed_message_is_durable_immediately_and_deliverable_later() {
    let store = Arc::new(Store::open_in_memory().expect("a store"));
    let clock = Arc::new(MockClock::new(1_700_000_000_000));
    let engine = Engine::with_defaults(store, clock.clone(), Defaults::default());

    engine
        .publish("notify.kenny", b"{}", Some("application/json"))
        .expect("the bootstrap");
    engine
        .claim_next("notify.kenny", "printer", false)
        .expect("subscribe");

    let due = clock.now_ms() + 30 * 60 * 1_000;
    let published = engine
        .publish_due("notify.kenny", b"{\"wake\":\"me\"}", None, Some(due))
        .expect("a delayed publish");
    assert_eq!(published.due_at, Some(due));

    assert!(
        engine
            .claim_next("notify.kenny", "printer", false)
            .expect("a poll")
            .claimed
            .is_none(),
        "it is stored, but not yet deliverable"
    );

    clock.set(due);
    assert_eq!(
        engine
            .claim_next("notify.kenny", "printer", false)
            .expect("a poll")
            .claimed
            .map(|claimed| claimed.message.id),
        Some(published.id),
        "and it arrives when it is due"
    );
}

#[tokio::test]
async fn l8_a_schedule_survives_a_restart() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let hub = spawn_at(dir.path()).await;
    publish(&hub, "/t/notify.kenny", "{}").await;
    assert_eq!(receive(&hub, "notify.kenny", "printer").await.status(), 204);

    // Due far enough ahead that it cannot arrive during the test.
    let response = publish(&hub, "/t/notify.kenny?delay=3600000", r#"{"later":true}"#).await;
    assert_eq!(response.status(), 201);
    let scheduled = body_json(response).await;
    assert!(
        scheduled["due_at"].is_i64(),
        "the response says when it will be delivered: {scheduled}"
    );

    hub.shutdown().await;
    let hub = spawn_at(dir.path()).await;

    assert_eq!(
        receive(&hub, "notify.kenny", "printer").await.status(),
        204,
        "the schedule is a column in the store, not a timer in memory, so a \
         restart neither loses it nor releases it early"
    );

    let metrics = reqwest::get(hub.url("/metrics"))
        .await
        .expect("a response")
        .text()
        .await
        .expect("a body");
    assert!(
        metrics.contains("kyu_messages 2"),
        "the message itself is durable from the moment it was accepted: {metrics}"
    );
}

#[tokio::test]
async fn l8_two_answers_to_when_are_refused() {
    let (hub, _dir) = spawn().await;
    let response = publish(&hub, "/t/notify.kenny?delay=1000&at=1700000000000", "{}").await;
    let status = response.status();
    let json = body_json(response).await;

    assert_eq!(status, 400, "got {json}");
    assert!(
        json["remedy"]
            .as_str()
            .unwrap_or_default()
            .contains("not both"),
        "one of them would have to be ignored, and silence is the thing to \
         avoid: {json}"
    );

    let negative = publish(&hub, "/t/notify.kenny?delay=-5", "{}").await;
    assert_eq!(negative.status(), 400);
}

// ─── W8 · backup and restore ────────────────────────────────────────────────

#[tokio::test]
async fn l8_a_backup_taken_under_load_restores_to_a_working_store() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let hub = spawn_at(dir.path()).await;
    publish(&hub, "/t/notify.kenny", "{}").await;
    assert_eq!(receive(&hub, "notify.kenny", "printer").await.status(), 204);

    // Traffic in flight while the backup runs.
    let base = hub.url("");
    let load = tokio::spawn(async move {
        let client = reqwest::Client::new();
        for n in 0..40 {
            let _ = client
                .post(format!("{base}/t/notify.kenny"))
                .header("content-type", "application/json")
                .body(format!(r#"{{"n":{n}}}"#))
                .send()
                .await;
        }
    });

    tokio::time::sleep(Duration::from_millis(30)).await;
    let response = reqwest::Client::new()
        .post(hub.url("/api/backup"))
        .send()
        .await
        .expect("a response");
    assert_eq!(response.status(), 200);
    let backup = body_json(response).await;
    let path = backup["backup"].as_str().expect("a path").to_string();
    assert!(backup["bytes"].as_u64().unwrap_or(0) > 0);
    assert!(
        backup["restore"].as_str().unwrap_or_default().len() > 40,
        "the response documents the restore, so the procedure is where the \
         backup is: {backup}"
    );

    load.await.expect("the load task");
    hub.shutdown().await;

    // Restore into a fresh data directory, exactly as the response says.
    let restored_dir = tempfile::tempdir().expect("a temp dir");
    std::fs::copy(&path, restored_dir.path().join("kyu.db")).expect("the restore copy");

    let restored = spawn_at(restored_dir.path()).await;
    let health = body_json(
        reqwest::get(restored.url("/healthz"))
            .await
            .expect("a response"),
    )
    .await;
    assert_eq!(health["status"], "ok", "the restored store opens and works");

    // The subscription and its backlog came back with it.
    let received = receive(&restored, "notify.kenny", "printer").await;
    assert_eq!(
        received.status(),
        200,
        "a backup that cannot deliver a message afterwards is not a backup"
    );
}

#[tokio::test]
async fn l8_a_backup_never_overwrites_an_existing_file() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let store = Store::open(dir.path()).expect("a store");
    let target = dir.path().join("taken.db");
    std::fs::write(&target, b"something precious").expect("a file");

    let error = store
        .backup_to(&target)
        .expect_err("an existing file must not be replaced");
    assert!(
        format!("{error:#}").contains("already exists"),
        "and it must say why: {error:#}"
    );
    assert_eq!(
        std::fs::read(&target).expect("the file"),
        b"something precious",
        "the original is untouched"
    );
}

// ─── W7 · structured logging ────────────────────────────────────────────────

#[tokio::test]
async fn l8_logs_can_be_emitted_as_json_lines() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
        listener.local_addr().expect("an address").port()
    };

    let output = Command::new(env!("CARGO_BIN_EXE_kyu"))
        .env("KYU_DATA_DIR", dir.path())
        .env("KYU_LISTEN", format!("127.0.0.1:{port}"))
        .env("KYU_LOG_FORMAT", "json")
        .env("KYU_LOG", "info")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("the binary must start");

    tokio::time::sleep(Duration::from_millis(800)).await;
    let mut process = output;
    process.kill().expect("stop the hub");
    let captured = process.wait_with_output().expect("output");
    let logs = String::from_utf8_lossy(&captured.stdout);

    let first = logs
        .lines()
        .find(|line| line.starts_with('{'))
        .expect("json mode must emit JSON lines");
    let parsed: Value = serde_json::from_str(first).expect("each line is one JSON object");
    assert!(
        parsed.get("fields").is_some() || parsed.get("message").is_some(),
        "with the event's fields available for filtering: {parsed}"
    );
    assert!(parsed.get("level").is_some());
}
