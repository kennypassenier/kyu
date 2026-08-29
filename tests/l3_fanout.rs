//! [L3] Fan-out and competing consumers (K4, S5).
//!
//! The two delivery patterns come from one concept — the subscription name
//! — so these tests are what keep them from drifting apart:
//!
//! - different names on a topic each receive every message and settle it
//!   independently (Kenny's B-and-C case)
//! - one name shared by several processes competes, so each message is
//!   handled exactly once

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use kyu::engine::Engine;
use kyu::engine::clock::SystemClock;
use kyu::http::{AppState, Limits, router};
use kyu::store::Store;
use kyu::sweeper::Heartbeat;
use serde_json::Value;
use tokio::task::JoinHandle;

struct Hub {
    addr: SocketAddr,
    store: Arc<Store>,
    _server: JoinHandle<()>,
}

impl Hub {
    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }
}

async fn spawn() -> (Hub, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let store = Arc::new(Store::open(dir.path()).expect("the store must open"));
    let engine = Arc::new(Engine::new(store.clone(), Arc::new(SystemClock)));
    let heartbeat = Heartbeat::starting_at(i64::MAX / 2);
    let state = AppState::new(
        engine,
        Limits {
            max_body_bytes: 1024 * 1024,
            default_wait_s: 2,
            max_wait_s: 300,
            recheck_interval: Duration::from_millis(200),
        },
        heartbeat.clone(),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("an ephemeral port");
    let addr = listener.local_addr().expect("a bound address");
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, router(state)).await;
    });

    (
        Hub {
            addr,
            store,
            _server: server,
        },
        dir,
    )
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
    let text = response.text().await.expect("a body");
    let json: Value = serde_json::from_str(&text).expect("JSON");
    json["id"].as_str().expect("an id").to_string()
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

fn message_id(response: &reqwest::Response) -> String {
    response
        .headers()
        .get("kyu-id")
        .and_then(|value| value.to_str().ok())
        .expect("a delivered message carries its id")
        .to_string()
}

/// Brings a subscription into existence (G7: it starts at "now").
async fn subscribe(hub: &Hub, topic: &str, subscription: &str) {
    let response = receive(hub, topic, subscription).await;
    assert_eq!(response.status(), 204);
}

/// Drains a subscription until it reports empty, acking as it goes.
async fn drain(hub: &Hub, topic: &str, subscription: &str) -> Vec<String> {
    let mut ids = Vec::new();
    loop {
        let response = receive(hub, topic, subscription).await;
        if response.status() == 204 {
            return ids;
        }
        assert_eq!(response.status(), 200);
        let id = message_id(&response);
        assert_eq!(ack(hub, topic, &id, subscription).await.status(), 200);
        ids.push(id);
    }
}

fn backlog(hub: &Hub, subscription: &str) -> i64 {
    hub.store.with_conn(|conn| {
        conn.query_row(
            "SELECT count(*)
               FROM deliveries d
               JOIN subscriptions s ON s.id = d.sub_id
              WHERE s.name = ?1 AND d.state = 'pending'",
            [subscription],
            |row| row.get(0),
        )
        .expect("counting the backlog")
    })
}

