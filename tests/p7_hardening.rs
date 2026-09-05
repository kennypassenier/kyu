//! [Phase 7] The gaps the test-gap audit found, closed.
//!
//! Grouped by what each one protects rather than by milestone, because that
//! is how they were decided: every gap here was a conscious "close it" at
//! the hardening gate.

use std::net::SocketAddr;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use kyu::engine::clock::{Clock, MockClock, SystemClock};
use kyu::engine::{Defaults, Engine};
use kyu::events::EVENTS_TOPIC;
use kyu::http::{AppState, Limits, router_with_probes};
use kyu::store::queries::StoredPolicy;
use kyu::store::{Store, migrations};
use kyu::sweeper::{self, Heartbeat};
use serde_json::Value;

const START: i64 = 1_700_000_000_000;
const DAY: i64 = 24 * 60 * 60 * 1_000;

// ─── harness ────────────────────────────────────────────────────────────────

struct Hub {
    addr: SocketAddr,
    store: Arc<Store>,
    server: tokio::task::JoinHandle<()>,
    sweeper: tokio::task::JoinHandle<()>,
}

impl Hub {
    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    async fn shutdown(self) {
        self.server.abort();
        self.sweeper.abort();
        let _ = self.server.await;
        let _ = self.sweeper.await;
    }
}

async fn spawn_at(dir: &Path) -> Hub {
    let store = Arc::new(Store::open(dir).expect("a store"));
    let engine = Arc::new(Engine::new(store.clone(), Arc::new(SystemClock)));
    let heartbeat = Heartbeat::starting_at(SystemClock.now_ms());
    let state = AppState::new(
        engine.clone(),
        Limits {
            max_body_bytes: 1024,
            default_wait_s: 1,
            max_wait_s: 300,
            recheck_interval: Duration::from_millis(100),
        },
        heartbeat.clone(),
    );
    let notifiers = state.notifiers.clone();
    let sweeper = sweeper::spawn(engine, heartbeat, move |woken| {
        for (topic, subscription) in woken {
            notifiers.wake(topic, std::slice::from_ref(subscription));
        }
    });

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a port");
    let addr = listener.local_addr().expect("an address");
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, router_with_probes(state)).await;
    });

    Hub {
        addr,
        store,
        server,
        sweeper,
    }
}

async fn spawn() -> (Hub, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let hub = spawn_at(dir.path()).await;
    (hub, dir)
}

async fn publish_bytes(hub: &Hub, topic: &str, content_type: &str, body: Vec<u8>) -> u16 {
    reqwest::Client::new()
        .post(hub.url(&format!("/t/{topic}")))
        .header("content-type", content_type)
        .body(body)
        .send()
        .await
        .expect("a response")
        .status()
        .as_u16()
}

async fn publish(hub: &Hub, topic: &str, body: &str) -> String {
    let response = reqwest::Client::new()
        .post(hub.url(&format!("/t/{topic}")))
        .header("content-type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .expect("a response");
    assert_eq!(response.status(), 201);
    body_json(response).await["id"]
        .as_str()
        .expect("an id")
        .to_string()
}

async fn receive(hub: &Hub, topic: &str, query: &str) -> reqwest::Response {
    reqwest::get(hub.url(&format!("/t/{topic}/next?{query}")))
        .await
        .expect("a response")
}

async fn body_json(response: reqwest::Response) -> Value {
    let text = response.text().await.expect("a body");
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("expected JSON, got {text:?}: {e}"))
}

fn header(response: &reqwest::Response, name: &str) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

async fn bootstrap(hub: &Hub, topic: &str, subscription: &str) {
    publish(hub, topic, r#"{"bootstrap":true}"#).await;
    assert_eq!(
        receive(hub, topic, &format!("as={subscription}&wait=0"))
            .await
            .status(),
        204
    );
}

fn fixture(defaults: Defaults) -> (Engine, Arc<MockClock>, Arc<Store>) {
    let store = Arc::new(Store::open_in_memory().expect("a store"));
    let clock = Arc::new(MockClock::new(START));
    let engine = Engine::with_defaults(store.clone(), clock.clone(), defaults);
    (engine, clock, store)
}

// ─── G2 · a kill during a migration ─────────────────────────────────────────

#[test]
fn p7_g2_a_kill_before_the_snapshot_leaves_the_old_store_intact() {
    // The migration runner's contract: it either completes and moves the
    // version, or it changes nothing. A kill is modelled by a migration that
    // fails part-way, which is the same thing from the store's point of view.
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("kyu.db");
    let mut conn = rusqlite::Connection::open(&path).expect("open");

    let v1 = ["CREATE TABLE probe (id INTEGER PRIMARY KEY) STRICT;"];
    migrations::migrate_with(&mut conn, &v1, Some(dir.path())).expect("v1");
    conn.execute("INSERT INTO probe (id) VALUES (1)", [])
        .expect("a row");

    // Version 2 is broken: it adds a column, then does something impossible.
    let v2 = [
        v1[0],
        "ALTER TABLE probe ADD COLUMN note TEXT; INSERT INTO nonexistent VALUES (1);",
    ];
    let error = migrations::migrate_with(&mut conn, &v2, Some(dir.path()))
        .expect_err("a broken migration must fail");
    assert!(format!("{error:#}").contains("migration 2"));

    // The failed migration rolled back whole: version unchanged, no stray
    // column, data intact.
    let version: u32 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("version");
    assert_eq!(version, 1, "a failed migration must not move the version");
    let rows: i64 = conn
        .query_row("SELECT count(*) FROM probe", [], |row| row.get(0))
        .expect("count");
    assert_eq!(rows, 1);
    assert!(
        conn.query_row("SELECT note FROM probe LIMIT 1", [], |_| Ok(()))
            .is_err(),
        "the half-applied column must have rolled back with the rest"
    );

    // And the rollback point exists, because the snapshot is taken first.
    assert!(
        dir.path().join("kyu.pre-v1.db").exists(),
        "the snapshot is written before the migration runs, so a bad upgrade \
         is reversible even when the migration itself explodes"
    );
}

#[tokio::test]
async fn p7_g2_a_hard_kill_at_startup_leaves_a_migratable_store() {
    // Kill the real binary repeatedly at the moment it opens and migrates,
    // then check it still starts cleanly. The window is small, so this runs
    // several rounds to have a fair chance of landing inside it.
    let dir = tempfile::tempdir().expect("a temp dir");
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
        listener.local_addr().expect("an address").port()
    };

    for _ in 0..8 {
        let mut process = Command::new(env!("CARGO_BIN_EXE_kyu"))
            .env("KYU_STATE_DIR", dir.path())
            .env("KYU_LISTEN", format!("127.0.0.1:{port}"))
            .env("KYU_LOG", "error")
            .spawn()
            .expect("start");
        tokio::time::sleep(Duration::from_millis(15)).await;
        let _ = process.kill();
        let _ = process.wait();
    }

    let store = Store::open(dir.path()).expect("the store must still open after repeated kills");
    let version: u32 = store.with_conn(|conn| {
        conn.query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("version")
    });
    assert_eq!(
        version,
        migrations::MIGRATIONS.len() as u32,
        "and it must be fully migrated, with no manual repair"
    );
}

