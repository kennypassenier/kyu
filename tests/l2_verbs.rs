//! [L2] The three verbs over real HTTP against a real store (standing rule
//! 9): a listener on an ephemeral port, a SQLite file in a temp directory,
//! and reqwest as the client. No mocked transport anywhere.
//!
//! One ordering rule shapes almost every test here: a topic exists once
//! something publishes to it, and a subscription exists once it first polls
//! (G7). So a message published before a consumer's first poll is not
//! delivered to that consumer — which is why the helpers below bootstrap a
//! subscription before the message under test is published.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use kyu::engine::Engine;
use kyu::engine::clock::SystemClock;
use kyu::http::{AppState, Limits, router_with_probes};
use kyu::store::Store;
use kyu::sweeper::Heartbeat;
use serde_json::Value;
use tokio::task::JoinHandle;

struct Hub {
    addr: SocketAddr,
    server: JoinHandle<()>,
}

impl Hub {
    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    /// Stops serving, the way a container restart would.
    async fn shutdown(self) {
        self.server.abort();
        let _ = self.server.await;
    }
}

/// A long recheck interval on purpose: it makes the mid-poll wakeup test
/// prove the notification path rather than accidentally passing because a
/// periodic re-check happened to fire.
fn test_limits(max_body_bytes: usize) -> Limits {
    Limits {
        max_body_bytes,
        default_wait_s: 2,
        max_wait_s: 300,
        recheck_interval: Duration::from_secs(600),
    }
}

async fn spawn_at(data_dir: &Path, limits: Limits) -> Hub {
    let store = Arc::new(Store::open(data_dir).expect("the store must open"));
    let engine = Arc::new(Engine::new(store, Arc::new(SystemClock)));
    // Far in the future, so the health endpoint never calls the sweeper
    // stalled in tests that do not run one.
    let heartbeat = Heartbeat::starting_at(i64::MAX / 2);
    let state = AppState::new(engine, limits, heartbeat);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("an ephemeral port");
    let addr = listener.local_addr().expect("a bound address");
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, router_with_probes(state)).await;
    });

    Hub { addr, server }
}

async fn spawn() -> (Hub, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let hub = spawn_at(dir.path(), test_limits(1024 * 1024)).await;
    (hub, dir)
}

async fn publish(hub: &Hub, topic: &str, content_type: &str, body: Vec<u8>) -> reqwest::Response {
    reqwest::Client::new()
        .post(hub.url(&format!("/t/{topic}")))
        .header("content-type", content_type)
        .body(body)
        .send()
        .await
        .expect("the hub must answer")
}

async fn publish_json(hub: &Hub, topic: &str, body: &str) -> String {
    let response = publish(hub, topic, "application/json", body.as_bytes().to_vec()).await;
    assert_eq!(response.status(), 201, "publish must answer 201 Created");
    let json = body_json(response).await;
    json["id"].as_str().expect("an id").to_string()
}

async fn receive(hub: &Hub, topic: &str, query: &str) -> reqwest::Response {
    reqwest::get(hub.url(&format!("/t/{topic}/next?{query}")))
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

async fn body_json(response: reqwest::Response) -> Value {
    let text = response.text().await.expect("a body");
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("expected JSON, got {text:?}: {error}"))
}

fn header(response: &reqwest::Response, name: &str) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

