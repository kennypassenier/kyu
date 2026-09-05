//! [L5] Crash safety (K12, S4) against a real process.
//!
//! These tests start the actual binary, put traffic through it, kill it with
//! SIGKILL — no shutdown hook, no flush, no chance to tidy up — and then
//! check that the promises held. Nothing here is simulated: real sockets,
//! real SQLite files, real signals (standing rule 9).
//!
//! Two outage shapes, because they leave different traces (standing rule
//! 15): a short one where leases outlive the downtime, and a long one where
//! they do not and the sweeper has to notice on restart.

use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command};
use std::time::Duration;

use serde_json::Value;

struct Hub {
    process: Child,
    port: u16,
}

impl Hub {
    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    /// SIGKILL: the process gets no warning and no chance to flush. This is
    /// the power-cut model, not a graceful stop.
    fn kill(&mut self) {
        self.process.kill().expect("the hub must be killable");
        self.process.wait().expect("the hub must be reapable");
    }
}

impl Drop for Hub {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

/// A port no other test in this binary will pick.
///
/// Every other suite hands its bound listener straight to the server, so it
/// never lets go of the port. This one cannot: it spawns the real binary,
/// which must bind the port itself. Asking the kernel for port 0 and then
/// releasing it leaves a window, and on a loaded machine two tests running
/// concurrently were handed the SAME port — one then talked to the other's
/// hub. It surfaced as "exactly the unacked half comes back: left 50, right
/// 10" in a test that publishes 21 messages; the 50 belonged to the
/// 70-message test next door.
///
/// The counter is what actually fixes it: two calls in this binary cannot
/// return the same number, whatever the timing. The bind check only skips
/// ports something outside this process already holds.
fn free_port() -> u16 {
    use std::sync::atomic::{AtomicU16, Ordering};
    // Above the ephemeral range Linux hands out by default, so the kernel is
    // not competing with us for these numbers.
    static NEXT: AtomicU16 = AtomicU16::new(61_000);

    for _ in 0..500 {
        let candidate = NEXT.fetch_add(1, Ordering::Relaxed);
        if TcpListener::bind(("127.0.0.1", candidate)).is_ok() {
            return candidate;
        }
    }
    panic!("no free port in the test range — is something holding 61000+?");
}

async fn start(data_dir: &Path, port: u16) -> Hub {
    let process = Command::new(env!("CARGO_BIN_EXE_kyu"))
        .env("KYU_STATE_DIR", data_dir)
        .env("KYU_LISTEN", format!("127.0.0.1:{port}"))
        .env("KYU_LOG", "warn")
        .spawn()
        .expect("the kyu binary must start");

    let hub = Hub { process, port };

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

async fn publish(hub: &Hub, topic: &str, body: String) -> Option<String> {
    let response = reqwest::Client::new()
        .post(hub.url(&format!("/t/{topic}")))
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .ok()?;
    if response.status() != 201 {
        return None;
    }
    let json: Value = serde_json::from_str(&response.text().await.ok()?).ok()?;
    Some(json["id"].as_str()?.to_string())
}

async fn receive(hub: &Hub, topic: &str, subscription: &str) -> reqwest::Response {
    reqwest::get(hub.url(&format!("/t/{topic}/next?as={subscription}&wait=0")))
        .await
        .expect("the hub must answer")
}

async fn ack(hub: &Hub, topic: &str, id: &str, subscription: &str) -> reqwest::Response {
    reqwest::Client::new()
        .post(hub.url(&format!("/t/{topic}/ack/{id}?as={subscription}")))
        .send()
        .await
        .expect("the hub must answer")
}

fn header(response: &reqwest::Response, name: &str) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

async fn bootstrap(hub: &Hub, topic: &str, subscription: &str) {
    publish(hub, topic, r#"{"bootstrap":true}"#.to_string())
        .await
        .expect("the bootstrap publish");
    assert_eq!(receive(hub, topic, subscription).await.status(), 204);
}

/// Drains everything currently deliverable, acking as it goes.
async fn drain(hub: &Hub, topic: &str, subscription: &str) -> Vec<String> {
    let mut ids = Vec::new();
    loop {
        let response = receive(hub, topic, subscription).await;
        if response.status() != 200 {
            return ids;
        }
        let id = header(&response, "kyu-id").expect("an id");
        assert_eq!(ack(hub, topic, &id, subscription).await.status(), 200);
        ids.push(id);
    }
}

#[tokio::test]
async fn l5_s4_every_confirmed_publish_survives_a_hard_kill() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let port = free_port();
    let mut hub = start(dir.path(), port).await;
    bootstrap(&hub, "notify.kenny", "printer").await;

    // Publish under load and remember only what the hub confirmed. A publish
    // whose connection died with the process proves nothing either way; a
    // 201 is a promise.
    let mut confirmed = Vec::new();
    for n in 0..60 {
        if let Some(id) = publish(&hub, "notify.kenny", format!(r#"{{"n":{n}}}"#)).await {
            confirmed.push(id);
        }
    }
    assert!(confirmed.len() >= 60, "the hub must have been healthy");

    hub.kill();

    let hub = start(dir.path(), port).await;
    let after = drain(&hub, "notify.kenny", "printer").await;

    assert_eq!(
        after, confirmed,
        "every confirmed publish must be present after a hard kill, in order"
    );
}

#[tokio::test]
async fn l5_s4_a_kill_during_traffic_loses_nothing_that_was_confirmed() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let port = free_port();
    let mut hub = start(dir.path(), port).await;
    bootstrap(&hub, "jobs.transcode", "worker").await;

    // Keep publishing right up to the moment of death, so the kill lands
    // mid-transaction rather than during a quiet pause.
    let mut confirmed = Vec::new();
    let deadline = std::time::Instant::now() + Duration::from_millis(700);
    let mut n = 0;
    while std::time::Instant::now() < deadline {
        if let Some(id) = publish(&hub, "jobs.transcode", format!(r#"{{"n":{n}}}"#)).await {
            confirmed.push(id);
        }
        n += 1;
    }
    hub.kill();

    let hub = start(dir.path(), port).await;
    let after = drain(&hub, "jobs.transcode", "worker").await;

    assert_eq!(
        after.len(),
        confirmed.len(),
        "a publish that answered 201 was already on disk (synchronous=FULL)"
    );
    assert_eq!(after, confirmed, "and their order is unchanged");
}

#[tokio::test]
async fn l5_s4_acks_survive_a_hard_kill() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let port = free_port();
    let mut hub = start(dir.path(), port).await;
    bootstrap(&hub, "notify.kenny", "printer").await;

    let mut acked = Vec::new();
    for n in 0..20 {
        publish(&hub, "notify.kenny", format!(r#"{{"n":{n}}}"#))
            .await
            .expect("a publish");
    }
    // Settle half of them, leave the rest waiting.
    for _ in 0..10 {
        let response = receive(&hub, "notify.kenny", "printer").await;
        assert_eq!(response.status(), 200);
        let id = header(&response, "kyu-id").expect("an id");
        assert_eq!(
            ack(&hub, "notify.kenny", &id, "printer").await.status(),
            200
        );
        acked.push(id);
    }

    hub.kill();

    let hub = start(dir.path(), port).await;
    let after = drain(&hub, "notify.kenny", "printer").await;

    assert_eq!(after.len(), 10, "exactly the unacked half comes back");
    for id in &acked {
        assert!(
            !after.contains(id),
            "an acked message must stay acked across a hard kill: {id}"
        );
    }
}

#[tokio::test]
async fn l5_s4_a_short_outage_leaves_claimed_messages_claimed() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let port = free_port();
    let mut hub = start(dir.path(), port).await;
    bootstrap(&hub, "jobs.transcode", "worker").await;

    // A long lease: the outage will be far shorter than it.
    assert_eq!(
        reqwest::Client::new()
            .put(hub.url("/api/t/jobs.transcode/subs/worker/policy"))
            .body(r#"{"lease_ms":600000}"#)
            .send()
            .await
            .expect("a response")
            .status(),
        200
    );

    publish(&hub, "jobs.transcode", r#"{"job":1}"#.to_string())
        .await
        .expect("a publish");
    assert_eq!(
        receive(&hub, "jobs.transcode", "worker").await.status(),
        200
    );

    hub.kill();
    let hub = start(dir.path(), port).await;

    // The consumer that held it may still be alive and about to ack, so the
    // message stays claimed until its lease genuinely runs out.
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    assert_eq!(
        receive(&hub, "jobs.transcode", "worker").await.status(),
        204,
        "a short outage must not shorten a lease that is still valid"
    );
}

#[tokio::test]
async fn l5_s4_a_long_outage_returns_in_flight_messages_to_the_queue() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let port = free_port();
    let mut hub = start(dir.path(), port).await;
    bootstrap(&hub, "jobs.transcode", "worker").await;

    // A short lease, so the downtime genuinely outlasts it — the difference
    // between a reboot and a week-long outage, compressed.
    assert_eq!(
        reqwest::Client::new()
            .put(hub.url("/api/t/jobs.transcode/subs/worker/policy"))
            .body(r#"{"lease_ms":200,"backoff_ms":0}"#)
            .send()
            .await
            .expect("a response")
            .status(),
        200
    );

    let id = publish(&hub, "jobs.transcode", r#"{"job":1}"#.to_string())
        .await
        .expect("a publish");
    let claimed = receive(&hub, "jobs.transcode", "worker").await;
    assert_eq!(claimed.status(), 200);
    assert_eq!(header(&claimed, "kyu-attempt").as_deref(), Some("1"));

    hub.kill();
    // The hub is down for longer than the lease. Nothing can expire it while
    // nothing is running: the sweeper has to catch up after the restart.
    tokio::time::sleep(Duration::from_millis(600)).await;
    let hub = start(dir.path(), port).await;

    let mut returned = None;
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(150)).await;
        let response = receive(&hub, "jobs.transcode", "worker").await;
        if response.status() == 200 {
            returned = Some(response);
            break;
        }
    }

    let response = returned.expect("an in-flight message must return after a long outage");
    assert_eq!(header(&response, "kyu-id").as_deref(), Some(id.as_str()));
    assert_eq!(
        header(&response, "kyu-attempt").as_deref(),
        Some("2"),
        "the interrupted attempt is counted, so a poison pill cannot loop forever"
    );
}

#[tokio::test]
async fn l5_a_hard_kill_never_needs_manual_repair_to_restart() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let port = free_port();

    // Ten kills in a row, each mid-traffic. The rejected brokers in Phase 1
    // were rejected partly for needing hand-repair of their state files
    // after exactly this; kyu has to come back by itself, every time.
    for round in 0..10 {
        let mut hub = start(dir.path(), port).await;
        bootstrap(&hub, "notify.kenny", &format!("round{round}")).await;
        for n in 0..5 {
            publish(
                &hub,
                "notify.kenny",
                format!(r#"{{"round":{round},"n":{n}}}"#),
            )
            .await
            .expect("a publish");
        }
        let _ = receive(&hub, "notify.kenny", &format!("round{round}")).await;
        hub.kill();
    }

    let hub = start(dir.path(), port).await;
    let health: Value = serde_json::from_str(
        &reqwest::get(hub.url("/healthz"))
            .await
            .expect("a response")
            .text()
            .await
            .expect("a body"),
    )
    .expect("JSON");
    assert_eq!(health["status"], "ok");
    assert_eq!(health["subsystems"]["store"]["ok"], true);
    assert!(
        dir.path()
            .join("kyu.db")
            .metadata()
            .expect("the store file must exist")
            .len()
            > 0,
        "the store is intact after ten hard kills"
    );
}

#[tokio::test]
async fn l5_healthz_reports_the_store_and_the_sweeper() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let port = free_port();
    let hub = start(dir.path(), port).await;

    let response = reqwest::get(hub.url("/healthz")).await.expect("a response");
    assert_eq!(response.status(), 200);
    let health: Value =
        serde_json::from_str(&response.text().await.expect("a body")).expect("JSON");

    assert_eq!(health["status"], "ok");
    assert_eq!(health["subsystems"]["store"]["ok"], true);
    assert_eq!(health["subsystems"]["store"]["detail"], "writable");
    assert_eq!(
        health["subsystems"]["sweeper"]["ok"], true,
        "a stopped sweeper is invisible from outside unless health says so"
    );
}

#[tokio::test]
async fn l5_healthz_goes_unhealthy_when_the_sweeper_stops() {
    // Built in-process rather than as a spawned binary, because the point is
    // a hub whose sweeper is *not* running: everything answers, but leases
    // stop expiring and messages quietly stop coming back.
    use std::sync::Arc;

    use kyu::engine::Engine;
    use kyu::engine::clock::{Clock, SystemClock};
    use kyu::http::{AppState, Limits, router_with_probes};
    use kyu::store::Store;
    use kyu::sweeper::{Heartbeat, STALE_AFTER_MS};

    let store = Arc::new(Store::open_in_memory().expect("a store"));
    let engine = Arc::new(Engine::new(store, Arc::new(SystemClock)));
    // A heartbeat from well before the staleness threshold: no sweeper has
    // reported in for far too long.
    let heartbeat = Heartbeat::starting_at(SystemClock.now_ms() - STALE_AFTER_MS * 10);
    let state = AppState::new(
        engine,
        Limits {
            max_body_bytes: 1024,
            default_wait_s: 1,
            max_wait_s: 300,
            recheck_interval: Duration::from_secs(5),
        },
        heartbeat,
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a port");
    let addr = listener.local_addr().expect("an address");
    tokio::spawn(async move {
        let _ = axum::serve(listener, router_with_probes(state)).await;
    });

    let response = reqwest::get(format!("http://{addr}/healthz"))
        .await
        .expect("a response");
    let status = response.status();
    let health: Value =
        serde_json::from_str(&response.text().await.expect("a body")).expect("JSON");

    assert_eq!(
        status, 503,
        "a hub whose sweeper has stopped is not healthy: {health}"
    );
    assert_eq!(health["status"], "degraded");
    assert_eq!(health["subsystems"]["sweeper"]["ok"], false);
    assert_eq!(
        health["subsystems"]["store"]["ok"], true,
        "the store itself is fine"
    );
    assert!(
        health["subsystems"]["sweeper"]["detail"]
            .as_str()
            .unwrap_or_default()
            .len()
            > 20,
        "and it must say what to do about it: {health}"
    );
}

#[tokio::test]
async fn l5_the_healthcheck_flag_answers_for_the_shell_less_image() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let port = free_port();
    let hub = start(dir.path(), port).await;

    // What the container healthcheck runs: the binary probing itself,
    // because distroless has no curl and no shell (T9).
    let healthy = Command::new(env!("CARGO_BIN_EXE_kyu"))
        .arg("--healthcheck")
        .env("KYU_LISTEN", format!("127.0.0.1:{port}"))
        .env("KYU_STATE_DIR", dir.path())
        .status()
        .expect("the healthcheck must run");
    assert!(healthy.success(), "a healthy hub must exit 0");

    drop(hub);
    tokio::time::sleep(Duration::from_millis(300)).await;

    let dead = Command::new(env!("CARGO_BIN_EXE_kyu"))
        .arg("--healthcheck")
        .env("KYU_LISTEN", format!("127.0.0.1:{port}"))
        .env("KYU_STATE_DIR", dir.path())
        .status()
        .expect("the healthcheck must run");
    assert!(
        !dead.success(),
        "and a hub that is gone must exit non-zero, or the container would \
         never be restarted"
    );
}
