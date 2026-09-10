//! The kit smoke test every project starts with (K34): it proves the
//! service still starts, still lets the admin in, still issues a client
//! token the API accepts, and still serves the dashboard — the four things
//! that break silently after a kit bump or a config change.
//!
//! This file is the kit's, not the project's: `chassis sync` rewrites it.
//! Put this service's own tests in other files under `tests/`.

use axum::routing::post;
use axum::{Json, Router};
use chassis::testing::TestApp;
use chassis::{AppSpec, Caller};
use reqwest::Method;

// Held as constants, not written into each line: the names are rendered by
// the scaffold and a long one would otherwise change how rustfmt wraps the
// code around them, so a generated project could fail its own fmt gate.
const NAME: &str = "kyu";
const REPO: &str = "kennypassenier/kyu";

/// The example route from `src/main.rs`. An integration test cannot import
/// the binary's `main`, so the smoke test registers the route itself; when
/// this service grows real routes, register those instead.
async fn receive(caller: Caller, Json(body): Json<serde_json::Value>) -> Json<serde_json::Value> {
    let from = match caller {
        Caller::Client { name, .. } => name,
        Caller::Admin => "admin".to_string(),
    };
    Json(serde_json::json!({ "from": from, "received": body }))
}

#[tokio::test]
async fn kit_smoke_login_client_token_api_call_and_dashboard() {
    let spec = AppSpec {
        name: NAME,
        version: env!("CARGO_PKG_VERSION"),
        repository: Some(REPO),
        ..Default::default()
    };
    let mut app = TestApp::start_with(spec, Router::new(), |app| {
        app.api_routes(Router::new().route("/v1/example", post(receive)));
    })
    .await;

    let (status, health) = app.get_json("/healthz").await;
    assert_eq!(status, 200, "{health}");
    assert_eq!(
        health["version"],
        env!("CARGO_PKG_VERSION"),
        "/healthz reports this build's version"
    );

    app.login().await;
    let client = app.issue_client("smoke", &[]).await;

    let (status, answer) = TestApp::send_json(
        app.bearer(Method::POST, "/v1/example", &client.token)
            .json(&serde_json::json!({ "hello": "world" })),
    )
    .await;
    assert_eq!(status, 200, "the API accepts the client token: {answer}");
    assert_eq!(answer["from"], "smoke", "the route sees the client by name");

    let (status, html) = app.page("/").await;
    assert_eq!(status, 200, "the admin's dashboard is served: {html}");
    assert!(html.contains(NAME), "the dashboard names the service");

    app.shutdown().await;
}
