//! How a message is shaped on the wire (AR2): the `kyu-*` metadata headers,
//! the raw and envelope response shapes, and the content-type safety list.

use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::json;

use crate::engine::{Claimed, NewSubscription};

/// Headers carrying the metadata a raw-body response cannot put in the
/// body (AR2). Lowercase on the wire either way; HTTP/2 and proxies
/// normalise the case, so clients must match case-insensitively.
pub(crate) const HEADER_ID: &str = "kyu-id";
pub(crate) const HEADER_TOPIC: &str = "kyu-topic";
pub(crate) const HEADER_ATTEMPT: &str = "kyu-attempt";
pub(crate) const HEADER_PUBLISHED_AT: &str = "kyu-published-at";
/// Says out loud what would otherwise be an unexplained empty answer.
pub(crate) const HEADER_NOTICE: &str = "kyu-notice";

/// A fresh subscription's first poll cannot see anything published before
/// it existed (G7). Saying so turns a confusing 204 into a lesson about how
/// the hub works — and points at the way to get that history instead.
pub(crate) fn notice(new: &NewSubscription) -> String {
    if new.retained_before == 0 {
        format!(
            "subscription {:?} was created by this poll and receives messages published from now on",
            new.name
        )
    } else {
        format!(
            "subscription {:?} was created by this poll and receives messages published from now on; \
             {} earlier message(s) on this topic predate it and were not delivered to it",
            new.name, new.retained_before
        )
    }
}

/// AR2's two response shapes. Raw is the default so payload bytes stay
/// exactly what was published; the envelope exists because parsing headers
/// in a shell is friction the re-entry test cannot afford.
pub(crate) fn render(
    claimed: Claimed,
    envelope: bool,
    created: Option<&NewSubscription>,
) -> Response {
    let attempt = claimed.attempt();
    let message = claimed.message;

    if !envelope {
        let content_type = message
            .content_type
            .clone()
            .unwrap_or_else(|| "application/octet-stream".to_string());

        let mut response = (StatusCode::OK, message.payload).into_response();
        let headers = response.headers_mut();
        if let Ok(value) = content_type.parse() {
            headers.insert(header::CONTENT_TYPE, value);
        }
        // nosniff always: it costs nothing and stops a browser deciding that
        // a text/plain payload is really HTML.
        headers.insert(
            axum::http::header::X_CONTENT_TYPE_OPTIONS,
            axum::http::HeaderValue::from_static("nosniff"),
        );
        // attachment only for the types a browser would execute. A payload
        // published as text/html would otherwise run in the hub's own origin
        // for anyone opening this URL — an iframe on a hostile page is
        // enough. Everything else still opens normally in a tab, which is
        // what the AR2 mini-round settled on: protection without making the
        // API unbrowsable.
        if executes_in_a_browser(&content_type) {
            headers.insert(
                header::CONTENT_DISPOSITION,
                axum::http::HeaderValue::from_static("attachment"),
            );
        }
        insert_str(headers, HEADER_ID, &message.id);
        insert_str(headers, HEADER_TOPIC, &claimed.topic);
        insert_str(headers, HEADER_ATTEMPT, &attempt.to_string());
        insert_str(
            headers,
            HEADER_PUBLISHED_AT,
            &message.published_at.to_string(),
        );
        return response;
    }

    // Exactly one payload key is present, and which one says how the bytes
    // were understood — never a silent transformation (G8):
    //   payload        the bytes parsed as JSON
    //   payload_text   valid UTF-8 that is not JSON
    //   payload_base64 anything else
    let mut body = json!({
        "id": message.id,
        "topic": claimed.topic,
        "attempt": attempt,
        "published_at": message.published_at,
        "content_type": message.content_type,
    });
    let map = body.as_object_mut().expect("a JSON object");

    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&message.payload) {
        map.insert("payload".to_string(), value);
    } else if let Ok(text) = String::from_utf8(message.payload.clone()) {
        map.insert("payload_text".to_string(), json!(text));
    } else {
        map.insert(
            "payload_base64".to_string(),
            json!(BASE64.encode(&message.payload)),
        );
    }

    let mut response = (StatusCode::OK, axum::Json(body)).into_response();
    if let Some(new) = created {
        insert_str(response.headers_mut(), HEADER_NOTICE, &notice(new));
    }
    response
}

/// Whether a browser would run this content type as code rather than show
/// it as data (AR2 amendment, 2026-08-28).
///
/// Deliberately a short, explicit list rather than a clever rule: the cost
/// of a wrong entry is a payload that renders itself in the hub's origin, so
/// the list errs towards including things. `nosniff` accompanies it, which
/// is what stops a browser executing something that is not on the list.
fn executes_in_a_browser(content_type: &str) -> bool {
    let media = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();

    matches!(
        media.as_str(),
        "text/html"
            | "application/xhtml+xml"
            | "image/svg+xml"
            | "text/xml"
            | "application/xml"
            | "text/xsl"
    ) || media.ends_with("+xml")
        || media.contains("javascript")
        || media.contains("ecmascript")
}

pub(crate) fn insert_str(headers: &mut HeaderMap, name: &'static str, value: &str) {
    if let Ok(parsed) = value.parse() {
        headers.insert(name, parsed);
    }
}
