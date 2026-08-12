//! [L4] The reliability endpoints over real HTTP, with the real sweeper
//! running (standing rule 9): policy, nack, dead letters and requeue, plus
//! the S2 crash test against actual wall-clock time.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use mailbox::engine::Engine;
use mailbox::engine::clock::{Clock, SystemClock};
use mailbox::http::{AppState, Limits, router};
use mailbox::store::Store;
use mailbox::sweeper;
use mailbox::sweeper::Heartbeat;
use serde_json::Value;
use tokio::task::JoinHandle;

struct Hub {
    addr: SocketAddr,
    server: JoinHandle<()>,
    sweeper: JoinHandle<()>,
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

/// Mirrors `main.rs`: the same wiring, so these tests exercise the sweeper
/// the way it actually runs rather than a stand-in.
async fn spawn_at(data_dir: &Path) -> Hub {
    let store = Arc::new(Store::open(data_dir).expect("the store must open"));
    let engine = Arc::new(Engine::new(store, Arc::new(SystemClock)));
    let heartbeat = Heartbeat::starting_at(SystemClock.now_ms());
    let state = AppState::new(
        engine.clone(),
        Limits {
            max_body_bytes: 1024 * 1024,
            default_wait_s: 2,
            max_wait_s: 300,
            recheck_interval: Duration::from_millis(200),
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
        .expect("an ephemeral port");
    let addr = listener.local_addr().expect("a bound address");
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, router(state)).await;
    });

    Hub {
        addr,
        server,
        sweeper,
    }
}

async fn spawn() -> (Hub, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let hub = spawn_at(dir.path()).await;
    (hub, dir)
}

