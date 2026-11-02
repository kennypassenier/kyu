//! Who may call what on the machine API.
//!
//! The kit's door only decides *whether* a caller is known; these layers
//! decide what each kind of caller may do. An app token (`Caller::Client`)
//! is for publishing and consuming; managing the hub is the login token's
//! job, and the consuming verbs additionally refuse a browser session, so a
//! page on another site cannot drive a poll with the cookie the browser
//! attaches by itself.
//!
//! A request with no `Caller` at all comes from a hub mounted without the
//! kit's door (the in-process tests): there is nobody to refuse.

use axum::extract::Request;
use axum::http::{StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use chassis::Caller;

use crate::http::error::ApiError;

/// Management routes: the admin login token only.
pub async fn require_admin(request: Request, next: Next) -> Response {
    if let Some(Caller::Client { name, .. }) = request.extensions().get::<Caller>() {
        return ApiError::new(
            StatusCode::FORBIDDEN,
            format!("the app token of {name:?} may publish and consume, not manage the hub"),
            "send the hub's login token (KYU_TOKEN) as Authorization: Bearer <token> for \
             this call; app tokens keep publish, receive, ack, nack and a subscription's \
             policy.",
        )
        .into_response();
    }
    next.run(request).await
}

/// Consuming routes: a caller must send the bearer token itself, not lean
/// on a session cookie.
pub async fn require_bearer(request: Request, next: Next) -> Response {
    let known = request.extensions().get::<Caller>().is_some();
    if known && !request.headers().contains_key(header::AUTHORIZATION) {
        return ApiError::new(
            StatusCode::UNAUTHORIZED,
            "this route does not accept a browser session",
            "send the token explicitly: Authorization: Bearer <token>. The dashboard \
             never calls this route; it only prints commands for you to run.",
        )
        .into_response();
    }
    next.run(request).await
}
