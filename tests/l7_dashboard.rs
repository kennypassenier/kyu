//! [L7] The dashboard (K10, W9, AR11) over real HTTP.
//!
//! Templates are checked at runtime rather than at compile time (T4), so
//! every page is rendered here with seeded state — that is the compensation
//! owed for choosing minijinja, and these tests are what makes the choice
//! safe. The last test executes the snippets the dashboard prints, because
//! an example that does not work is worse than no example at all (S1).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use kyu::engine::Engine;
use kyu::engine::clock::{Clock, SystemClock};
use kyu::http::{AppState, Limits, router_with_probes};
use kyu::store::Store;
use kyu::sweeper::Heartbeat;

struct Hub {
    addr: SocketAddr,
}

impl Hub {
    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }
}

async fn spawn() -> (Hub, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let store = Arc::new(Store::open(dir.path()).expect("a store"));
    let engine = Arc::new(Engine::new(store, Arc::new(SystemClock)));
    let state = AppState::new(
        engine,
        Limits {
            max_body_bytes: 1024 * 1024,
            default_wait_s: 1,
            max_wait_s: 300,
            recheck_interval: Duration::from_millis(200),
        },
        Heartbeat::starting_at(SystemClock.now_ms()),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a port");
    let addr = listener.local_addr().expect("an address");
    tokio::spawn(async move {
        let _ = axum::serve(listener, router_with_probes(state)).await;
    });

    (Hub { addr }, dir)
}

async fn publish(hub: &Hub, topic: &str, content_type: &str, body: Vec<u8>) -> String {
    let response = reqwest::Client::new()
        .post(hub.url(&format!("/t/{topic}")))
        .header("content-type", content_type)
        .body(body)
        .send()
        .await
        .expect("a response");
    assert_eq!(response.status(), 201);
    let json: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("a body")).expect("JSON");
    json["id"].as_str().expect("an id").to_string()
}

