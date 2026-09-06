//! [Phase 7] The security review's findings, each pinned by a test that
//! fails against the code as it was.
//!
//! All three come from the same root: a payload or its metadata is
//! attacker-controlled (any LAN device may publish), and it later reaches
//! somewhere that treats it as trusted — a clipboard, a browser origin, or
//! a state-changing request.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use kyu::engine::Engine;
use kyu::engine::clock::{Clock, SystemClock};
use kyu::http::{AppState, Limits, router_with_probes};
use kyu::store::Store;
use kyu::sweeper::Heartbeat;

struct Hub {
    addr: SocketAddr,
}

impl Hub {
    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }
}

async fn spawn() -> (Hub, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let store = Arc::new(Store::open(dir.path()).expect("a store"));
    let engine = Arc::new(Engine::new(store, Arc::new(SystemClock)));
    let state = AppState::new(
        engine,
        Limits {
            max_body_bytes: 65_536,
            default_wait_s: 1,
            max_wait_s: 300,
            recheck_interval: Duration::from_millis(100),
        },
        Heartbeat::starting_at(SystemClock.now_ms()),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a port");
    let addr = listener.local_addr().expect("an address");
    tokio::spawn(async move {
        let _ = axum::serve(listener, router_with_probes(state)).await;
    });
    (Hub { addr }, dir)
}

async fn publish_with_type(hub: &Hub, topic: &str, content_type: &str, body: &str) -> u16 {
    reqwest::Client::new()
        .post(hub.url(&format!("/t/{topic}")))
        .header("content-type", content_type)
        .body(body.to_string())
        .send()
        .await
        .expect("a response")
        .status()
        .as_u16()
}

// Findings 1 and 2 moved on 2026-09-06 (step 2 of the chassis migration):
// the snippet-injection test lives in `k2_dashboard.rs` against the kit,
// and cross-origin form posts are refused by the kit (proven there too).

// ─── Finding 3 · a payload rendering itself in the hub's origin ─────────────

#[tokio::test]
async fn p7_sec3_a_payload_cannot_render_itself_in_the_hubs_origin() {
    let (hub, _dir) = spawn().await;
    publish_with_type(&hub, "notify.kenny", "application/json", "{}").await;
    let _ = reqwest::get(hub.url("/t/notify.kenny/next?as=probe&wait=0")).await;

    // Published as HTML, this would otherwise execute in the hub's own origin
    // for anyone who opens the receive URL — including an iframe on a hostile
    // page.
    publish_with_type(
        &hub,
        "notify.kenny",
        "text/html",
        "<script>alert('pwned')</script>",
    )
    .await;

    let response = reqwest::get(hub.url("/t/notify.kenny/next?as=probe&wait=0"))
        .await
        .expect("a response");
    assert_eq!(response.status(), 200);

    let disposition = response
        .headers()
        .get("content-disposition")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let nosniff = response
        .headers()
        .get("x-content-type-options")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();

    assert!(
        disposition.contains("attachment"),
        "the raw body must be handed over as a download, never rendered in \
         the hub's origin"
    );
    assert_eq!(nosniff, "nosniff", "and never sniffed into something worse");
    assert_eq!(
        content_type, "text/html",
        "while AR2's promise holds: the stored content type is still reported"
    );
    assert_eq!(
        response.text().await.expect("a body"),
        "<script>alert('pwned')</script>",
        "and the bytes are still returned verbatim"
    );
}

#[tokio::test]
async fn p7_sec3_an_ordinary_payload_still_opens_in_a_browser() {
    // The AR2 mini-round chose protection that does not make the API
    // unbrowsable: only types a browser would execute are forced to
    // download. JSON, text and images stay openable in a tab.
    let (hub, _dir) = spawn().await;
    publish_with_type(&hub, "notify.kenny", "application/json", "{}").await;
    let _ = reqwest::get(hub.url("/t/notify.kenny/next?as=probe&wait=0")).await;

    for (content_type, body) in [
        ("application/json", r#"{"title":"Backup klaar"}"#),
        ("text/plain", "de was is klaar"),
    ] {
        publish_with_type(&hub, "notify.kenny", content_type, body).await;
        let response = reqwest::get(hub.url("/t/notify.kenny/next?as=probe&wait=0"))
            .await
            .expect("a response");
        assert_eq!(response.status(), 200);

        assert!(
            response.headers().get("content-disposition").is_none(),
            "{content_type} is data, not code — it must still open in a tab"
        );
        assert_eq!(
            response
                .headers()
                .get("x-content-type-options")
                .and_then(|v| v.to_str().ok()),
            Some("nosniff"),
            "but nosniff still applies, so a browser cannot decide it is HTML"
        );
    }
}

#[tokio::test]
async fn p7_sec3_every_executable_type_is_forced_to_download() {
    let (hub, _dir) = spawn().await;
    publish_with_type(&hub, "notify.kenny", "application/json", "{}").await;
    let _ = reqwest::get(hub.url("/t/notify.kenny/next?as=probe&wait=0")).await;

    // The list the mini-round settled on, each one checked rather than
    // assumed — a wrong entry here is a payload running in the hub's origin.
    for content_type in [
        "text/html",
        "text/html; charset=utf-8",
        "TEXT/HTML",
        "application/xhtml+xml",
        "image/svg+xml",
        "application/xml",
        "text/xml",
        "application/rss+xml",
        "text/javascript",
        "application/ecmascript",
    ] {
        publish_with_type(&hub, "notify.kenny", content_type, "<x/>").await;
        let response = reqwest::get(hub.url("/t/notify.kenny/next?as=probe&wait=0"))
            .await
            .expect("a response");
        assert_eq!(response.status(), 200, "{content_type}");
        assert!(
            response
                .headers()
                .get("content-disposition")
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .contains("attachment"),
            "{content_type} would execute in a browser and must be downloaded"
        );
    }
}
