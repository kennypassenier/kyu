//! Damping of repeated notifications: `Kyu-Dedup-Key` / `Kyu-Dedup-Window`
//! on `POST /t/{topic}`, schema 8, the sweeper's pruning and the metrics.
//!
//! The hub runs on a mocked clock and no background sweeper: a test moves
//! time and calls `Engine::sweep` itself, so no test waits on a window.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use kyu::engine::Engine;
use kyu::engine::clock::MockClock;
use kyu::http::{AppState, Limits, router_with_probes};
use kyu::store::Store;
use kyu::store::dedup::MAX_ROWS;
use kyu::sweeper::Heartbeat;
use serde_json::Value;
use tokio::task::JoinHandle;

const HOUR: i64 = 3_600_000;

struct Hub {
    addr: SocketAddr,
    engine: Arc<Engine>,
    clock: Arc<MockClock>,
    store: Arc<Store>,
    _server: JoinHandle<()>,
    _dir: tempfile::TempDir,
}

impl Hub {
    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    async fn publish(&self, topic: &str, headers: &[(&str, &str)]) -> reqwest::Response {
        let mut request = reqwest::Client::new()
            .post(self.url(&format!("/t/{topic}")))
            .body("{\"title\":\"disk almost full\"}");
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        request.send().await.expect("the hub must answer")
    }

    fn count(&self, sql: &str) -> i64 {
        self.store.with_conn(|conn| {
            conn.query_row(sql, [], |row| row.get(0))
                .expect("counting rows")
        })
    }
}

async fn spawn() -> Hub {
    let dir = tempfile::tempdir().expect("a temp dir");
    let store = Arc::new(Store::open(dir.path()).expect("the store must open"));
    let clock = Arc::new(MockClock::new(1_800_000_000_000));
    let engine = Arc::new(Engine::new(store.clone(), clock.clone()));
    let state = AppState::new(
        engine.clone(),
        Limits {
            max_body_bytes: 1024 * 1024,
            default_wait_s: 1,
            max_wait_s: 300,
            recheck_interval: Duration::from_millis(100),
        },
        Heartbeat::starting_at(i64::MAX / 2),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("an ephemeral port");
    let addr = listener.local_addr().expect("a bound address");
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, router_with_probes(state)).await;
    });
    Hub {
        addr,
        engine,
        clock,
        store,
        _server: server,
        _dir: dir,
    }
}

async fn id_of(response: reqwest::Response) -> String {
    let json: Value = response.json().await.expect("a JSON answer");
    json["id"].as_str().expect("an id").to_string()
}

#[tokio::test]
async fn dedup_without_a_key_behaves_exactly_as_before() {
    let hub = spawn().await;
    let first = hub.publish("alerts", &[]).await;
    let second = hub.publish("alerts", &[]).await;
    assert_eq!(first.status(), 201);
    assert_eq!(second.status(), 201);
    assert!(first.headers().get("kyu-duplicate").is_none());
    let body: Value = first.json().await.unwrap();
    assert!(body.get("duplicate").is_none());
    assert_eq!(hub.count("SELECT count(*) FROM messages"), 2);
    assert_eq!(hub.count("SELECT count(*) FROM dedup_keys"), 0);
}

#[tokio::test]
async fn dedup_a_repeat_inside_the_window_returns_the_original_id() {
    let hub = spawn().await;
    let headers = [("kyu-dedup-key", "disk-full"), ("kyu-dedup-window", "20h")];
    let first = hub.publish("alerts", &headers).await;
    assert_eq!(first.status(), 201);
    let original = id_of(first).await;

    hub.clock.advance(19 * HOUR);
    let repeat = hub.publish("alerts", &headers).await;
    assert_eq!(repeat.status(), 200);
    assert_eq!(
        repeat
            .headers()
            .get("kyu-duplicate")
            .and_then(|v| v.to_str().ok()),
        Some("true")
    );
    let body: Value = repeat.json().await.unwrap();
    assert_eq!(body["id"], original.as_str());
    assert_eq!(body["duplicate"], true);
    assert_eq!(hub.count("SELECT count(*) FROM messages"), 1);
}

