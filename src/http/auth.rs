//! The door (W2).
//!
//! Two ways in, both carrying the same kind of secret:
//!
//! - `Authorization: Bearer <token>` — what every script, curl call and
//!   integration uses. The token is either the bootstrap token from the
//!   environment or one generated for a registered app.
//! - A `kyu_session` cookie — what a browser gets after logging in,
//!   because a browser will not attach a bearer header on its own.
//!
//! What is deliberately left open: `/healthz` and `/metrics`. A monitoring
//! stack that fails closed lies to you during an outage, which is the one
//! moment you believe it. Neither reveals a payload; `/metrics` names topics
//! and subscriptions, and that was the accepted trade at the mini-round.
//!
//! Fail-closed by construction: this layer is attached to a sub-router, so a
//! route added there is guarded whether or not anyone remembered to think
//! about it. Only the handful of routes in the open router are reachable
//! without a token.

use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect, Response};

use super::AppState;
use super::error::ApiError;
use crate::config::Auth;

/// The cookie a logged-in browser carries. `HttpOnly` so a script injected
/// into a payload cannot read it (AR11 treats payloads as untrusted), `Lax`
/// so following a link to the dashboard still works while a cross-site form
/// post does not carry it.
pub const SESSION_COOKIE: &str = "kyu_session";
/// How long "remember me" lasts. Long enough that you are not logging in
/// every day, short enough that a borrowed laptop forgets eventually.
pub const REMEMBER_SECONDS: i64 = 60 * 60 * 24 * 30;

pub fn set_cookie_value(token: &str, remember: bool) -> String {
    let base = format!("{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax");
    if remember {
        format!("{base}; Max-Age={REMEMBER_SECONDS}")
    } else {
        base
    }
}

pub fn clear_cookie_value() -> String {
    format!("{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0")
}

/// Pulls the session cookie out of a `Cookie` header.
///
/// Hand-rolled rather than pulling in a cookie crate for one lookup (T6).
/// Matching is on the exact name, so `not_kyu_session=` cannot be
/// mistaken for ours.
pub fn session_cookie(headers: &axum::http::HeaderMap) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    raw.split(';').find_map(|part| {
        let (name, value) = part.split_once('=')?;
        (name.trim() == SESSION_COOKIE).then(|| value.trim().to_string())
    })
}

/// The bearer token of a request, if it carries one.
pub fn bearer_token(headers: &axum::http::HeaderMap) -> Option<String> {
    let raw = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, value) = raw.split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("bearer")
        .then(|| value.trim().to_string())
}

/// Who is calling, once a credential has been accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Caller {
    /// The hub has no token configured. Everyone is the admin.
    Unprotected,
    /// The bootstrap token from the environment.
    Admin,
    /// A registered app, by name. Recorded so logs can say which app acted —
    /// the reason Kenny wanted per-app tokens in the first place.
    App(String),
}

/// Checks a credential against the bootstrap token first (free) and then
/// against the registered apps (a store read).
///
/// Revocation is immediate by design: there is no cache of accepted tokens,
/// because "revoked but still working for another minute" is not a thing
/// anyone wants to reason about at the moment they are revoking something.
pub fn authenticate(state: &AppState, candidate: &str) -> Option<Caller> {
    let Auth::Protected { key, .. } = state.auth.as_ref() else {
        return Some(Caller::Unprotected);
    };
    if candidate.is_empty() {
        return None;
    }
    if state.auth.matches_bootstrap(candidate) {
        return Some(Caller::Admin);
    }
    match state.engine.app_for_token(candidate, key) {
        Ok(Some(name)) => Some(Caller::App(name)),
        Ok(None) => None,
        Err(error) => {
            // A store failure must not become an open door.
            tracing::error!(error = ?error, "cannot check a token against the app list");
            None
        }
    }
}

