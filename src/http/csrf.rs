//! Cross-origin protection for state-changing requests.
//!
//! This layer predates the door (W2) and still earns its place after it: a
//! hub may deliberately run with no token at all, and even a protected one
//! carries a session cookie that a cross-site form post would otherwise ride
//! on. The threat model assumes the LAN is trusted (N3); a browser breaks
//! that assumption, because a page anywhere on the internet can make the
//! owner's browser
//! POST to `http://hub.lan:8080/...`, and a form post is a "simple request"
//! that needs no preflight and no readable response — the side effect has
//! already happened. That is how an internet-side attacker reaches an
//! intranet service that "only the LAN can see".
//!
//! The rule: if a request announces an `Origin` and it is not ours, refuse.
//! Browsers always send `Origin` on cross-origin state-changing requests;
//! curl, scripts and the HA integrations send none at all, so the API keeps
//! working exactly as documented.

use axum::extract::Request;
use axum::http::{Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use super::error::ApiError;

pub async fn same_origin_only(request: Request, next: Next) -> Response {
    // Reads change nothing, and blocking them would break linking to the
    // dashboard from anywhere.
    if matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    ) {
        return next.run(request).await;
    }

    let headers = request.headers();
    let Some(origin) = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    else {
        // No Origin at all: not a browser. This is every script, every curl,
        // every automation — the ordinary way the hub is used.
        return next.run(request).await;
    };

    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let origin_authority = origin.split_once("://").map(|(_, rest)| rest).unwrap_or("");

    if !origin_authority.is_empty() && origin_authority == host {
        return next.run(request).await;
    }

    tracing::warn!(
        origin,
        host,
        method = %request.method(),
        "refused a cross-origin state-changing request"
    );

    ApiError::new(
        StatusCode::FORBIDDEN,
        format!("this request came from another origin ({origin})"),
        "mailbox refuses state-changing requests that a browser reports as \
         cross-origin: an unprotected hub has nothing else to fall back on, \
         and a protected one would otherwise let a foreign page ride your \
         session cookie. \
         Call the API from a script or a terminal — those send no Origin \
         header — or use the dashboard served by the hub itself.",
    )
    .into_response()
}