async fn publish(hub: &Hub, topic: &str, body: &str) -> String {
    let response = reqwest::Client::new()
        .post(hub.url(&format!("/t/{topic}")))
        .header("content-type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .expect("the hub must answer");
    assert_eq!(response.status(), 201);
    body_json(response).await["id"]
        .as_str()
        .expect("an id")
        .to_string()
}

async fn receive(hub: &Hub, topic: &str, subscription: &str) -> reqwest::Response {
    reqwest::get(hub.url(&format!("/t/{topic}/next?as={subscription}&wait=0")))
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

async fn set_policy(hub: &Hub, topic: &str, subscription: &str, body: &str) -> reqwest::Response {
    reqwest::Client::new()
        .put(hub.url(&format!("/api/t/{topic}/subs/{subscription}/policy")))
        .body(body.to_string())
        .send()
        .await
        .expect("the hub must answer")
}

async fn bootstrap(hub: &Hub, topic: &str, subscription: &str) {
    publish(hub, topic, r#"{"bootstrap":true}"#).await;
    assert_eq!(receive(hub, topic, subscription).await.status(), 204);
}

#[tokio::test]
async fn l4_s2_a_killed_consumer_gets_its_message_redelivered() {
    let (hub, _dir) = spawn().await;
    bootstrap(&hub, "jobs.transcode", "worker").await;

    // A short lease, so a real sweeper on real time can be watched doing its
    // job inside a test.
    assert_eq!(
        set_policy(
            &hub,
            "jobs.transcode",
            "worker",
            r#"{"lease_ms":300,"backoff_ms":0}"#
        )
        .await
        .status(),
        200
    );

    let id = publish(&hub, "jobs.transcode", r#"{"file":"a.mkv"}"#).await;

    // A consumer that claims the message and is then killed before acking.
    let url = hub.url("/t/jobs.transcode/next?as=worker&wait=0");
    let consumer = tokio::spawn(async move {
        let response = reqwest::get(url).await.expect("a response");
        assert_eq!(response.status(), 200);
        // ... and here it dies, without acking.
        std::future::pending::<()>().await;
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    consumer.abort();

    // The sweeper ticks once a second; give it room to notice.
    let mut redelivered = None;
    for _ in 0..30 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let response = receive(&hub, "jobs.transcode", "worker").await;
        if response.status() == 200 {
            redelivered = Some(response);
            break;
        }
    }

    let response = redelivered.expect("a killed consumer's message must come back");
    assert_eq!(
        header(&response, "mailbox-id").as_deref(),
        Some(id.as_str())
    );
    assert_eq!(
        header(&response, "mailbox-attempt").as_deref(),
        Some("2"),
        "and it arrives marked as the second attempt, so a consumer can tell"
    );
}

#[tokio::test]
async fn l4_the_policy_endpoint_reports_what_is_in_force_and_what_is_explicit() {
    let (hub, _dir) = spawn().await;
    bootstrap(&hub, "notify.kenny", "tts").await;

    let fresh = body_json(
        reqwest::get(hub.url("/api/t/notify.kenny/subs/tts/policy"))
            .await
            .expect("a response"),
    )
    .await;
    assert_eq!(fresh["effective"]["lease_ms"], 30_000);
    assert_eq!(fresh["effective"]["max_attempts"], 5);
    assert_eq!(
        fresh["explicit"]["lease_ms"],
        Value::Null,
        "nothing is stored until it is set, so the defaults can still move"
    );
    assert_eq!(
        fresh["retry_schedule_ms"],
        serde_json::json!([1000, 2000, 3000, 4000]),
        "the schedule is spelled out rather than left to be inferred"
    );

    let updated =
        body_json(set_policy(&hub, "notify.kenny", "tts", r#"{"ttl_ms":600000}"#).await).await;
    assert_eq!(updated["effective"]["ttl_ms"], 600_000);
    assert_eq!(updated["explicit"]["ttl_ms"], 600_000);
    assert_eq!(
        updated["effective"]["lease_ms"], 30_000,
        "an unset field still reports its default"
    );
}

#[tokio::test]
async fn l4_a_policy_that_cannot_work_is_refused_with_a_remedy() {
    let (hub, _dir) = spawn().await;
    bootstrap(&hub, "notify.kenny", "printer").await;

    for (what, body, expected_status) in [
        ("a zero lease", r#"{"lease_ms":0}"#, 400),
        ("no attempts at all", r#"{"max_attempts":0}"#, 400),
        ("a negative backoff", r#"{"backoff_ms":-5}"#, 400),
        ("a zero TTL", r#"{"ttl_ms":0}"#, 400),
        ("a body that is not JSON", "not json", 400),
    ] {
        let response = set_policy(&hub, "notify.kenny", "printer", body).await;
        let status = response.status().as_u16();
        let json = body_json(response).await;
        assert_eq!(status, expected_status, "{what}: got {json}");
        assert!(
            json["remedy"].as_str().unwrap_or_default().len() > 20,
            "{what}: must carry a remedy, got {json}"
        );
    }
}

#[tokio::test]
async fn l4_nack_returns_a_message_without_waiting_for_the_lease() {
    let (hub, _dir) = spawn().await;
    bootstrap(&hub, "jobs.transcode", "worker").await;
    assert_eq!(
        set_policy(&hub, "jobs.transcode", "worker", r#"{"backoff_ms":0}"#)
            .await
            .status(),
        200
    );

    let id = publish(&hub, "jobs.transcode", r#"{"job":1}"#).await;
    assert_eq!(
        receive(&hub, "jobs.transcode", "worker").await.status(),
        200
    );

    let nacked = reqwest::Client::new()
        .post(hub.url(&format!("/t/jobs.transcode/nack/{id}?as=worker")))
        .send()
        .await
        .expect("a response");
    assert_eq!(nacked.status(), 200);
    assert_eq!(body_json(nacked).await["outcome"], "redelivered");

    let again = receive(&hub, "jobs.transcode", "worker").await;
    assert_eq!(again.status(), 200);
    assert_eq!(header(&again, "mailbox-id").as_deref(), Some(id.as_str()));
    assert_eq!(header(&again, "mailbox-attempt").as_deref(), Some("2"));
}

#[tokio::test]
async fn l4_dead_letters_are_listed_with_their_payload_and_can_be_requeued() {
    let (hub, _dir) = spawn().await;
    bootstrap(&hub, "print.receipt", "printer").await;

    let id = publish(&hub, "print.receipt", r#"{"receipt":"kapot"}"#).await;
    assert_eq!(
        receive(&hub, "print.receipt", "printer").await.status(),
        200
    );

    // A poison pill: this payload will never work, so skip the retries.
    let nacked = reqwest::Client::new()
        .post(hub.url(&format!("/t/print.receipt/nack/{id}?as=printer&dead=true")))
        .send()
        .await
        .expect("a response");
    assert_eq!(nacked.status(), 200);
    assert_eq!(body_json(nacked).await["outcome"], "dead_lettered");

    let listed = body_json(
        reqwest::get(hub.url("/api/t/print.receipt/subs/printer/dead"))
            .await
            .expect("a response"),
    )
    .await;
    let letters = listed["dead_letters"].as_array().expect("an array");
    assert_eq!(letters.len(), 1);
    assert_eq!(letters[0]["id"], id);
    assert_eq!(
        letters[0]["payload_text"], r#"{"receipt":"kapot"}"#,
        "the payload is visible, which is the point of a dead-letter list"
    );
    assert_eq!(letters[0]["truncated"], false);
    assert!(letters[0]["dead_at"].is_i64());

    let requeued = reqwest::Client::new()
        .post(hub.url(&format!(
            "/api/t/print.receipt/subs/printer/dead/{id}/requeue"
        )))
        .send()
        .await
        .expect("a response");
    assert_eq!(requeued.status(), 200);

    let redelivered = receive(&hub, "print.receipt", "printer").await;
    assert_eq!(redelivered.status(), 200);
    assert_eq!(
        header(&redelivered, "mailbox-attempt").as_deref(),
        Some("1"),
        "a requeued message starts its attempts over"
    );

    let empty = body_json(
        reqwest::get(hub.url("/api/t/print.receipt/subs/printer/dead"))
            .await
            .expect("a response"),
    )
    .await;
    assert!(
        empty["dead_letters"]
            .as_array()
            .expect("an array")
            .is_empty()
    );
}

#[tokio::test]
async fn l4_dead_letters_survive_a_restart() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let hub = spawn_at(dir.path()).await;

    bootstrap(&hub, "print.receipt", "printer").await;
    let id = publish(&hub, "print.receipt", r#"{"receipt":"kapot"}"#).await;
    assert_eq!(
        receive(&hub, "print.receipt", "printer").await.status(),
        200
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

    hub.shutdown().await;
    let hub = spawn_at(dir.path()).await;

    let listed = body_json(
        reqwest::get(hub.url("/api/t/print.receipt/subs/printer/dead"))
            .await
            .expect("a response"),
    )
    .await;
    let letters = listed["dead_letters"].as_array().expect("an array");
    assert_eq!(
        letters.len(),
        1,
        "a dead letter waits for a human, so it has to outlive a restart"
    );
    assert_eq!(letters[0]["id"], id);
}

#[tokio::test]
async fn l4_requeueing_something_that_is_not_dead_is_refused() {
    let (hub, _dir) = spawn().await;
    bootstrap(&hub, "print.receipt", "printer").await;
    let id = publish(&hub, "print.receipt", r#"{"receipt":1}"#).await;

    let response = reqwest::Client::new()
        .post(hub.url(&format!(
            "/api/t/print.receipt/subs/printer/dead/{id}/requeue"
        )))
        .send()
        .await
        .expect("a response");
    let status = response.status().as_u16();
    let json = body_json(response).await;

    assert_eq!(status, 409, "got {json}");
    assert!(
        json["error"]
            .as_str()
            .unwrap_or_default()
            .contains("pending"),
        "the error names the state it actually found: {json}"
    );
    assert!(json["remedy"].as_str().unwrap_or_default().len() > 20);
}