pub async fn require_token(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    if !state.auth.is_protected() {
        return next.run(request).await;
    }

    let headers = request.headers().clone();
    let candidate = bearer_token(&headers).or_else(|| session_cookie(&headers));

    if let Some(candidate) = candidate.as_deref()
        && let Some(caller) = authenticate(&state, candidate)
    {
        if let Caller::App(name) = &caller {
            tracing::debug!(app = %name, "authenticated");
        }
        let mut request = request;
        request.extensions_mut().insert(caller);
        return next.run(request).await;
    }

    // A browser gets sent to the login page; anything else gets a 401 it can
    // act on. Telling curl to "log in" would be useless advice, and
    // answering a browser with JSON would look like the hub is broken.
    if wants_html(&headers) {
        return Redirect::to("/login").into_response();
    }

    ApiError::new(
        StatusCode::UNAUTHORIZED,
        "this hub requires a token".to_string(),
        "send it as an Authorization header: -H 'authorization: Bearer <token>'. \
         The dashboard prints a ready-to-paste command for every topic; tokens \
         are generated on its apps page."
            .to_string(),
    )
    .into_response()
}

fn wants_html(headers: &axum::http::HeaderMap) -> bool {
    headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|accept| accept.contains("text/html"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(
                axum::http::HeaderName::from_bytes(name.as_bytes()).expect("a valid header name"),
                value.parse().expect("a valid header value"),
            );
        }
        map
    }

    #[test]
    fn p7_a_bearer_header_is_read_case_insensitively() {
        assert_eq!(
            bearer_token(&headers(&[("authorization", "Bearer abc123")])).as_deref(),
            Some("abc123")
        );
        assert_eq!(
            bearer_token(&headers(&[("authorization", "bearer abc123")])).as_deref(),
            Some("abc123")
        );
    }

    #[test]
    fn p7_other_authorization_schemes_are_not_mistaken_for_bearer() {
        assert_eq!(
            bearer_token(&headers(&[("authorization", "Basic abc123")])),
            None
        );
        assert_eq!(bearer_token(&headers(&[("authorization", "abc123")])), None);
    }

    #[test]
    fn p7_the_session_cookie_is_found_among_others() {
        let map = headers(&[("cookie", "theme=dark; kyu_session=abc123; other=1")]);
        assert_eq!(session_cookie(&map).as_deref(), Some("abc123"));
    }

    #[test]
    fn p7_a_cookie_whose_name_merely_ends_in_ours_is_ignored() {
        // "not_kyu_session" ends with our name; a sloppy `contains`
        // check would accept it and let any site name a cookie into the hub.
        let map = headers(&[("cookie", "not_kyu_session=abc123")]);
        assert_eq!(session_cookie(&map), None);
    }

    #[test]
    fn p7_no_cookie_header_is_not_an_error() {
        assert_eq!(session_cookie(&HeaderMap::new()), None);
    }

    #[test]
    fn p7_remember_me_changes_only_the_lifetime() {
        let remembered = set_cookie_value("abc", true);
        let session = set_cookie_value("abc", false);
        assert!(remembered.contains("Max-Age=2592000"), "{remembered}");
        assert!(!session.contains("Max-Age"), "{session}");
        for cookie in [&remembered, &session] {
            assert!(cookie.contains("HttpOnly"), "a script must not read it");
            assert!(cookie.contains("SameSite=Lax"), "no cross-site form posts");
        }
    }

    #[test]
    fn p7_clearing_the_cookie_expires_it_immediately() {
        let cleared = clear_cookie_value();
        assert!(cleared.contains("Max-Age=0"), "{cleared}");
        assert!(cleared.starts_with("kyu_session=;"), "{cleared}");
    }

    #[test]
    fn p7_only_html_callers_are_redirected_to_the_login_page() {
        assert!(wants_html(&headers(&[(
            "accept",
            "text/html,application/xhtml+xml"
        )])));
        assert!(!wants_html(&headers(&[("accept", "application/json")])));
        assert!(
            !wants_html(&HeaderMap::new()),
            "curl sends no Accept at all"
        );
    }
}
