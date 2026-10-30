//! The HTTP side of publish damping: reading `Kyu-Dedup-Key` and
//! `Kyu-Dedup-Window`, and answering a suppressed duplicate.

use axum::Json;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::engine::dedup::{self, Dedup};

use super::error::ApiError;

pub const HEADER_KEY: &str = "kyu-dedup-key";
pub const HEADER_WINDOW: &str = "kyu-dedup-window";
/// Set to `true` on the answer to a suppressed duplicate.
pub const HEADER_DUPLICATE: &str = "kyu-duplicate";

/// `None` without a key header, so a plain publish behaves exactly as
/// before. A window without a key is refused rather than ignored.
pub fn from_headers(headers: &HeaderMap) -> Result<Option<Dedup>, ApiError> {
    let key = headers.get(HEADER_KEY);
    let window = headers.get(HEADER_WINDOW);
    let Some(key) = key else {
        if window.is_some() {
            return Err(bad_request(
                "Kyu-Dedup-Window needs a Kyu-Dedup-Key".to_string(),
                "add Kyu-Dedup-Key: <key>, or drop the window header.",
            ));
        }
        return Ok(None);
    };
    let key = key.to_str().map_err(|_| {
        bad_request(
            "Kyu-Dedup-Key is not printable ASCII".to_string(),
            "use 1 to 200 printable ASCII characters.",
        )
    })?;
    let window = match window {
        None => None,
        Some(raw) => {
            let raw = raw.to_str().map_err(|_| {
                bad_request(
                    "Kyu-Dedup-Window is not readable text".to_string(),
                    "use milliseconds or a unit, such as 20h or 30m.",
                )
            })?;
            Some(dedup::parse_window(raw).map_err(|why| {
                bad_request(
                    format!("Kyu-Dedup-Window: {why}"),
                    "use whole milliseconds or a number with ms, s, m, h or d, at most 7d.",
                )
            })?)
        }
    };
    Dedup::new(key, window).map(Some).map_err(|why| {
        bad_request(
            format!("Kyu-Dedup-Key / Kyu-Dedup-Window: {why}"),
            "key: 1 to 200 printable ASCII bytes; window: 1 ms to 7 days (default 24h).",
        )
    })
}

fn bad_request(error: String, remedy: &str) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, error, remedy)
}

/// 200 (not 201: nothing was created) with the original message's id.
pub fn duplicate_response(original_id: &str) -> Response {
    let mut response = (
        StatusCode::OK,
        Json(json!({ "id": original_id, "duplicate": true })),
    )
        .into_response();
    response
        .headers_mut()
        .insert(HEADER_DUPLICATE, HeaderValue::from_static("true"));
    response
}
