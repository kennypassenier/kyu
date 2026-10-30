//! kyu's own browser assets, embedded in the binary and served from memory.

use axum::extract::Path;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::http::error::ApiError;
use crate::http::page_modules::{MODULE_CONTENT_TYPE, PAGE_MODULES};

/// The hub's own two assets, beyond what the kit serves under `/static`:
/// the page styles and the copy/reveal behaviour of the command snippets.
/// Embedded in the binary, so the image stays a single file (T9).
/// kyu's own browser files, served from memory on `/assets/{file}`: one
/// script and one stylesheet per page beside the shared ones, so each page
/// carries only what it uses (kp-themes 11.0.0 adoption).
const ASSETS: &[(&str, &str, &str)] = &[
    (
        "common.js",
        include_str!("../../../static/common.js"),
        "text/javascript; charset=utf-8",
    ),
    (
        "topics.js",
        include_str!("../../../static/topics.js"),
        "text/javascript; charset=utf-8",
    ),
    (
        "topic.js",
        include_str!("../../../static/topic.js"),
        "text/javascript; charset=utf-8",
    ),
    (
        "subscription.js",
        include_str!("../../../static/subscription.js"),
        "text/javascript; charset=utf-8",
    ),
    (
        "kyu.css",
        include_str!("../../../static/kyu.css"),
        "text/css; charset=utf-8",
    ),
    (
        "topics.css",
        include_str!("../../../static/topics.css"),
        "text/css; charset=utf-8",
    ),
    (
        "topic-layout.css",
        include_str!("../../../static/topic-layout.css"),
        "text/css; charset=utf-8",
    ),
    (
        "topic-send.css",
        include_str!("../../../static/topic-send.css"),
        "text/css; charset=utf-8",
    ),
    (
        "topic-subs.css",
        include_str!("../../../static/topic-subs.css"),
        "text/css; charset=utf-8",
    ),
    (
        "topic-dead.css",
        include_str!("../../../static/topic-dead.css"),
        "text/css; charset=utf-8",
    ),
    (
        "topic-recent.css",
        include_str!("../../../static/topic-recent.css"),
        "text/css; charset=utf-8",
    ),
    (
        "topic-publish.css",
        include_str!("../../../static/topic-publish.css"),
        "text/css; charset=utf-8",
    ),
    (
        "topic-actions.css",
        include_str!("../../../static/topic-actions.css"),
        "text/css; charset=utf-8",
    ),
    (
        "subscription.css",
        include_str!("../../../static/subscription.css"),
        "text/css; charset=utf-8",
    ),
];

/// The cache-busting fingerprint every page appends to its asset URLs.
///
/// Without it the year-long cache header below would serve yesterday's
/// JavaScript after an upgrade — which is exactly how a fixed bug appears
/// not to be fixed. The assets only ever change when the binary does, so
/// hashing their contents at first use is both sufficient and free.
pub static ASSET_VERSION: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    // FNV-1a: not cryptographic, and does not need to be — this answers
    // "did these bytes change", nothing more (T6: no crate for six lines).
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let parts = PAGE_MODULES.iter().map(|(_, body)| body);
    for byte in ASSETS
        .iter()
        .map(|(_, body, _)| body)
        .chain(parts)
        .flat_map(|body| body.bytes())
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
});

/// `GET /assets/{file}` — served from memory, never from disk.
///
/// An explicit match rather than a path join: a lookup that builds a path
/// from user input is how a static handler turns into a file-disclosure
/// bug, and there are exactly two files.
pub async fn kyu_asset(Path(file): Path<String>, headers: HeaderMap) -> Response {
    if let Some(&(_, body, content_type)) = ASSETS.iter().find(|(name, _, _)| *name == file) {
        return (
            [
                (header::CONTENT_TYPE, content_type),
                // Safe because every reference carries ?v=<ASSET_VERSION>.
                (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
            ],
            body,
        )
            .into_response();
    }
    if let Some(&(_, body)) = PAGE_MODULES.iter().find(|(name, _)| *name == file) {
        return page_module(&headers, body);
    }
    ApiError::new(
        StatusCode::NOT_FOUND,
        format!("no asset named {file:?}"),
        "the hub serves its own page scripts and stylesheets here; everything else is the kit's, under /static",
    )
    .into_response()
}

/// A part module of a page script. Its import carries no `?v=`, so it must
/// not be cached as immutable: it revalidates by `ETag` (the fingerprint of
/// all assets) and answers 304 when the browser already has this build's copy.
fn page_module(request: &HeaderMap, body: &'static str) -> Response {
    let etag = format!("\"{}\"", *ASSET_VERSION);
    let cache = (header::CACHE_CONTROL, "no-cache".to_string());
    let tag = (header::ETAG, etag.clone());
    let unchanged = request
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|sent| sent.split(',').any(|candidate| candidate.trim() == etag));
    if unchanged {
        return (StatusCode::NOT_MODIFIED, [cache, tag]).into_response();
    }
    (
        [
            (header::CONTENT_TYPE, MODULE_CONTENT_TYPE.to_string()),
            cache,
            tag,
        ],
        body,
    )
        .into_response()
}