#[tokio::test]
async fn dedup_a_repeat_after_the_window_passes_and_rearms_the_key() {
    let hub = spawn().await;
    let headers = [("kyu-dedup-key", "disk-full"), ("kyu-dedup-window", "30m")];
    let first = id_of(hub.publish("alerts", &headers).await).await;

    hub.clock.advance(31 * 60_000);
    let again = hub.publish("alerts", &headers).await;
    assert_eq!(again.status(), 201);
    let second = id_of(again).await;
    assert_ne!(first, second);

    let repeat = hub.publish("alerts", &headers).await;
    assert_eq!(repeat.status(), 200);
    assert_eq!(id_of(repeat).await, second);
}

#[tokio::test]
async fn dedup_a_different_key_or_topic_passes() {
    let hub = spawn().await;
    let a = [("kyu-dedup-key", "a")];
    let b = [("kyu-dedup-key", "b")];
    assert_eq!(hub.publish("alerts", &a).await.status(), 201);
    assert_eq!(hub.publish("alerts", &b).await.status(), 201);
    assert_eq!(hub.publish("other", &a).await.status(), 201);
    assert_eq!(hub.publish("alerts", &a).await.status(), 200);
    assert_eq!(hub.count("SELECT count(*) FROM messages"), 3);
}

#[tokio::test]
async fn dedup_a_key_alone_defaults_to_24_hours() {
    let hub = spawn().await;
    let headers = [("kyu-dedup-key", "nightly")];
    hub.publish("alerts", &headers).await;
    hub.clock.advance(23 * HOUR);
    assert_eq!(hub.publish("alerts", &headers).await.status(), 200);
    hub.clock.advance(2 * HOUR);
    assert_eq!(hub.publish("alerts", &headers).await.status(), 201);
}

#[tokio::test]
async fn dedup_windows_accept_milliseconds_and_units() {
    let hub = spawn().await;
    for (key, window) in [("m", "90000"), ("s", "45s"), ("d", "2d")] {
        let response = hub
            .publish(
                "alerts",
                &[("kyu-dedup-key", key), ("kyu-dedup-window", window)],
            )
            .await;
        assert_eq!(response.status(), 201, "window {window}");
    }
}

#[tokio::test]
async fn dedup_bad_headers_are_refused_with_a_remedy() {
    let hub = spawn().await;
    let cases: [&[(&str, &str)]; 5] = [
        &[("kyu-dedup-key", "k"), ("kyu-dedup-window", "8d")],
        &[("kyu-dedup-key", "k"), ("kyu-dedup-window", "soon")],
        &[("kyu-dedup-key", "k"), ("kyu-dedup-window", "0")],
        &[("kyu-dedup-window", "1h")],
        &[("kyu-dedup-key", "")],
    ];
    for headers in cases {
        let response = hub.publish("alerts", headers).await;
        assert_eq!(response.status(), 400, "{headers:?}");
        let body: Value = response.json().await.unwrap();
        assert!(body["remedy"].as_str().is_some_and(|r| !r.is_empty()));
    }
    let too_long = "k".repeat(201);
    let response = hub.publish("alerts", &[("kyu-dedup-key", &too_long)]).await;
    assert_eq!(response.status(), 400);
    assert_eq!(hub.count("SELECT count(*) FROM messages"), 0);
}

#[tokio::test]
async fn dedup_a_suppressed_duplicate_delivers_nothing_and_is_counted() {
    let hub = spawn().await;
    // A poll needs an existing topic (G7: a typo must not create one), so a
    // plain message makes the topic first; the subscription sees only what
    // follows it.
    hub.publish("alerts", &[]).await;
    let sub = reqwest::get(hub.url("/t/alerts/next?as=ha&wait=0"))
        .await
        .unwrap();
    assert_eq!(sub.status(), 204);
    let headers = [("kyu-dedup-key", "disk-full")];
    hub.publish("alerts", &headers).await;
    hub.publish("alerts", &headers).await;
    hub.publish("alerts", &headers).await;

    assert_eq!(
        hub.count("SELECT count(*) FROM deliveries WHERE state = 'pending'"),
        1
    );
    assert_eq!(hub.count("SELECT sum(deduplicated) FROM stats_hourly"), 2);
    assert_eq!(hub.count("SELECT sum(published) FROM stats_hourly"), 2);
}