#[tokio::test]
async fn l3_two_subscriptions_each_receive_every_message() {
    let (hub, _dir) = spawn().await;
    publish(&hub, "notify.kenny", r#"{"bootstrap":true}"#).await;
    subscribe(&hub, "notify.kenny", "ha-forwarder").await;
    subscribe(&hub, "notify.kenny", "receipt-printer").await;

    let first = publish(&hub, "notify.kenny", r#"{"n":1}"#).await;
    let second = publish(&hub, "notify.kenny", r#"{"n":2}"#).await;

    let forwarder = drain(&hub, "notify.kenny", "ha-forwarder").await;
    let printer = drain(&hub, "notify.kenny", "receipt-printer").await;

    assert_eq!(forwarder, vec![first.clone(), second.clone()]);
    assert_eq!(
        printer,
        vec![first, second],
        "each subscription receives every message, in publish order"
    );
}

#[tokio::test]
async fn l3_an_ack_settles_one_subscription_only() {
    let (hub, _dir) = spawn().await;
    publish(&hub, "notify.kenny", r#"{"bootstrap":true}"#).await;
    subscribe(&hub, "notify.kenny", "ha-forwarder").await;
    subscribe(&hub, "notify.kenny", "receipt-printer").await;

    let id = publish(&hub, "notify.kenny", r#"{"title":"Backup klaar"}"#).await;

    // One consumer handles and settles it.
    let response = receive(&hub, "notify.kenny", "ha-forwarder").await;
    assert_eq!(message_id(&response), id);
    assert_eq!(
        ack(&hub, "notify.kenny", &id, "ha-forwarder")
            .await
            .status(),
        200
    );

    // The other has not been consulted, so its copy is untouched. This is
    // the whole reason subscriptions exist rather than one shared queue.
    assert_eq!(backlog(&hub, "receipt-printer"), 1);
    let response = receive(&hub, "notify.kenny", "receipt-printer").await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        message_id(&response),
        id,
        "an ack by one subscription must not consume another's copy"
    );
}

#[tokio::test]
async fn l3_a_dead_subscription_does_not_delay_its_sibling() {
    let (hub, _dir) = spawn().await;
    publish(&hub, "notify.kenny", r#"{"bootstrap":true}"#).await;
    subscribe(&hub, "notify.kenny", "alive").await;
    subscribe(&hub, "notify.kenny", "dead").await;

    let count = 25;
    let mut published = Vec::new();
    for n in 0..count {
        published.push(publish(&hub, "notify.kenny", &format!(r#"{{"n":{n}}}"#)).await);
    }

    // "dead" never polls: its consumer is switched off, exactly like the
    // printer LXC being down for a week.
    let started = Instant::now();
    let drained = drain(&hub, "notify.kenny", "alive").await;
    let elapsed = started.elapsed();

    assert_eq!(
        drained, published,
        "the live subscription drains everything, in order"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "and it is not slowed down by the dead one: {elapsed:?}"
    );
    assert_eq!(
        backlog(&hub, "dead"),
        count,
        "the dead subscription's backlog waits for it, complete and untouched"
    );
}

#[tokio::test]
async fn l3_competing_consumers_share_the_work_without_duplicating_it() {
    let (hub, _dir) = spawn().await;
    publish(&hub, "jobs.transcode", r#"{"bootstrap":true}"#).await;
    // One name, so these are competitors rather than independent readers.
    subscribe(&hub, "jobs.transcode", "worker").await;

    let count = 40usize;
    let mut published = Vec::new();
    for n in 0..count {
        published.push(publish(&hub, "jobs.transcode", &format!(r#"{{"job":{n}}}"#)).await);
    }

    // Four workers, all polling as "worker", all at once.
    let mut workers = Vec::new();
    for _ in 0..4 {
        let base = hub.url("");
        workers.push(tokio::spawn(async move {
            let client = reqwest::Client::new();
            let mut handled = Vec::new();
            loop {
                let response = client
                    .get(format!("{base}/t/jobs.transcode/next?as=worker&wait=0"))
                    .send()
                    .await
                    .expect("a response");
                if response.status() == 204 {
                    return handled;
                }
                assert_eq!(response.status(), 200);
                let id = response
                    .headers()
                    .get("kyu-id")
                    .and_then(|value| value.to_str().ok())
                    .expect("an id")
                    .to_string();
                let acked = client
                    .post(format!("{base}/t/jobs.transcode/ack/{id}?as=worker"))
                    .send()
                    .await
                    .expect("a response");
                assert_eq!(acked.status(), 200);
                handled.push(id);
            }
        }));
    }

    let mut all = Vec::new();
    let mut per_worker = Vec::new();
    for worker in workers {
        let handled = worker.await.expect("a worker task");
        per_worker.push(handled.len());
        all.extend(handled);
    }

    let mut sorted = all.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        all.len(),
        "no message may be handled twice: {} handled, {} distinct",
        all.len(),
        sorted.len()
    );
    assert_eq!(
        sorted.len(),
        count,
        "and none may be lost: {per_worker:?} handled per worker"
    );

    let mut expected = published;
    expected.sort();
    assert_eq!(sorted, expected);
    assert_eq!(backlog(&hub, "worker"), 0);
}

#[tokio::test]
async fn l3_an_archived_subscription_receives_nothing_new() {
    let (hub, _dir) = spawn().await;
    publish(&hub, "notify.kenny", r#"{"bootstrap":true}"#).await;
    subscribe(&hub, "notify.kenny", "retired").await;
    subscribe(&hub, "notify.kenny", "current").await;

    // K11's lifecycle lands in L6; archiving directly here is enough to
    // prove the publish path already respects the state, which is what
    // makes AR3's retention rule safe.
    hub.store.with_conn(|conn| {
        conn.execute(
            "UPDATE subscriptions SET state = 'archived' WHERE name = ?1",
            ["retired"],
        )
        .expect("archiving the subscription")
    });

    let id = publish(&hub, "notify.kenny", r#"{"n":1}"#).await;

    assert_eq!(
        backlog(&hub, "retired"),
        0,
        "an archived subscription must not accumulate new messages"
    );
    assert_eq!(
        backlog(&hub, "current"),
        1,
        "while an active one still receives them"
    );

    let response = receive(&hub, "notify.kenny", "current").await;
    assert_eq!(message_id(&response), id);
}

#[tokio::test]
async fn l3_the_same_name_on_two_topics_is_two_subscriptions() {
    let (hub, _dir) = spawn().await;
    publish(&hub, "notify.kenny", r#"{"bootstrap":true}"#).await;
    publish(&hub, "jobs.transcode", r#"{"bootstrap":true}"#).await;
    subscribe(&hub, "notify.kenny", "worker").await;
    subscribe(&hub, "jobs.transcode", "worker").await;

    let notify_id = publish(&hub, "notify.kenny", r#"{"which":"notify"}"#).await;
    let jobs_id = publish(&hub, "jobs.transcode", r#"{"which":"jobs"}"#).await;

    // Subscriptions are scoped to their topic, so one name polling two
    // topics keeps two independent positions.
    let from_notify = receive(&hub, "notify.kenny", "worker").await;
    assert_eq!(message_id(&from_notify), notify_id);
    let from_jobs = receive(&hub, "jobs.transcode", "worker").await;
    assert_eq!(message_id(&from_jobs), jobs_id);
}
