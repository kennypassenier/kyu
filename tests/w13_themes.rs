//! [W13] The house themes and their picker.
//!
//! kyu consumes `@kp-soft/themes` v1.0.0 — the shared package JobTracker,
//! almanac and kp-soft use — so the same eleven themes look the same
//! everywhere and a choice made in one app behaves the same in the next.
//!
//! Since v1.0.0 the package ships the framework-free channel kyu needed, so
//! the picker's behaviour is no longer kyu's to reimplement: the module is
//! vendored verbatim and attaches to markup kyu's server writes. What these
//! tests pin is the seam between the two — the contract attributes, and the
//! fact that the theme list kyu renders and the list the package generated
//! still agree. That agreement is the thing a vendored copy loses silently.

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

/// The eleven themes, name AND label, read out of the package's own
/// generated registry rather than written here: a literal list in this test
/// would be one more copy to go stale, which is the failure the package
/// removed in v1.0.0.
fn themes_from_registry(registry: &str) -> Vec<(String, String)> {
    registry
        .split("{ name: '")
        .skip(1)
        .filter_map(|entry| {
            let name = entry.split('\'').next()?.to_string();
            let label = entry
                .split("label: '")
                .nth(1)?
                .split('\'')
                .next()?
                .to_string();
            Some((name, label))
        })
        .collect()
}

#[tokio::test]
async fn w13_the_picker_offers_exactly_the_themes_the_package_defines() {
    // The seam: kyu renders the menu server-side (a menu built by
    // JavaScript is an empty box on first paint), so its list and the
    // package's generated one are two things that must agree. Comparing
    // them here means a vendored update that adds or renames a theme
    // cannot leave the picker quietly behind.
    let (hub, _dir) = spawn().await;
    let page = hub.text("/").await;
    let registry = hub.text("/static/theme-registry.js").await;

    let expected = themes_from_registry(&registry);
    assert_eq!(
        expected.len(),
        11,
        "the package should define eleven themes; found {expected:?}"
    );

    for (name, label) in &expected {
        assert!(
            page.contains(&format!("data-kp-theme=\"{name}\"")),
            "the picker must offer {name}"
        );
        assert!(
            page.contains(&format!("class=\"kp-swatch\" data-theme=\"{name}\"")),
            "and {name} must get a swatch that wears the theme itself, \
             rather than a colour copied out of the stylesheet"
        );
        // kyu holds the labels in Rust because it renders the menu
        // server-side, so they are the one thing here that CAN drift from
        // the package. Comparing them is what makes that copy safe: if
        // kp-themes renames a theme, this fails instead of kyu quietly
        // showing the old word. (Gap found by the almanac session, which
        // had the same one.)
        assert!(
            page.contains(&format!("data-theme=\"{name}\"></span>{label}</button>")),
            "{name} must be labelled {label:?}, exactly as the package calls it"
        );
    }
    assert_eq!(
        page.matches("data-kp-theme=\"").count(),
        expected.len(),
        "and it must offer no themes the package does not define"
    );
}

#[tokio::test]
async fn w13_the_markup_carries_the_packages_contract_attributes() {
    // Every one of these is a contract value of @kp-soft/themes (their
    // TH26): the vendored module attaches to them. Renaming one here would
    // leave a picker that renders perfectly and does nothing at all.
    let (hub, _dir) = spawn().await;
    let page = hub.text("/").await;

    for attribute in [
        "data-kp-theme-picker",
        "data-kp-theme-status",
        "class=\"kp-swatch\"",
        "class=\"kp-menu\"",
    ] {
        assert!(
            page.contains(attribute),
            "the markup must carry {attribute}"
        );
    }
    assert!(
        page.contains("type=\"module\" src=\"/static/theme-picker.js"),
        "and load the package's picker as a module, which is how it attaches"
    );
}

#[tokio::test]
async fn w13_the_storage_contract_matches_the_shared_package() {
    // What must never drift silently: JobTracker, almanac, kp-soft and kyu
    // all write the SAME key with the SAME values, so behind one hostname a
    // choice follows you between apps. Read from the vendored registry, so
    // this asserts what kyu actually serves rather than what it intended.
    let (hub, _dir) = spawn().await;
    let registry = hub.text("/static/theme-registry.js").await;

    assert!(
        registry.contains("STORAGE_KEY = 'theme'"),
        "the localStorage key is 'theme'"
    );
    assert!(
        registry.contains("DEFAULT_THEME = 'formal'"),
        "and the default theme is formal"
    );

    let page = hub.text("/").await;
    assert!(
        page.contains("localStorage.getItem('theme')"),
        "the no-flash snippet in <head> reads that same key before first paint"
    );
}

#[tokio::test]
async fn w13_the_module_chain_is_reachable_and_nothing_else_is() {
    // theme-picker.js imports ./theme-core.js, which imports
    // ./theme-registry.js. Served flat, those relative specifiers resolve
    // under /static — so a missing one breaks the picker with nothing on the
    // page to say why.
    let (hub, _dir) = spawn().await;

    for (path, expected) in [
        ("/static/themes.css", "text/css"),
        ("/static/components.css", "text/css"),
        ("/static/theme-bridge.css", "text/css"),
        ("/static/theme-core.js", "text/javascript"),
        ("/static/theme-picker.js", "text/javascript"),
        ("/static/theme-registry.js", "text/javascript"),
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

    // The allowlist is the whole defence of this handler; adding five assets
    // must not have turned it into a path join.
    assert_eq!(
        hub.get("/static/themes.css/../../etc/passwd")
            .await
            .status(),
        404,
        "the static handler still serves only what it names"
    );
    assert_eq!(
        hub.get("/static/theme.js").await.status(),
        404,
        "and the hand-written picker of 2.2.0 is gone, not merely unused"
    );
}

#[tokio::test]
async fn w13_every_theme_the_picker_offers_exists_in_the_stylesheet() {
    // A theme could be selectable and do nothing at all if the registry and
    // the stylesheet disagreed — they are separate files in the package, and
    // kyu vendored both.
    let (hub, _dir) = spawn().await;
    let css = hub.text("/static/themes.css").await;
    let registry = hub.text("/static/theme-registry.js").await;

    for (name, _) in themes_from_registry(&registry) {
        assert!(
            css.contains(&format!("data-theme='{name}'"))
                || css.contains(&format!("data-theme=\"{name}\"")),
            "themes.css must define {name}; the picker offers it"
        );
    }
}