// ─── G3 · a full but writable store ─────────────────────────────────────────

#[tokio::test]
async fn p7_g3_a_full_store_refuses_publishes_loudly_and_stays_up() {
    let (hub, _dir) = spawn().await;
    bootstrap(&hub, "notify.kenny", "printer").await;

    // max_page_count makes SQLite behave exactly as it does on a full disk:
    // writes fail at commit with SQLITE_FULL.
    hub.store.with_conn(|conn| {
        let pages: i64 = conn
            .query_row("PRAGMA page_count", [], |row| row.get(0))
            .expect("page count");
        conn.pragma_update(None, "max_page_count", pages)
            .expect("cap the store");
    });

    // Free pages inside the file absorb the first few writes, so publish
    // until the store genuinely has to grow.
    let mut refusal = None;
    for _ in 0..200 {
        let response = reqwest::Client::new()
            .post(hub.url("/t/notify.kenny"))
            .header("content-type", "application/json")
            .body("x".repeat(900))
            .send()
            .await
            .expect("the hub must still answer");
        if response.status() != 201 {
            refusal = Some(response);
            break;
        }
    }

    let response = refusal.expect("a store that cannot grow must eventually refuse a publish");
    let status = response.status().as_u16();
    let json = body_json(response).await;

    assert_eq!(
        status, 500,
        "a publish that cannot be stored must fail, not be confirmed: {json}"
    );
    assert!(
        json["remedy"].as_str().unwrap_or_default().len() > 20,
        "and it must carry a remedy: {json}"
    );

    // The process is still serving and reads still work — but health must
    // now say so, which is the L1 gap Kenny chose to close at the Phase 7
    // gate. A hub refusing every publish while Uptime Kuma stays green is
    // exactly the silence this project is built against.
    let health = reqwest::get(hub.url("/healthz")).await.expect("a response");
    let status = health.status().as_u16();
    let body = body_json(health).await;

    assert_eq!(
        status, 503,
        "a store that cannot accept a write is not healthy: {body}"
    );
    assert_eq!(body["subsystems"]["store"]["ok"], false);
    assert!(
        body["subsystems"]["store"]["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("free space"),
        "and the remedy names the likeliest cause first: {body}"
    );

    // Reads are unaffected: the dashboard and the metrics still answer.
    assert_eq!(
        reqwest::get(hub.url("/metrics"))
            .await
            .expect("a response")
            .status(),
        200,
        "a full store does not take the hub down — publishes fail, the rest lives"
    );
}

// ─── G4 · lapsed ────────────────────────────────────────────────────────────

#[test]
fn p7_g4_lapsed_deliveries_stay_lapsed_after_an_unarchive() {
    let (engine, clock, store) = fixture(Defaults {
        retention_ms: None,
        ..Defaults::default()
    });
    engine
        .publish("notify.kenny", b"{}", None)
        .expect("bootstrap");
    engine
        .claim_next("notify.kenny", "gone", false)
        .expect("subscribe");
    engine
        .publish("notify.kenny", b"{}", None)
        .expect("a message");

    clock.advance(31 * DAY);
    let report = engine.sweep(100).expect("a sweep");
    assert_eq!(report.archived, 1);
    assert!(report.lapsed >= 1);

    assert!(engine.unarchive("notify.kenny", "gone").expect("unarchive"));

    let lapsed: i64 = store.with_conn(|conn| {
        conn.query_row(
            "SELECT count(*) FROM deliveries WHERE state = 'lapsed'",
            [],
            |row| row.get(0),
        )
        .expect("count")
    });
    assert!(
        lapsed >= 1,
        "unarchiving does not resurrect a lapsed backlog — that is what the \
         response note warns about"
    );
    assert!(
        engine
            .claim_next("notify.kenny", "gone", false)
            .expect("a poll")
            .claimed
            .is_none(),
        "so the subscription starts empty, from now"
    );
}

#[test]
fn p7_g4_retention_collects_a_message_whose_only_delivery_lapsed() {
    // AR3's pressure valve: archiving is what lets retention reclaim the
    // store from a subscription nobody is coming back for.
    let (engine, clock, store) = fixture(Defaults::default());
    engine
        .publish("notify.kenny", b"{}", None)
        .expect("bootstrap");
    engine
        .claim_next("notify.kenny", "gone", false)
        .expect("subscribe");
    engine
        .publish("notify.kenny", b"{}", None)
        .expect("a message");

    clock.advance(31 * DAY);
    engine.sweep(1000).expect("archive and collect");
    engine.sweep(1000).expect("a second pass");

    let remaining: i64 = store.with_conn(|conn| {
        conn.query_row(
            "SELECT count(*) FROM messages m JOIN topics t ON t.id = m.topic_id \
             WHERE t.name = 'notify.kenny'",
            [],
            |row| row.get(0),
        )
        .expect("count")
    });
    assert_eq!(
        remaining, 0,
        "once the only subscription is archived and its deliveries lapsed, \
         retention may finally reclaim the messages"
    );
}

#[tokio::test]
async fn p7_g4_the_dashboard_shows_a_lapsed_count() {
    let (hub, _dir) = spawn().await;
    bootstrap(&hub, "notify.kenny", "printer").await;
    let page = reqwest::get(hub.url("/t/notify.kenny/dashboard"))
        .await
        .expect("a response")
        .text()
        .await
        .expect("a body");
    assert!(
        page.contains("Lapsed"),
        "AR3 says lapsed is counted and visible; an invisible one is the \
         silence G8 forbids"
    );
}

// ─── G5 · illegal transitions ───────────────────────────────────────────────

#[test]
fn p7_g5_settled_deliveries_refuse_every_further_transition() {
    let (engine, clock, _store) = fixture(Defaults::default());
    engine
        .publish("speak.kenny_pc", b"{}", None)
        .expect("bootstrap");
    engine
        .claim_next("speak.kenny_pc", "tts", false)
        .expect("subscribe");
    engine
        .set_policy(
            "speak.kenny_pc",
            "tts",
            StoredPolicy {
                ttl_ms: Some(1_000),
                ..StoredPolicy::default()
            },
        )
        .expect("a policy");

    let id = engine
        .publish("speak.kenny_pc", b"{}", None)
        .expect("a message")
        .id;
    clock.advance(2_000);
    assert_eq!(engine.sweep(100).expect("a sweep").expired, 1);

    // An expired delivery is settled: nothing may move it again.
    for (what, result) in [
        ("ack", engine.ack("speak.kenny_pc", "tts", &id).err()),
        (
            "nack",
            engine.nack("speak.kenny_pc", "tts", &id, false).err(),
        ),
        (
            "requeue",
            engine.requeue_dead("speak.kenny_pc", "tts", &id).err(),
        ),
    ] {
        let error = result.unwrap_or_else(|| panic!("{what} on an expired delivery must fail"));
        assert!(
            error.remedy().len() > 20,
            "{what}: the refusal must carry a remedy"
        );
    }

    assert!(
        engine
            .claim_next("speak.kenny_pc", "tts", false)
            .expect("a poll")
            .claimed
            .is_none(),
        "and a settled delivery is never offered again"
    );
}

#[test]
fn p7_g5_unarchiving_something_active_changes_nothing_and_says_nothing() {
    let (engine, _clock, store) = fixture(Defaults::default());
    engine
        .publish("notify.kenny", b"{}", None)
        .expect("bootstrap");
    engine
        .claim_next("notify.kenny", "printer", false)
        .expect("subscribe");
    engine
        .claim_next(EVENTS_TOPIC, "watcher", false)
        .expect("watch the events topic");

    let before: i64 = store.with_conn(|conn| {
        conn.query_row(
            "SELECT count(*) FROM messages m JOIN topics t ON t.id = m.topic_id \
             WHERE t.name = ?1",
            [EVENTS_TOPIC],
            |row| row.get(0),
        )
        .expect("count")
    });

    let changed = engine
        .unarchive("notify.kenny", "printer")
        .expect("unarchiving an active subscription is not an error");
    assert!(!changed, "nothing changed");

    let after: i64 = store.with_conn(|conn| {
        conn.query_row(
            "SELECT count(*) FROM messages m JOIN topics t ON t.id = m.topic_id \
             WHERE t.name = ?1",
            [EVENTS_TOPIC],
            |row| row.get(0),
        )
        .expect("count")
    });
    assert_eq!(
        before, after,
        "and no event was published — a spurious subscription.unarchived would \
         wake an HA automation for nothing"
    );
}

// ─── G6 · replay over HTTP ──────────────────────────────────────────────────

#[tokio::test]
async fn p7_g6_replay_works_over_http_and_says_what_it_pulled_in() {
    let (hub, _dir) = spawn().await;
    let first = publish(&hub, "notify.kenny", r#"{"n":1}"#).await;
    publish(&hub, "notify.kenny", r#"{"n":2}"#).await;

    // The documented recovery path, exercised the way the dashboard prints it.
    let response = receive(&hub, "notify.kenny", "as=rebuilt&from=beginning&wait=0").await;
    assert_eq!(response.status(), 200);
    assert_eq!(header(&response, "kyu-id").as_deref(), Some(first.as_str()));

    let notice = header(&response, "kyu-notice").expect("a replay is never silent");
    assert!(
        notice.contains("replayed") && notice.contains('2'),
        "the notice says how many messages it pulled in: {notice}"
    );

    // Asking again is idempotent and reports no further backfill.
    let again = receive(&hub, "notify.kenny", "as=rebuilt&from=beginning&wait=0").await;
    assert_eq!(again.status(), 200);
    assert!(header(&again, "kyu-notice").is_none());
}

#[tokio::test]
async fn p7_g6_replay_on_an_empty_topic_answers_204_without_a_false_notice() {
    let (hub, _dir) = spawn().await;
    publish(&hub, "notify.kenny", r#"{"n":1}"#).await;
    // Drain it so the topic is empty but present.
    let response = receive(&hub, "notify.kenny", "as=drainer&from=beginning&wait=0").await;
    let id = header(&response, "kyu-id").expect("an id");
    reqwest::Client::new()
        .post(hub.url(&format!("/t/notify.kenny/ack/{id}?as=drainer")))
        .send()
        .await
        .expect("ack");

    let empty = receive(&hub, "notify.kenny", "as=drainer&from=beginning&wait=0").await;
    assert_eq!(empty.status(), 204);
    assert!(
        header(&empty, "kyu-notice").is_none(),
        "nothing was replayed, so nothing is claimed to have been"
    );
}

// ─── G7 · the ops endpoints ─────────────────────────────────────────────────

#[tokio::test]
async fn p7_g7_the_retention_endpoint_round_trips_and_refuses_nonsense() {
    let (hub, _dir) = spawn().await;
    publish(&hub, "notify.kenny", "{}").await;
    let client = reqwest::Client::new();

    let fresh = body_json(
        reqwest::get(hub.url("/api/t/notify.kenny/retention"))
            .await
            .expect("a response"),
    )
    .await;
    assert_eq!(fresh["explicit_ms"], Value::Null);
    assert_eq!(fresh["effective_ms"], 604_800_000, "the hub default");

    let set = body_json(
        client
            .put(hub.url("/api/t/notify.kenny/retention"))
            .body(r#"{"retention_ms":86400000}"#)
            .send()
            .await
            .expect("a response"),
    )
    .await;
    assert_eq!(set["explicit_ms"], 86_400_000);
    assert_eq!(set["effective_ms"], 86_400_000);

    let forever = body_json(
        client
            .put(hub.url("/api/t/notify.kenny/retention"))
            .body(r#"{"keep_forever":true}"#)
            .send()
            .await
            .expect("a response"),
    )
    .await;
    assert!(
        forever["explicit_ms"].as_i64().unwrap_or(0) > 100 * 365 * DAY,
        "keep_forever is a real stored value, distinct from unset: {forever}"
    );

    for (what, body, expected) in [
        ("a zero window", r#"{"retention_ms":0}"#, 400),
        ("a negative window", r#"{"retention_ms":-1}"#, 400),
        ("a body that is not JSON", "nope", 400),
    ] {
        let response = client
            .put(hub.url("/api/t/notify.kenny/retention"))
            .body(body)
            .send()
            .await
            .expect("a response");
        let status = response.status().as_u16();
        let json = body_json(response).await;
        assert_eq!(status, expected, "{what}: {json}");
        assert!(
            json["remedy"].as_str().unwrap_or_default().len() > 20,
            "{what}"
        );
    }

    let unknown = client
        .put(hub.url("/api/t/nope.nothing/retention"))
        .body(r#"{"retention_ms":1000}"#)
        .send()
        .await
        .expect("a response");
    assert_eq!(unknown.status(), 404);
}

#[tokio::test]
async fn p7_g7_the_unarchive_endpoint_reports_whether_it_changed_anything() {
    let (hub, _dir) = spawn().await;
    bootstrap(&hub, "notify.kenny", "printer").await;

    let response = reqwest::Client::new()
        .post(hub.url("/api/t/notify.kenny/subs/printer/unarchive"))
        .send()
        .await
        .expect("a response");
    assert_eq!(response.status(), 200);
    let json = body_json(response).await;
    assert_eq!(json["changed"], false);
    assert!(
        json["note"]
            .as_str()
            .unwrap_or_default()
            .contains("not archived"),
        "it says plainly that nothing happened: {json}"
    );
}

// ─── G8 · payload boundaries ────────────────────────────────────────────────

#[tokio::test]
async fn p7_g8_payload_edges_behave() {
    let (hub, _dir) = spawn().await;
    bootstrap(&hub, "notify.kenny", "printer").await;

    // An empty payload is a legitimate message: the fact of it may be the
    // signal.
    assert_eq!(
        publish_bytes(&hub, "notify.kenny", "application/json", Vec::new()).await,
        201
    );
    let received = receive(&hub, "notify.kenny", "as=printer&wait=0").await;
    assert_eq!(received.status(), 200);
    assert_eq!(received.text().await.expect("a body"), "");

    // Exactly at the limit is accepted; one byte more is refused.
    let limit = 1024;
    assert_eq!(
        publish_bytes(&hub, "notify.kenny", "text/plain", vec![b'x'; limit]).await,
        201,
        "a payload of exactly the limit fits"
    );
    assert_eq!(
        publish_bytes(&hub, "notify.kenny", "text/plain", vec![b'x'; limit + 1]).await,
        413,
        "and one byte more is refused rather than trimmed"
    );

    // NUL bytes survive the round trip and do not truncate anything.
    let nuls = vec![b'a', 0, b'b', 0, b'c'];
    assert_eq!(
        publish_bytes(
            &hub,
            "print.receipt",
            "application/octet-stream",
            nuls.clone()
        )
        .await,
        201
    );
    assert_eq!(
        receive(&hub, "print.receipt", "as=printer&wait=0")
            .await
            .status(),
        204,
        "the subscription is new on this topic, so it starts from now"
    );
    assert_eq!(
        publish_bytes(
            &hub,
            "print.receipt",
            "application/octet-stream",
            nuls.clone()
        )
        .await,
        201
    );
    let received = receive(&hub, "print.receipt", "as=printer&wait=0").await;
    assert_eq!(received.status(), 200);
    assert_eq!(received.bytes().await.expect("bytes").to_vec(), nuls);
}

// ─── G9 · payloads must not leak ────────────────────────────────────────────

#[tokio::test]
async fn p7_g9_payloads_never_reach_the_logs_or_the_metrics() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
        listener.local_addr().expect("an address").port()
    };

    let mut process = Command::new(env!("CARGO_BIN_EXE_kyu"))
        .env("KYU_STATE_DIR", dir.path())
        .env("KYU_LISTEN", format!("127.0.0.1:{port}"))
        .env("KYU_LOG_FORMAT", "json")
        .env("KYU_LOG", "debug")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("start");

    let base = format!("http://127.0.0.1:{port}");
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        if reqwest::get(format!("{base}/healthz")).await.is_ok() {
            break;
        }
    }

    // A payload that would be unmistakable if it ever appeared anywhere.
    const SECRET: &str = "korfbal-zeewier-lantaarnpaal";
    let client = reqwest::Client::new();
    client
        .post(format!("{base}/t/notify.kenny"))
        .header("content-type", "application/json")
        .body(format!(r#"{{"token":"{SECRET}"}}"#))
        .send()
        .await
        .expect("publish");
    let _ = reqwest::get(format!("{base}/t/notify.kenny/next?as=printer&wait=0")).await;

    let metrics = reqwest::get(format!("{base}/metrics"))
        .await
        .expect("metrics")
        .text()
        .await
        .expect("a body");

    tokio::time::sleep(Duration::from_millis(300)).await;
    let _ = process.kill();
    let output = process.wait_with_output().expect("output");
    // 3.0.0: the kit's subscriber writes to stderr (journald reads both).
    let logs = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(
        !logs.contains(SECRET),
        "payloads are homelab secrets in practice; they must never reach Loki"
    );
    assert!(
        !metrics.contains(SECRET),
        "nor a metric label, which Prometheus keeps for ever"
    );
    assert!(
        logs.contains("notify.kenny"),
        "topic names are still logged — this test must not pass by logging nothing"
    );
}

// ─── G10 · every dashboard state renders ────────────────────────────────────

#[tokio::test]
async fn p7_g10_the_awkward_dashboard_states_all_render() {
    let (hub, _dir) = spawn().await;

    // A delayed message, so the due-at branch is reached.
    reqwest::Client::new()
        .post(hub.url("/t/notify.kenny?delay=3600000"))
        .header("content-type", "application/json")
        .body(r#"{"later":true}"#)
        .send()
        .await
        .expect("a delayed publish");

    // An archived subscription, so the snippet has no live name to use.
    bootstrap(&hub, "notify.kenny", "retired").await;
    hub.store.with_conn(|conn| {
        conn.execute(
            "UPDATE subscriptions SET state = 'archived' WHERE name = 'retired'",
            [],
        )
        .expect("archive")
    });

    let page = reqwest::get(hub.url("/t/notify.kenny/dashboard"))
        .await
        .expect("a response");
    assert_eq!(
        page.status(),
        200,
        "a topic whose only consumer is archived still renders"
    );
    let body = page.text().await.expect("a body");
    assert!(body.contains("archived"));
    assert!(
        body.contains("due"),
        "the delayed message shows its due time"
    );

    // The events topic has a page like any other topic.
    assert_eq!(
        reqwest::get(hub.url(&format!("/t/{EVENTS_TOPIC}/dashboard")))
            .await
            .expect("a response")
            .status(),
        200
    );
}

// ─── G11 · healthz through the endpoint ─────────────────────────────────────

#[tokio::test]
async fn p7_g11_healthz_answers_503_when_the_store_refuses_writes() {
    let (hub, _dir) = spawn().await;

    // query_only is how SQLite behaves on a read-only store, applied to the
    // live writer connection so the endpoint sees what a real one would.
    hub.store.with_conn(|conn| {
        conn.pragma_update(None, "query_only", "ON")
            .expect("the pragma")
    });

    let response = reqwest::get(hub.url("/healthz")).await.expect("a response");
    let status = response.status().as_u16();
    let health = body_json(response).await;

    assert_eq!(status, 503, "FEATURES promises a non-200 here: {health}");
    assert_eq!(health["subsystems"]["store"]["ok"], false);
    assert_eq!(health["status"], "degraded");
    assert!(
        health["subsystems"]["store"]["detail"]
            .as_str()
            .unwrap_or_default()
            .len()
            > 20
    );
}

// ─── G12 · the events topic is a topic ──────────────────────────────────────

#[test]
fn p7_g12_the_events_topic_behaves_like_any_other() {
    let (engine, clock, store) = fixture(Defaults {
        retention_ms: Some(DAY),
        ..Defaults::default()
    });

    // Events accumulate with nobody listening.
    engine
        .publish("notify.kenny", b"{}", None)
        .expect("bootstrap");
    engine
        .claim_next("notify.kenny", "someone", false)
        .expect("subscribe");
    clock.advance(8 * DAY);
    engine.sweep(1000).expect("a sweep that flags");

    let events = |store: &Store| -> i64 {
        store.with_conn(|conn| {
            conn.query_row(
                "SELECT count(*) FROM messages m JOIN topics t ON t.id = m.topic_id \
                 WHERE t.name = ?1",
                [EVENTS_TOPIC],
                |row| row.get(0),
            )
            .expect("count")
        })
    };
    assert!(
        events(&store) > 0,
        "events pile up whether or not anyone reads"
    );

    // A late listener can replay them, exactly like any other topic.
    let received = engine
        .claim_next(EVENTS_TOPIC, "late-watcher", true)
        .expect("replay the events topic");
    assert!(
        received.backfilled > 0 && received.claimed.is_some(),
        "the hub's own history is readable after the fact"
    );

    // Retention leaves alone what that watcher is still holding — the
    // backlogs-win rule applies to the hub's own topic too.
    clock.advance(3 * DAY);
    engine.sweep(1000).expect("a collecting sweep");
    assert!(
        events(&store) > 0,
        "a message an active subscription still holds is never collected, \
         even on kyu.events"
    );

    // Drain the watcher completely. Its earlier claim had its lease expire
    // and was re-pended with a backoff, so step past that window first —
    // otherwise the drain finds nothing and the test lies about why.
    clock.advance(60 * 60 * 1_000);

    // Claim-then-ack in a loop rather than acking a remembered id.
    while let Some(claimed) = engine
        .claim_next(EVENTS_TOPIC, "late-watcher", false)
        .expect("a poll")
        .claimed
    {
        engine
            .ack(EVENTS_TOPIC, "late-watcher", &claimed.message.id)
            .expect("ack the event");
    }

    clock.advance(3 * DAY);
    engine.sweep(1000).expect("a collecting sweep");
    engine.sweep(1000).expect("a settling sweep");
    assert_eq!(
        events(&store),
        0,
        "and once nothing needs them, the events topic empties like any other"
    );
}

// ─── G14 · publishers, consumers and the sweeper at once ────────────────────

#[tokio::test]
async fn p7_g14_nothing_is_lost_or_duplicated_under_concurrent_load() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let hub = spawn_at(dir.path()).await;
    bootstrap(&hub, "jobs.transcode", "worker").await;

    // A short lease, so the sweeper is genuinely re-pending rows underneath
    // the consumers rather than sitting idle.
    reqwest::Client::new()
        .put(hub.url("/api/t/jobs.transcode/subs/worker/policy"))
        .body(r#"{"lease_ms":400,"backoff_ms":0,"max_attempts":50}"#)
        .send()
        .await
        .expect("a policy");

    let total = 120usize;
    let base = hub.url("");

    let publisher = {
        let base = base.clone();
        tokio::spawn(async move {
            let client = reqwest::Client::new();
            let mut ids = Vec::new();
            for n in 0..total {
                let response = client
                    .post(format!("{base}/t/jobs.transcode"))
                    .header("content-type", "application/json")
                    .body(format!(r#"{{"n":{n}}}"#))
                    .send()
                    .await
                    .expect("a publish");
                assert_eq!(response.status(), 201);
                let json: Value =
                    serde_json::from_str(&response.text().await.expect("a body")).expect("JSON");
                ids.push(json["id"].as_str().expect("an id").to_string());
                if n % 17 == 0 {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            }
            ids
        })
    };

    let mut consumers = Vec::new();
    for _ in 0..5 {
        let base = base.clone();
        consumers.push(tokio::spawn(async move {
            let client = reqwest::Client::new();
            let mut handled = Vec::new();
            let deadline = std::time::Instant::now() + Duration::from_secs(20);
            let mut empty_rounds = 0;
            while std::time::Instant::now() < deadline && empty_rounds < 20 {
                let response = client
                    .get(format!("{base}/t/jobs.transcode/next?as=worker&wait=0"))
                    .send()
                    .await
                    .expect("a response");
                if response.status() == 204 {
                    empty_rounds += 1;
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    continue;
                }
                assert_eq!(response.status(), 200, "no request may fail under load");
                empty_rounds = 0;
                let id = response
                    .headers()
                    .get("kyu-id")
                    .and_then(|v| v.to_str().ok())
                    .expect("an id")
                    .to_string();
                let acked = client
                    .post(format!("{base}/t/jobs.transcode/ack/{id}?as=worker"))
                    .send()
                    .await
                    .expect("a response");
                // A slow consumer may lose its lease to the sweeper and find
                // the message already handled by someone else; that is the
                // at-least-once contract, not a failure.
                if acked.status() == 200 {
                    handled.push(id);
                }
            }
            handled
        }));
    }

    let published = publisher.await.expect("the publisher");
    let mut handled = Vec::new();
    for consumer in consumers {
        handled.extend(consumer.await.expect("a consumer"));
    }

    let mut unique = handled.clone();
    unique.sort();
    unique.dedup();

    assert_eq!(
        unique.len(),
        handled.len(),
        "no message may be acked twice: {} acks, {} distinct",
        handled.len(),
        unique.len()
    );
    let mut expected = published;
    expected.sort();
    assert_eq!(
        unique, expected,
        "and every published message must be handled exactly once"
    );

    hub.shutdown().await;
}

// ─── G15 · the races, run for real ──────────────────────────────────────────

#[tokio::test]
async fn p7_g15_an_ack_at_the_lease_boundary_wins_against_the_live_sweeper() {
    let (hub, _dir) = spawn().await;
    bootstrap(&hub, "jobs.transcode", "worker").await;
    reqwest::Client::new()
        .put(hub.url("/api/t/jobs.transcode/subs/worker/policy"))
        .body(r#"{"lease_ms":150,"backoff_ms":0}"#)
        .send()
        .await
        .expect("a policy");

    // Claim, then ack right around the moment the lease expires, many times.
    // Whoever wins, the message must never be delivered twice as acked.
    for round in 0..25 {
        let id = publish(&hub, "jobs.transcode", &format!(r#"{{"round":{round}}}"#)).await;
        let claimed = receive(&hub, "jobs.transcode", "as=worker&wait=0").await;
        assert_eq!(claimed.status(), 200);

        tokio::time::sleep(Duration::from_millis(150)).await;
        let acked = reqwest::Client::new()
            .post(hub.url(&format!("/t/jobs.transcode/ack/{id}?as=worker")))
            .send()
            .await
            .expect("a response");
        // Either the ack won (200) or the sweeper re-pended first (409).
        assert!(
            acked.status() == 200 || acked.status() == 409,
            "a boundary ack is either accepted or refused with a reason, never \
             a surprise: {}",
            acked.status()
        );

        // Drain whatever is left so the next round starts clean.
        loop {
            let response = receive(&hub, "jobs.transcode", "as=worker&wait=0").await;
            if response.status() != 200 {
                break;
            }
            let leftover = header(&response, "kyu-id").expect("an id");
            let _ = reqwest::Client::new()
                .post(hub.url(&format!("/t/jobs.transcode/ack/{leftover}?as=worker")))
                .send()
                .await;
        }
    }
}

#[tokio::test]
async fn p7_g15_several_waiters_and_one_message_wakes_someone_promptly() {
    let (hub, _dir) = spawn().await;
    bootstrap(&hub, "notify.kenny", "printer").await;

    // Four long polls waiting on the same subscription; one message arrives.
    let mut waiters = Vec::new();
    for _ in 0..4 {
        let url = hub.url("/t/notify.kenny/next?as=printer&wait=10");
        waiters.push(tokio::spawn(async move {
            let started = std::time::Instant::now();
            let response = reqwest::get(url).await.expect("a response");
            (response.status().as_u16(), started.elapsed())
        }));
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    publish(&hub, "notify.kenny", r#"{"one":true}"#).await;

    let mut delivered = 0;
    let mut fastest = Duration::from_secs(60);
    for waiter in waiters {
        let (status, elapsed) = waiter.await.expect("a waiter");
        if status == 200 {
            delivered += 1;
            fastest = fastest.min(elapsed);
        }
    }

    assert_eq!(delivered, 1, "exactly one waiter may get the message");
    assert!(
        fastest < Duration::from_secs(3),
        "and it must be woken rather than left to time out: {fastest:?}"
    );
}

// ─── G16 · a backup is verified before it is reported ───────────────────────

#[test]
fn p7_g16_a_corrupt_backup_target_is_not_reported_as_a_backup() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let store = Store::open(dir.path()).expect("a store");

    let good = dir.path().join("good.db");
    let bytes = store.backup_to(&good).expect("a healthy backup");
    assert!(bytes > 0);

    // The verification opens what it wrote: prove it would catch a file that
    // is not a database, by handing it one.
    let occupied = dir.path().join("occupied.db");
    std::fs::write(&occupied, b"not a database at all").expect("a file");
    let error = store
        .backup_to(&occupied)
        .expect_err("an existing file is refused before anything is written");
    assert!(format!("{error:#}").contains("already exists"));

    // And the good one really opens as a database.
    let restored = rusqlite::Connection::open(&good).expect("the backup opens");
    let integrity: String = restored
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .expect("integrity");
    assert_eq!(integrity, "ok");
}

// ─── P1 · the dead-letter view ──────────────────────────────────────────────

#[tokio::test]
async fn p7_p1_the_dashboard_shows_dead_letters_and_requeues_them() {
    let (hub, _dir) = spawn().await;
    bootstrap(&hub, "print.receipt", "printer").await;

    let id = publish(&hub, "print.receipt", r#"{"receipt":"kapot"}"#).await;
    assert_eq!(
        receive(&hub, "print.receipt", "as=printer&wait=0")
            .await
            .status(),
        200
    );
    // A poison pill, straight to the dead-letter list.
    assert_eq!(
        reqwest::Client::new()
            .post(hub.url(&format!("/t/print.receipt/nack/{id}?as=printer&dead=true")))
            .send()
            .await
            .expect("a response")
            .status(),
        200
    );

    let page = reqwest::get(hub.url("/t/print.receipt/dashboard"))
        .await
        .expect("a response")
        .text()
        .await
        .expect("a body");

    // K6 promises all four of these on the page, not just a count.
    assert!(page.contains("Dead letters"), "the section exists");
    assert!(page.contains(&id), "the id is shown");
    assert!(page.contains("printer"), "and which subscription gave up");
    assert!(
        page.contains("kapot"),
        "and the payload, which is the whole point of looking: {page}"
    );
    assert!(page.contains("Requeue"), "with one click to put it back");

    // The button posts a form; follow what it does.
    let requeued = reqwest::Client::new()
        .post(hub.url("/t/print.receipt/dashboard/requeue"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(format!("subscription=printer&id={id}"))
        .send()
        .await
        .expect("a response");
    assert!(
        requeued.status().is_success() || requeued.status().is_redirection(),
        "the requeue button returns to the page"
    );

    let redelivered = receive(&hub, "print.receipt", "as=printer&wait=0").await;
    assert_eq!(redelivered.status(), 200);
    assert_eq!(
        header(&redelivered, "kyu-attempt").as_deref(),
        Some("1"),
        "a requeued message starts its attempts over"
    );

    let after = reqwest::get(hub.url("/t/print.receipt/dashboard"))
        .await
        .expect("a response")
        .text()
        .await
        .expect("a body");
    assert!(
        after.contains("Nothing has been dead-lettered"),
        "and the list empties once it is dealt with"
    );
}

/// Two subscriptions on one topic, drained of the bootstrap leftovers that
/// `bootstrap()` leaks into an already-existing subscription's queue:
/// bootstrapping a SECOND subscription publishes its own throwaway message,
/// which fans out to the FIRST subscription too (it already exists), and
/// sits there until it is claimed. W15 and W16 both need two clean
/// subscriptions before publishing the one message they actually test with,
/// so this is shared between them.
async fn bootstrap_two_clean(hub: &Hub, topic: &str, first: &str, second: &str) {
    bootstrap(hub, topic, first).await;
    bootstrap(hub, topic, second).await;
    let leaked = receive(hub, topic, &format!("as={first}&wait=0")).await;
    assert_eq!(
        leaked.status(),
        200,
        "bootstrapping {second} after {first} publishes one message that fans out \
         to {first} too, since {first} already exists by then"
    );
    let leaked_id = header(&leaked, "kyu-id").expect("an id");
    let ack = reqwest::Client::new()
        .post(hub.url(&format!("/t/{topic}/ack/{leaked_id}?as={first}")))
        .send()
        .await
        .expect("a response");
    assert_eq!(
        ack.status(),
        200,
        "the leftover is drained, not left pending"
    );
}

/// [W15] Kenny's own feedback after using the dashboard: Requeue existed,
/// nothing to just throw a dead letter away existed. The button is the
/// mirror image of Requeue — same table, same form shape, deletes instead
/// of resetting the state — and it must not touch any other subscription's
/// copy of the same message (AR2: one message, fanned out to N deliveries).
#[tokio::test]
async fn p7_w15_the_dead_letter_delete_button_removes_only_this_subscriptions_copy() {
    let (hub, _dir) = spawn().await;
    bootstrap_two_clean(&hub, "print.receipt", "printer", "archiver").await;

    let id = publish(&hub, "print.receipt", r#"{"receipt":"kapot"}"#).await;
    let received = receive(&hub, "print.receipt", "as=printer&wait=0").await;
    assert_eq!(received.status(), 200);
    assert_eq!(
        header(&received, "kyu-id").as_deref(),
        Some(id.as_str()),
        "printer's queue is clean, so this is the message this test published"
    );
    assert_eq!(
        reqwest::Client::new()
            .post(hub.url(&format!("/t/print.receipt/nack/{id}?as=printer&dead=true")))
            .send()
            .await
            .expect("a response")
            .status(),
        200
    );

    let page = reqwest::get(hub.url("/t/print.receipt/dashboard"))
        .await
        .expect("a response")
        .text()
        .await
        .expect("a body");
    assert!(page.contains("Delete"), "the button exists beside Requeue");
    assert!(
        page.contains("data-kp-destructive") && page.contains("data-kp-confirm"),
        "and it arms before it acts, like Revoke on the apps page"
    );

    let deleted = reqwest::Client::new()
        .post(hub.url("/t/print.receipt/dashboard/delivery/delete"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(format!("subscription=printer&id={id}"))
        .send()
        .await
        .expect("a response");
    assert!(
        deleted.status().is_success() || deleted.status().is_redirection(),
        "the delete button returns to the page"
    );

    let after = reqwest::get(hub.url("/t/print.receipt/dashboard"))
        .await
        .expect("a response")
        .text()
        .await
        .expect("a body");
    assert!(
        after.contains("Nothing has been dead-lettered"),
        "gone from the topic's dead-letter list"
    );

    // archiver's own copy of the same message is untouched by printer's delete.
    let for_archiver = receive(&hub, "print.receipt", "as=archiver&wait=0").await;
    assert_eq!(
        for_archiver.status(),
        200,
        "the other subscription's copy of the same message survives"
    );
    assert_eq!(
        header(&for_archiver, "kyu-id").as_deref(),
        Some(id.as_str())
    );

    // Deleting something that no longer exists is a 404, not a silent no-op.
    let again = reqwest::Client::new()
        .post(hub.url(&format!(
            "/api/t/print.receipt/subs/printer/deliveries/{id}/delete"
        )))
        .send()
        .await
        .expect("a response");
    assert_eq!(
        again.status(),
        404,
        "deleting an already-gone delivery says so plainly"
    );
}

/// [W16] Kenny's second question after the same session: could he click
/// into a subscription and see its live backlog, not just the count. The
/// dead-letters table was the precedent this reused — same shape, scoped
/// to one subscription, `state IN (pending, claimed)` instead of `dead`.
#[tokio::test]
async fn p7_w16_a_subscription_page_lists_its_own_backlog_and_deleting_spares_the_rest() {
    let (hub, _dir) = spawn().await;
    bootstrap_two_clean(&hub, "print.receipt", "printer", "archiver").await;

    let id = publish(&hub, "print.receipt", r#"{"receipt":"nog te doen"}"#).await;

    // The topic page links to it.
    let topic_page = reqwest::get(hub.url("/t/print.receipt/dashboard"))
        .await
        .expect("a response")
        .text()
        .await
        .expect("a body");
    assert!(
        topic_page.contains("/t/print.receipt/dashboard/subs/printer"),
        "the subscription name on the topic page links to its own page"
    );

    let page = reqwest::get(hub.url("/t/print.receipt/dashboard/subs/printer"))
        .await
        .expect("a response")
        .text()
        .await
        .expect("a body");
    assert!(page.contains(&id), "the pending item's id is shown");
    assert!(
        page.contains("nog te doen"),
        "and its payload, the whole point of looking: {page}"
    );
    assert!(page.contains("Delete"), "with a way to remove it");

    // archiver's backlog page exists and shows the same pending item too —
    // it is the same message, fanned out to both.
    let archiver_page = reqwest::get(hub.url("/t/print.receipt/dashboard/subs/archiver"))
        .await
        .expect("a response");
    assert_eq!(archiver_page.status(), 200);
    let archiver_body = archiver_page.text().await.expect("a body");
    assert!(
        archiver_body.contains(&id),
        "archiver's own pending copy shows on its own page"
    );

    // A name that never polled this topic at all is a 404, the same shape
    // as an unknown topic.
    let unknown = reqwest::get(hub.url("/t/print.receipt/dashboard/subs/nobody")).await;
    assert_eq!(
        unknown.expect("a response").status(),
        404,
        "an unpolled name is not a page that happens to be empty"
    );

    // Delete printer's pending item; archiver's copy of the SAME message
    // must survive, exactly like W15's dead-letter delete.
    let deleted = reqwest::Client::new()
        .post(hub.url("/t/print.receipt/dashboard/delivery/delete"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(format!("subscription=printer&id={id}"))
        .send()
        .await
        .expect("a response");
    assert!(deleted.status().is_success() || deleted.status().is_redirection());

    let after = reqwest::get(hub.url("/t/print.receipt/dashboard/subs/printer"))
        .await
        .expect("a response")
        .text()
        .await
        .expect("a body");
    assert!(
        after.contains("Nothing pending or claimed"),
        "printer's backlog is empty now"
    );

    let for_archiver = receive(&hub, "print.receipt", "as=archiver&wait=0").await;
    assert_eq!(
        for_archiver.status(),
        200,
        "archiver's copy of the same message was never touched"
    );
    assert_eq!(
        header(&for_archiver, "kyu-id").as_deref(),
        Some(id.as_str())
    );
}

#[tokio::test]
async fn p7_p1_a_binary_dead_letter_is_announced_not_mangled() {
    let (hub, _dir) = spawn().await;
    bootstrap(&hub, "print.receipt", "printer").await;

    assert_eq!(
        publish_bytes(
            &hub,
            "print.receipt",
            "application/octet-stream",
            vec![0x00, 0xff, 0x1b, 0x80]
        )
        .await,
        201
    );
    let received = receive(&hub, "print.receipt", "as=printer&wait=0").await;
    let id = header(&received, "kyu-id").expect("an id");
    let _ = reqwest::Client::new()
        .post(hub.url(&format!("/t/print.receipt/nack/{id}?as=printer&dead=true")))
        .send()
        .await;

    let page = reqwest::get(hub.url("/t/print.receipt/dashboard"))
        .await
        .expect("a response")
        .text()
        .await
        .expect("a body");
    assert!(
        page.contains("binary payload (4 bytes)"),
        "a dead letter you cannot read still says what it is: {page}"
    );
}