/// Turns rendered HTML back into the text a browser would show. minijinja
/// escapes `/` and `"` as entities, which is right for the page and wrong
/// for a test that wants to run what is printed.
fn unescape(html: &str) -> String {
    html.replace("&#x2f;", "/")
        .replace("&#x27;", "'")
        .replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

async fn page(hub: &Hub, path: &str) -> String {
    let response = reqwest::get(hub.url(path)).await.expect("a response");
    assert_eq!(response.status(), 200, "{path} must render");
    response.text().await.expect("a body")
}

async fn subscribe(hub: &Hub, topic: &str, subscription: &str) {
    let response = reqwest::get(hub.url(&format!("/t/{topic}/next?as={subscription}&wait=0")))
        .await
        .expect("a response");
    assert!(response.status() == 204 || response.status() == 200);
}

#[tokio::test]
async fn l7_the_index_lists_every_topic_with_its_counts() {
    let (hub, _dir) = spawn().await;
    publish(&hub, "notify.kenny", "application/json", b"{}".to_vec()).await;
    subscribe(&hub, "notify.kenny", "printer").await;
    publish(&hub, "notify.kenny", "application/json", b"{}".to_vec()).await;
    publish(&hub, "jobs.transcode", "application/json", b"{}".to_vec()).await;

    let body = page(&hub, "/").await;

    assert!(body.contains("notify.kenny"), "the topic is listed");
    assert!(body.contains("jobs.transcode"));
    assert!(
        body.contains("kyu.events"),
        "the hub's own event topic is a topic like any other"
    );
    assert!(
        body.contains("/t/notify.kenny/dashboard"),
        "and links to it"
    );
}

#[tokio::test]
async fn l7_the_index_always_shows_how_to_start_a_topic() {
    let (hub, _dir) = spawn().await;
    let body = unescape(&page(&hub, "/").await);

    // A fresh hub is never literally empty — kyu.events is always
    // there — so the getting-started example is unconditional rather than
    // an empty state nobody would ever see.
    assert!(
        body.contains("Start a topic") && body.contains("curl"),
        "the index must always show how to publish: {body}"
    );
    assert!(body.contains("/t/notify.kenny"));
}

#[tokio::test]
async fn l7_the_topic_page_shows_subscriptions_backlogs_and_policy() {
    let (hub, _dir) = spawn().await;
    publish(&hub, "notify.kenny", "application/json", b"{}".to_vec()).await;
    subscribe(&hub, "notify.kenny", "printer").await;
    subscribe(&hub, "notify.kenny", "ha-forwarder").await;
    reqwest::Client::new()
        .put(hub.url("/api/t/notify.kenny/subs/ha-forwarder/policy"))
        .body(r#"{"ttl_ms":600000}"#)
        .send()
        .await
        .expect("a policy");
    publish(
        &hub,
        "notify.kenny",
        "application/json",
        br#"{"title":"Backup klaar"}"#.to_vec(),
    )
    .await;

    let body = page(&hub, "/t/notify.kenny/dashboard").await;

    assert!(body.contains("printer") && body.contains("ha-forwarder"));
    assert!(body.contains("active"), "states are shown");
    assert!(
        body.contains("(default)"),
        "a value that comes from a default says so, instead of looking chosen"
    );
    assert!(body.contains("ttl 600000ms"), "an explicit policy is shown");
    assert!(
        body.contains("Backup klaar"),
        "recent payloads are visible: {body}"
    );
}

#[tokio::test]
async fn l7_a_payload_cannot_script_the_dashboard() {
    let (hub, _dir) = spawn().await;
    // The classic: stored XSS delivered through a queue.
    publish(
        &hub,
        "notify.kenny",
        "application/json",
        br#"{"title":"<script>alert('pwned')</script>"}"#.to_vec(),
    )
    .await;

    let body = page(&hub, "/t/notify.kenny/dashboard").await;

    assert!(
        !body.contains("<script>alert"),
        "an unescaped payload would make every consumer a way in: {body}"
    );
    assert!(
        body.contains("&lt;script&gt;"),
        "it must still be readable, just inert"
    );
}

#[tokio::test]
async fn l7_binary_and_oversized_payloads_are_marked_not_mangled() {
    let (hub, _dir) = spawn().await;
    publish(
        &hub,
        "print.receipt",
        "application/octet-stream",
        vec![0x00, 0xff, 0x1b, 0x80],
    )
    .await;
    publish(
        &hub,
        "print.receipt",
        "text/plain",
        "x".repeat(9000).into_bytes(),
    )
    .await;

    let body = page(&hub, "/t/print.receipt/dashboard").await;

    assert!(
        body.contains("binary payload (4 bytes)"),
        "binary is announced with its size: {body}"
    );
    assert!(
        body.contains("showing the first 4096 of 9000 bytes"),
        "and truncation is never silent"
    );
}

#[tokio::test]
async fn l7_a_topic_nobody_polls_explains_the_bootstrap_order() {
    let (hub, _dir) = spawn().await;
    publish(&hub, "notify.kenny", "application/json", b"{}".to_vec()).await;

    let body = page(&hub, "/t/notify.kenny/dashboard").await;

    assert!(
        body.contains("first polls"),
        "the G7 trap is the one thing a newcomer must be told: {body}"
    );
    assert!(body.contains("from=beginning"));
}

#[tokio::test]
async fn l7_an_unknown_topic_answers_404_with_a_remedy() {
    let (hub, _dir) = spawn().await;
    let response = reqwest::get(hub.url("/t/nope.nothing/dashboard"))
        .await
        .expect("a response");
    assert_eq!(response.status(), 404);
    let json: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("a body")).expect("JSON");
    assert!(json["remedy"].as_str().unwrap_or_default().len() > 20);
}

#[tokio::test]
async fn l7_the_test_publish_form_puts_a_real_message_on_the_topic() {
    let (hub, _dir) = spawn().await;
    publish(&hub, "notify.kenny", "application/json", b"{}".to_vec()).await;
    subscribe(&hub, "notify.kenny", "printer").await;

    let response = reqwest::Client::new()
        .post(hub.url("/t/notify.kenny/dashboard/publish"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body("payload=%7B%22from%22%3A%22dashboard%22%7D")
        .send()
        .await
        .expect("a response");
    assert!(
        response.status().is_success() || response.status().is_redirection(),
        "the form posts and returns to the page"
    );

    let received = reqwest::get(hub.url("/t/notify.kenny/next?as=printer&wait=0"))
        .await
        .expect("a response");
    assert_eq!(received.status(), 200);
    assert_eq!(
        received.text().await.expect("a body"),
        r#"{"from":"dashboard"}"#,
        "W9 exists to answer 'is the producer broken or the consumer' in one click"
    );
}

#[tokio::test]
async fn l7_the_snippets_the_dashboard_prints_actually_work() {
    let (hub, _dir) = spawn().await;
    publish(&hub, "notify.kenny", "application/json", b"{}".to_vec()).await;
    subscribe(&hub, "notify.kenny", "printer").await;
    publish(
        &hub,
        "notify.kenny",
        "application/json",
        br#"{"title":"Backup klaar"}"#.to_vec(),
    )
    .await;

    // Read the page the way a browser shows it, then run what it printed.
    // This is the mechanism behind S1: the example is not documentation
    // about the hub, it is the hub describing itself.
    let body = unescape(&page(&hub, "/t/notify.kenny/dashboard").await);

    // The command as the page displays it, not as it is stashed in the
    // reveal/copy data attributes: those hold the same text, so the line has
    // to be pinned to the one a reader would actually see and retype.
    let receive = body
        .lines()
        .find(|line| {
            line.trim_start().starts_with("curl")
                && line.contains("/next?as=")
                && line.contains("envelope=json")
        })
        .expect("the envelope snippet must be on the page")
        .to_string();
    assert!(
        receive.contains("as=printer"),
        "the snippet names a subscription that exists: {receive}"
    );

    let path = receive
        .split('"')
        .nth(1)
        .expect("the snippet quotes its URL");
    let url = hub.url(path);

    let response = reqwest::get(&url).await.expect("the snippet must run");
    assert_eq!(
        response.status(),
        200,
        "the command the dashboard printed must actually return a message: {url}"
    );
    let envelope: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("a body")).expect("JSON");
    assert_eq!(envelope["payload"]["title"], "Backup klaar");
}