#[tokio::test]
async fn dedup_the_sweeper_prunes_expired_keys_and_orphans() {
    let hub = spawn().await;
    hub.publish(
        "alerts",
        &[("kyu-dedup-key", "short"), ("kyu-dedup-window", "1h")],
    )
    .await;
    hub.publish(
        "alerts",
        &[("kyu-dedup-key", "long"), ("kyu-dedup-window", "3d")],
    )
    .await;
    hub.publish("gone", &[("kyu-dedup-key", "x")]).await;

    // A deleted topic takes its keys with it (cascade) ...
    hub.store.with_conn(|conn| {
        conn.execute("DELETE FROM topics WHERE name = 'gone'", [])
            .expect("delete the topic");
    });
    assert_eq!(hub.count("SELECT count(*) FROM dedup_keys"), 2);

    // ... and an expired key goes at the next sweep, a live one stays.
    hub.clock.advance(2 * HOUR);
    hub.engine.sweep(500).expect("a sweep");
    assert_eq!(hub.count("SELECT count(*) FROM dedup_keys"), 1);
    assert_eq!(
        hub.count("SELECT count(*) FROM dedup_keys WHERE key = 'long'"),
        1
    );
}

#[tokio::test]
async fn dedup_the_key_table_is_capped_by_evicting_the_oldest() {
    let hub = spawn().await;
    // Fill the table to its cap directly, then publish one more key.
    hub.store.with_conn(|conn| {
        conn.execute(
            "INSERT INTO topics (name, created_at) VALUES ('bulk', 0)",
            [],
        )
        .expect("a topic");
        let topic_id: i64 = conn
            .query_row("SELECT id FROM topics WHERE name = 'bulk'", [], |r| {
                r.get(0)
            })
            .expect("its id");
        let mut insert = conn
            .prepare(
                "INSERT INTO dedup_keys (topic_id, key, message_id, published_at, expires_at)
                 VALUES (?1, ?2, 'm', ?3, ?4)",
            )
            .expect("prepare");
        for n in 0..MAX_ROWS {
            insert
                .execute((topic_id, format!("k{n}"), n, i64::MAX / 2))
                .expect("insert");
        }
    });
    assert_eq!(hub.count("SELECT count(*) FROM dedup_keys"), MAX_ROWS);

    assert_eq!(
        hub.publish("alerts", &[("kyu-dedup-key", "newest")])
            .await
            .status(),
        201
    );
    assert_eq!(hub.count("SELECT count(*) FROM dedup_keys"), MAX_ROWS);
    assert_eq!(
        hub.count("SELECT count(*) FROM dedup_keys WHERE key = 'k0'"),
        0,
        "the oldest key was evicted"
    );
    assert_eq!(
        hub.count("SELECT count(*) FROM dedup_keys WHERE key = 'newest'"),
        1
    );
}

#[tokio::test]
async fn dedup_the_store_is_at_schema_8_with_its_table() {
    let hub = spawn().await;
    assert_eq!(hub.count("PRAGMA user_version"), 8);
    assert_eq!(
        hub.count("SELECT count(*) FROM pragma_table_info('dedup_keys')"),
        5
    );
}

#[tokio::test]
async fn dedup_metrics_report_keys_and_suppressed_duplicates() {
    let hub = spawn().await;
    let headers = [("kyu-dedup-key", "disk-full")];
    hub.publish("alerts", &headers).await;
    hub.publish("alerts", &headers).await;
    let scraped = hub.store.read(|conn| {
        Ok((
            kyu::store::dedup::live_keys(conn)?,
            kyu::store::dedup::suppressed_by_topic(conn)?,
        ))
    });
    let (keys, suppressed) = scraped.expect("the scrape queries");
    assert_eq!(keys, 1);
    assert_eq!(suppressed, vec![("alerts".to_string(), 1)]);
}
