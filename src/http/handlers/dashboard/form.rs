//! Helpers for dashboard form posts: form-value decoding and the two ways a
//! post ends (JSON for the page script, redirect for a plain form).

use axum::http::{HeaderMap, header};
use axum::response::{IntoResponse, Redirect, Response};
use serde_json::json;

pub(crate) fn urldecode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        out.push(byte);
                        index += 3;
                    }
                    Err(_) => {
                        out.push(bytes[index]);
                        index += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Whether the page's script asked for JSON instead of the no-script
/// redirect (`Accept: application/json`).
pub(crate) fn wants_json(headers: &HeaderMap) -> bool {
    headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|accept| accept.contains("application/json"))
}

/// How a dashboard form post ends: `{ "ok": true }` for a script, the
/// redirect back to the page for a browser form.
pub(crate) fn done(headers: &HeaderMap, location: &str) -> Response {
    if wants_json(headers) {
        axum::Json(json!({ "ok": true })).into_response()
    } else {
        Redirect::to(location).into_response()
    }
}
