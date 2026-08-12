//! [L0] Walking-skeleton tests: the server binds a real socket and
//! answers over real HTTP (standing rule 9 — no mocked transport).

use std::net::SocketAddr;
use std::sync::Arc;

use mailbox::engine::Engine;
use mailbox::engine::clock::SystemClock;
use mailbox::http::{AppState, Limits, router};
use mailbox::store::Store;

/// Starts the router on an ephemeral port and returns its address. The
/// task is left running; the test process exiting cleans it up.
async fn spawn_server() -> SocketAddr {
    let store = Arc::new(Store::open_in_memory().expect("an in-memory store"));
    let engine = Arc::new(Engine::new(store, Arc::new(SystemClock)));
    let state = AppState::new(
        engine,
        Limits {
            max_body_bytes: 1024,
            default_wait_s: 1,
            max_wait_s: 300,
            recheck_interval: std::time::Duration::from_secs(5),
        },
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("binding an ephemeral port must succeed");
    let addr = listener
        .local_addr()
        .expect("a bound socket has an address");

    tokio::spawn(async move {
        axum::serve(listener, router(state))
            .await
            .expect("the test server must not fail");
    });

    addr
}

#[tokio::test]
async fn l0_healthz_returns_200_json() {
    let addr = spawn_server().await;

    let response = reqwest::get(format!("http://{addr}/healthz"))
        .await
        .expect("the server must answer");

    assert_eq!(response.status(), 200);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("application/json")
    );
    assert_eq!(response.text().await.expect("a body"), r#"{"status":"ok"}"#);
}

#[tokio::test]
async fn l0_an_unknown_route_returns_404() {
    let addr = spawn_server().await;

    let response = reqwest::get(format!("http://{addr}/nothing-here"))
        .await
        .expect("the server must answer");

    assert_eq!(response.status(), 404);
}
