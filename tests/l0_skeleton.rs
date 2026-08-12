//! [L0] Walking-skeleton tests: the server binds a real socket and
//! answers over real HTTP (standing rule 9 — no mocked transport).

use std::net::SocketAddr;

use mailbox::http;

/// Starts the router on an ephemeral port and returns its address. The
/// task is left running; the test process exiting cleans it up.
async fn spawn_server() -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("binding an ephemeral port must succeed");
    let addr = listener
        .local_addr()
        .expect("a bound socket has an address");

    tokio::spawn(async move {
        axum::serve(listener, http::router())
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
async fn l0_unknown_route_returns_404() {
    let addr = spawn_server().await;

    let response = reqwest::get(format!("http://{addr}/t/notify.kenny"))
        .await
        .expect("the server must answer");

    // The three verbs arrive in L2; until then this route genuinely does
    // not exist, and the skeleton must say so rather than 500.
    assert_eq!(response.status(), 404);
}
