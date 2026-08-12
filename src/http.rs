//! The HTTP surface (AR2). Routes translate HTTP into engine calls and
//! back; business logic stays in [`crate::engine`].
//!
//! The three verbs (K1 publish, K2 long-poll receive, K3 ack) arrive in
//! L2. L0 serves only the health endpoint, so that the container, the
//! compose healthcheck and CI have something real to check.

use axum::Router;
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::get;

pub fn router() -> Router {
    Router::new().route("/healthz", get(healthz))
}

/// Liveness for Uptime Kuma and the container healthcheck (W6). L5 grows
/// this into a real readiness check (store writable, sweeper alive) and
/// adds the `--healthcheck` flag the shell-less image needs.
async fn healthz() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"status":"ok"}"#,
    )
}