/// Brings a topic and a subscription into existence, the way a real
/// consumer must: publish once so the topic exists, then poll once so the
/// subscription does. Returns nothing — that first poll is empty by design.
async fn bootstrap(hub: &Hub, topic: &str, subscription: &str) {
    publish_json(hub, topic, r#"{"bootstrap":true}"#).await;
    let response = receive(hub, topic, &format!("as={subscription}&wait=0")).await;
    assert_eq!(
        response.status(),
        204,
        "a subscription's first poll cannot see what predates it (G7)"
    );
}

#[tokio::test]
async fn l2_round_trip_in_raw_mode() {
    let (hub, _dir) = spawn().await;
    bootstrap(&hub, "notify.kenny", "printer").await;
    let id = publish_json(&hub, "notify.kenny", r#"{"title":"Backup done"}"#).await;

    let response = receive(&hub, "notify.kenny", "as=printer").await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        header(&response, "kyu-id").as_deref(),
        Some(id.as_str()),
        "the id travels in a header so the body stays the payload"
    );
    assert_eq!(
        header(&response, "kyu-topic").as_deref(),
        Some("notify.kenny")
    );
    assert_eq!(
        header(&response, "kyu-attempt").as_deref(),
        Some("1"),
        "a first delivery is attempt 1"
    );
    assert!(header(&response, "kyu-published-at").is_some());
    assert_eq!(
        header(&response, "content-type").as_deref(),
        Some("application/json")
    );

    let body = response.text().await.expect("a body");
    assert_eq!(
        body, r#"{"title":"Backup done"}"#,
        "the body must be the payload verbatim"
    );

    assert_eq!(
        ack(&hub, "notify.kenny", &id, "printer").await.status(),
        200
    );
    assert_eq!(
        receive(&hub, "notify.kenny", "as=printer&wait=0")
            .await
            .status(),
        204,
        "an acked message must not come back around"
    );
}

#[tokio::test]
async fn l2_round_trip_in_envelope_mode() {
    let (hub, _dir) = spawn().await;
    bootstrap(&hub, "notify.kenny", "ha-forwarder").await;
    let id = publish_json(&hub, "notify.kenny", r#"{"title":"Backup done"}"#).await;

    let response = receive(&hub, "notify.kenny", "as=ha-forwarder&envelope=json").await;
    assert_eq!(response.status(), 200);

    let envelope = body_json(response).await;
    assert_eq!(envelope["id"], id);
    assert_eq!(envelope["topic"], "notify.kenny");
    assert_eq!(envelope["attempt"], 1);
    assert_eq!(envelope["content_type"], "application/json");
    assert_eq!(
        envelope["payload"]["title"], "Backup done",
        "a JSON payload embeds as JSON, not as a quoted string"
    );
    assert!(
        envelope.get("payload_base64").is_none(),
        "JSON must not be base64-encoded"
    );

    assert_eq!(
        ack(&hub, "notify.kenny", &id, "ha-forwarder")
            .await
            .status(),
        200
    );
}

#[tokio::test]
async fn l2_an_acked_message_never_returns_after_a_restart() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let hub = spawn_at(dir.path(), test_limits(1024 * 1024)).await;

    bootstrap(&hub, "jobs.transcode", "worker").await;
    let id = publish_json(&hub, "jobs.transcode", r#"{"file":"a.mkv"}"#).await;
    assert_eq!(
        receive(&hub, "jobs.transcode", "as=worker").await.status(),
        200
    );
    assert_eq!(
        ack(&hub, "jobs.transcode", &id, "worker").await.status(),
        200
    );

    hub.shutdown().await;

    // Same data directory, fresh process state — a container restart.
    let hub = spawn_at(dir.path(), test_limits(1024 * 1024)).await;
    assert_eq!(
        receive(&hub, "jobs.transcode", "as=worker&wait=0")
            .await
            .status(),
        204,
        "an ack that survived a restart must still hold"
    );
}

#[tokio::test]
async fn l2_an_empty_topic_answers_204_after_the_wait_window() {
    let (hub, _dir) = spawn().await;
    bootstrap(&hub, "notify.kenny", "printer").await;

    let started = Instant::now();
    let response = receive(&hub, "notify.kenny", "as=printer&wait=1").await;
    let elapsed = started.elapsed();

    assert_eq!(response.status(), 204);
    assert!(
        elapsed >= Duration::from_millis(900),
        "the poll must actually wait its window rather than return at once: {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "and it must not overstay it: {elapsed:?}"
    );
}

#[tokio::test]
async fn l2_a_message_published_mid_poll_arrives_at_once() {
    let (hub, _dir) = spawn().await;
    bootstrap(&hub, "notify.kenny", "printer").await;

    let url = hub.url("/t/notify.kenny/next?as=printer&wait=20");
    let poll = tokio::spawn(async move { reqwest::get(url).await.expect("a response") });

    tokio::time::sleep(Duration::from_millis(300)).await;
    let started = Instant::now();
    let id = publish_json(&hub, "notify.kenny", r#"{"title":"late"}"#).await;

    let response = poll.await.expect("the poll task");
    let elapsed = started.elapsed();

    assert_eq!(response.status(), 200);
    assert_eq!(header(&response, "kyu-id").as_deref(), Some(id.as_str()));
    assert!(
        elapsed < Duration::from_secs(3),
        "the waiting poll must be woken by the publish, not by its own timeout \
         or a periodic re-check (this hub rechecks every 600s): {elapsed:?}"
    );
}

#[tokio::test]
async fn l2_a_new_subscription_starts_from_now_and_says_so() {
    let (hub, _dir) = spawn().await;
    let earlier = publish_json(&hub, "notify.kenny", r#"{"n":"earlier"}"#).await;

    // The first poll creates the subscription. It finds nothing, and the
    // response explains why rather than leaving an unexplained 204 (G8).
    let response = receive(&hub, "notify.kenny", "as=latecomer&wait=0").await;
    assert_eq!(response.status(), 204);
    let notice = header(&response, "kyu-notice").expect("a notice header");
    assert!(
        notice.contains("latecomer") && notice.contains("from now on"),
        "the notice must name the subscription and its start position: {notice}"
    );
    assert!(
        notice.contains('1') && notice.contains("predate"),
        "and it must say how many messages it could not see: {notice}"
    );

    let later = publish_json(&hub, "notify.kenny", r#"{"n":"later"}"#).await;
    let response = receive(&hub, "notify.kenny", "as=latecomer&wait=0").await;
    assert_eq!(response.status(), 200);
    let got = header(&response, "kyu-id").expect("an id");
    assert_eq!(got, later);
    assert_ne!(got, earlier);
    assert!(
        header(&response, "kyu-notice").is_none(),
        "the notice belongs to the poll that created the subscription, not to later ones"
    );
}

#[tokio::test]
async fn l2_a_binary_payload_survives_the_round_trip() {
    let (hub, _dir) = spawn().await;
    bootstrap(&hub, "print.receipt", "printer").await;

    // Bytes that are not valid UTF-8 and include an ANSI escape.
    let bytes: Vec<u8> = vec![0x00, 0xff, 0x1b, 0x5b, 0x41, 0x80, 0x7f];
    assert_eq!(
        publish(
            &hub,
            "print.receipt",
            "application/octet-stream",
            bytes.clone()
        )
        .await
        .status(),
        201
    );

    let raw = receive(&hub, "print.receipt", "as=printer").await;
    assert_eq!(raw.status(), 200);
    let id = header(&raw, "kyu-id").expect("an id");
    assert_eq!(
        raw.bytes().await.expect("bytes").to_vec(),
        bytes,
        "raw mode must hand back exactly the bytes that were published"
    );
    assert_eq!(
        ack(&hub, "print.receipt", &id, "printer").await.status(),
        200
    );

    // In envelope mode the same bytes are flagged as base64 rather than
    // mangled into replacement characters.
    assert_eq!(
        publish(
            &hub,
            "print.receipt",
            "application/octet-stream",
            bytes.clone()
        )
        .await
        .status(),
        201
    );
    let enveloped = receive(&hub, "print.receipt", "as=printer&envelope=json").await;
    assert_eq!(enveloped.status(), 200);
    let envelope = body_json(enveloped).await;
    assert!(
        envelope["payload_base64"].is_string(),
        "binary must be flagged as base64: {envelope}"
    );
    assert!(envelope.get("payload").is_none());
    assert!(envelope.get("payload_text").is_none());
}

#[tokio::test]
async fn l2_plain_text_is_reported_as_text_in_the_envelope() {
    let (hub, _dir) = spawn().await;
    bootstrap(&hub, "speak.kenny_pc", "tts").await;
    assert_eq!(
        publish(
            &hub,
            "speak.kenny_pc",
            "text/plain",
            b"de was is klaar".to_vec()
        )
        .await
        .status(),
        201
    );

    let envelope = body_json(receive(&hub, "speak.kenny_pc", "as=tts&envelope=json").await).await;
    assert_eq!(envelope["payload_text"], "de was is klaar");
    assert!(envelope.get("payload_base64").is_none());
    assert!(envelope.get("payload").is_none());
}

#[tokio::test]
async fn l2_the_content_type_is_stored_verbatim() {
    let (hub, _dir) = spawn().await;
    bootstrap(&hub, "notify.kenny", "printer").await;

    // What `curl -d '{"a":1}'` actually sends: JSON bytes labelled as a
    // form. kyu hands back the label it was given (AR2) instead of
    // quietly correcting it.
    assert_eq!(
        publish(
            &hub,
            "notify.kenny",
            "application/x-www-form-urlencoded",
            br#"{"a":1}"#.to_vec()
        )
        .await
        .status(),
        201
    );

    let response = receive(&hub, "notify.kenny", "as=printer").await;
    assert_eq!(
        header(&response, "content-type").as_deref(),
        Some("application/x-www-form-urlencoded"),
        "the stored content type comes back as-is, however wrong it was"
    );
}

#[tokio::test]
async fn l2_every_error_carries_a_remedy() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let hub = spawn_at(dir.path(), test_limits(64)).await;
    let client = reqwest::Client::new();

    // A claimed-then-acked message, to build the ack failures on.
    publish_json(&hub, "notify.kenny", r#"{"n":0}"#).await;
    let _ = receive(&hub, "notify.kenny", "as=printer&wait=0").await;
    let id = publish_json(&hub, "notify.kenny", r#"{"n":1}"#).await;
    assert_eq!(
        receive(&hub, "notify.kenny", "as=printer").await.status(),
        200
    );
    assert_eq!(
        ack(&hub, "notify.kenny", &id, "printer").await.status(),
        200
    );

    // Note: `from=beginning` used to belong in this table as a 501. L6
    // implemented replay (K8), so it is a feature now and is tested in
    // l6_history rather than here.
    let cases: Vec<(&str, u16, reqwest::RequestBuilder)> = vec![
        (
            "an invalid topic name",
            400,
            client.post(hub.url("/t/NotifyKenny")).body("{}"),
        ),
        (
            "the reserved prefix",
            403,
            client.post(hub.url("/t/kyu.events")).body("{}"),
        ),
        (
            "an oversized payload",
            413,
            client
                .post(hub.url("/t/notify.kenny"))
                .body("x".repeat(500)),
        ),
        (
            "a topic that does not exist",
            404,
            client.get(hub.url("/t/nope.nothing/next?as=printer&wait=0")),
        ),
        (
            "a missing as parameter",
            400,
            client.get(hub.url("/t/notify.kenny/next?wait=0")),
        ),
        (
            "an invalid subscription name",
            400,
            client.get(hub.url("/t/notify.kenny/next?as=BAD&wait=0")),
        ),
        (
            "a wait beyond the maximum",
            400,
            client.get(hub.url("/t/notify.kenny/next?as=printer&wait=9999")),
        ),
        (
            "an unknown subscription acking",
            404,
            client.post(hub.url(&format!("/t/notify.kenny/ack/{id}?as=ghost"))),
        ),
        (
            "an id that was never delivered",
            404,
            client.post(hub.url("/t/notify.kenny/ack/01ARZ3NDEKTSV4RRFFQ69G5FAV?as=printer")),
        ),
        (
            "acking the same message twice",
            409,
            client.post(hub.url(&format!("/t/notify.kenny/ack/{id}?as=printer"))),
        ),
    ];

    for (what, expected_status, request) in cases {
        let response = request.send().await.expect("the hub must answer");
        let status = response.status().as_u16();
        let body = body_json(response).await;

        assert_eq!(
            status, expected_status,
            "{what}: expected {expected_status}, got {status} with body {body}"
        );
        let error = body["error"].as_str().unwrap_or_default();
        let remedy = body["remedy"].as_str().unwrap_or_default();
        assert!(
            !error.is_empty(),
            "{what}: the error must say what happened"
        );
        assert!(
            !remedy.is_empty(),
            "{what}: every error carries a remedy (standing rule 11); body was {body}"
        );
        assert!(
            remedy.len() > 20,
            "{what}: the remedy must be actionable, not a token: {remedy:?}"
        );
    }
}
