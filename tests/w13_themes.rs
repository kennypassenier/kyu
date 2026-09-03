//! [W13] The house themes and their picker.
//!
//! kyu uses `@kp-soft/themes` v0.1.1 — the shared package JobTracker and
//! kp-soft use — so the same seven themes look the same everywhere and a
//! choice made in one place feels like the same mechanism in the next.
//!
//! kyu cannot USE the package: it has no npm and no build step, and the
//! package ships a React hook and a JSX component. What it can do, and what
//! these tests pin, is honour the same contract: the same seven themes, the
//! same storage key, the same default. That contract is shared across
//! projects, so it must not be possible to rename it here by accident.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use kyu::engine::Engine;
use kyu::engine::clock::{Clock, SystemClock};
use kyu::http::{AppState, Limits, router};
use kyu::store::Store;
use kyu::sweeper::Heartbeat;

struct Hub {
    addr: SocketAddr,
}

impl Hub {
    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    async fn get(&self, path: &str) -> reqwest::Response {
        reqwest::get(self.url(path)).await.expect("a response")
    }

    async fn text(&self, path: &str) -> String {
        self.get(path).await.text().await.expect("a body")
    }
}

async fn spawn() -> (Hub, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let store = Arc::new(Store::open(dir.path()).expect("a store"));
    let engine = Arc::new(Engine::new(store, Arc::new(SystemClock)));
    let state = AppState::new(
        engine,
        Limits {
            max_body_bytes: 1024 * 1024,
            default_wait_s: 1,
            max_wait_s: 300,
            recheck_interval: Duration::from_millis(200),
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

const THEMES: [(&str, &str, bool); 7] = [
    ("formal", "Formeel", false),
    ("light", "Licht", false),
    ("dark", "Donker", true),
    ("cyberpunk", "Cyberpunk", true),
    ("pastel", "Pastel", false),
    ("terminal", "Terminal", true),
    ("topo", "Topografisch", false),
];

#[tokio::test]
async fn w13_the_picker_offers_all_seven_themes_with_their_swatches() {
    let (hub, _dir) = spawn().await;
    let page = hub.text("/").await;

    for (name, label, dark) in THEMES {
        assert!(
            page.contains(&format!("data-theme=\"{name}\"")),
            "the picker must offer {name}"
        );
        assert!(page.contains(label), "and name it in Dutch as {label:?}");
        assert!(
            page.contains(&format!(
                "data-theme=\"{name}\" data-dark=\"{}\"",
                if dark { "true" } else { "false" }
            )),
            "{name} must be marked dark={dark}, which is what drives the \
             .dark class and Bootstrap's own dark mode"
        );
    }
    assert!(
        page.matches("theme-picker__swatch").count() == 7,
        "every theme gets a swatch previewing it without activating it"
    );
    assert!(
        page.contains("linear-gradient(135deg,"),
        "and the swatch is the package's own two-tone gradient"
    );
}

#[tokio::test]
async fn w13_the_storage_contract_matches_the_shared_package() {
    // The one thing that must never drift silently: JobTracker, kp-soft and
    // kyu all write the SAME key with the SAME values, so behind one
    // hostname the choice follows you between apps. A rename here would be
    // invisible until someone noticed their theme "not sticking".
    let (hub, _dir) = spawn().await;
    let script = hub.text("/static/theme.js").await;

    assert!(
        script.contains("'theme'"),
        "the localStorage key is 'theme', exactly as the package uses it"
    );
    assert!(
        script.contains("'formal'"),
        "and the default theme is formal, as in the package"
    );
    assert!(
        script.contains("classList.toggle('dark'"),
        "dark themes also carry the .dark class the package sets"
    );

    // The head script applies the stored theme before first paint.
    let page = hub.text("/").await;
    assert!(
        page.contains("localStorage.getItem('theme')"),
        "the no-flash script reads the same key"
    );
}

#[tokio::test]
async fn w13_the_theme_assets_are_served_and_nothing_else_is() {
    let (hub, _dir) = spawn().await;

    for (path, expected) in [
        ("/static/themes.css", "text/css"),
        ("/static/theme-bridge.css", "text/css"),
        ("/static/theme.js", "text/javascript"),
    ] {
        let response = hub.get(path).await;
        assert_eq!(response.status(), 200, "{path} must be served");
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        assert!(
            content_type.starts_with(expected),
            "{path} must be {expected}, got {content_type}"
        );
    }

    // The allowlist is the whole defence of this handler; adding assets must
    // not have turned it into a path join.
    assert_eq!(
        hub.get("/static/themes.css/../../etc/passwd")
            .await
            .status(),
        404,
        "the static handler still serves only what it names"
    );
}

#[tokio::test]
async fn w13_every_theme_block_the_picker_offers_exists_in_the_stylesheet() {
    // The picker is rendered from Rust, the tokens come from a vendored copy
    // of someone else's file. If those two ever disagree, a theme would be
    // selectable and do nothing at all.
    let (hub, _dir) = spawn().await;
    let css = hub.text("/static/themes.css").await;

    for (name, _, _) in THEMES {
        assert!(
            css.contains(&format!("[data-theme='{name}']")),
            "themes.css must define {name}; the picker offers it"
        );
    }
}
