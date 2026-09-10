//! The in-process kit harness (K2, 2026-09-06; rebuilt on the kit's own
//! `chassis::testing::TestApp` for K-harness, chassis-rs 1.8.0; simplified
//! again on chassis-rs 2.0.0 once CF-12 made `TestApp::token()`/`::login()`
//! track an `extra_env`-overridden secret instead of the one generated at
//! launch — kyu's own manual login workaround is gone): the hub assembled
//! exactly as the binary assembles it — `kyu::kit::mount` on a real
//! `chassis::App` — started on a free port with the kit's door in front.
//! Tests that need the dashboard or the token door go through here; tests
//! about the engine and the open API keep `router_with_probes`.
//!
//! `TestApp` owns the generic half (spawn, admin login, the session cookie,
//! bearer requests, `issue_client`) — kyu no longer hand-rolls any of it.
//! What stays kyu's own: assembling the engine/store/config the way
//! `main.rs` does, and the three verbs (`publish`/`receive`/`bootstrap`),
//! which the kit does not know about.
#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use chassis::testing::TestApp;
use chassis::{App, AppSpec};
use kyu::config::{Auth, Config};
use kyu::engine::Engine;
use kyu::engine::clock::{Clock, SystemClock};
use kyu::http::{AppState, Limits, assets};
use kyu::kit::{import_app_tokens, mount};
use kyu::store::Store;
use kyu::sweeper::{self, Heartbeat};

pub struct KitHub {
    app: TestApp,
    pub addr: SocketAddr,
    pub store: Arc<Store>,
    pub engine: Arc<Engine>,
    sweeper: tokio::task::JoinHandle<()>,
    /// `spawn_kit_in`'s caller-supplied state directory, kept alive as long
    /// as the app runs (`KYU_STATE_DIR` points into it). `TestApp` owns and
    /// cleans up its own separate tempdir regardless — this one is `None`
    /// for the common `spawn_kit` case, which never overrides the state
    /// directory.
    _dir: Option<tempfile::TempDir>,
}

impl KitHub {
    pub fn url(&self, path: &str) -> String {
        self.app.url(path)
    }

    /// A browser with the admin session.
    pub async fn get(&self, path: &str) -> reqwest::Response {
        self.app
            .request(reqwest::Method::GET, path)
            .send()
            .await
            .expect("a response")
    }

    /// A browser without a session.
    pub async fn get_anon(&self, path: &str) -> reqwest::Response {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("a client")
            .get(self.url(path))
            .send()
            .await
            .expect("a response")
    }

    /// A page that must render for the admin.
    pub async fn page(&self, path: &str) -> String {
        let response = self.get(path).await;
        assert_eq!(response.status(), 200, "{path} must render");
        response.text().await.expect("a body")
    }

    /// One of the dashboard's own forms, posted by the admin's browser.
    pub async fn form(&self, path: &str, body: &str) -> reqwest::Response {
        self.app
            .request(reqwest::Method::POST, path)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(body.to_string())
            .send()
            .await
            .expect("a response")
    }

    /// A script with a bearer token.
    pub fn bearer(
        &self,
        method: reqwest::Method,
        path: &str,
        token: &str,
    ) -> reqwest::RequestBuilder {
        self.app.bearer(method, path, token)
    }

    /// The admin's own token, for scripts that send it as a bearer (it
    /// doubles as the hub's own login token — K2 step 2).
    pub fn token(&self) -> &str {
        self.app.token()
    }

    /// The admin session cookie (`name=value`), for a request built by hand
    /// instead of through `get`/`form` (a cross-origin CSRF probe, say).
    pub fn session_cookie(&self) -> &str {
        self.app.session_cookie().expect("login ran at spawn")
    }

    pub async fn publish_as(
        &self,
        token: &str,
        topic: &str,
        content_type: &str,
        body: Vec<u8>,
    ) -> reqwest::Response {
        self.bearer(reqwest::Method::POST, &format!("/t/{topic}"), token)
            .header("content-type", content_type)
            .body(body)
            .send()
            .await
            .expect("a response")
    }

    /// Publish with the login token; returns the message id.
    pub async fn publish(&self, topic: &str, body: &str) -> String {
        let token = self.token().to_string();
        let response = self
            .publish_as(&token, topic, "application/json", body.as_bytes().to_vec())
            .await;
        assert_eq!(response.status(), 201, "publish to {topic}");
        body_json(response).await["id"]
            .as_str()
            .expect("an id")
            .to_string()
    }

    pub async fn publish_bytes(&self, topic: &str, content_type: &str, body: Vec<u8>) -> u16 {
        let token = self.token().to_string();
        self.publish_as(&token, topic, content_type, body)
            .await
            .status()
            .as_u16()
    }

    pub async fn receive(&self, topic: &str, query: &str) -> reqwest::Response {
        let token = self.token().to_string();
        self.bearer(
            reqwest::Method::GET,
            &format!("/t/{topic}/next?{query}"),
            &token,
        )
        .send()
        .await
        .expect("a response")
    }

    pub async fn post_api(&self, path: &str) -> reqwest::Response {
        let token = self.token().to_string();
        self.bearer(reqwest::Method::POST, path, &token)
            .send()
            .await
            .expect("a response")
    }

    /// A subscription exists once it has polled; a fresh one starts at the
    /// end of the topic, so the first poll answers 204.
    pub async fn bootstrap(&self, topic: &str, subscription: &str) {
        self.publish(topic, r#"{"bootstrap":true}"#).await;
        assert_eq!(
            self.receive(topic, &format!("as={subscription}&wait=0"))
                .await
                .status(),
            204
        );
    }

    /// Two subscriptions on one topic, drained of the bootstrap leftover that
    /// the second bootstrap fans out to the first.
    pub async fn bootstrap_two_clean(&self, topic: &str, first: &str, second: &str) {
        self.bootstrap(topic, first).await;
        self.bootstrap(topic, second).await;
        let leaked = self.receive(topic, &format!("as={first}&wait=0")).await;
        assert_eq!(
            leaked.status(),
            200,
            "the second bootstrap leaks one message to {first}"
        );
        let leaked_id = header(&leaked, "kyu-id").expect("an id");
        let ack = self
            .post_api(&format!("/t/{topic}/ack/{leaked_id}?as={first}"))
            .await;
        assert_eq!(ack.status(), 200);
    }

    pub async fn shutdown(mut self) {
        self.sweeper.abort();
        let _ = (&mut self.sweeper).await;
        self.app.shutdown().await;
    }
}

pub async fn body_json(response: reqwest::Response) -> serde_json::Value {
    let text = response.text().await.expect("a body");
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("expected JSON, got {text:?}: {e}"))
}

pub fn header(response: &reqwest::Response, name: &str) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

/// Turns rendered HTML back into the text a browser would show.
pub fn unescape(html: &str) -> String {
    html.replace("&#x2f;", "/")
        .replace("&#x27;", "'")
        .replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn spec() -> AppSpec {
    AppSpec {
        name: "kyu",
        version: env!("CARGO_PKG_VERSION"),
        repository: Some("kennypassenier/kyu"),
        ..Default::default()
    }
}

/// What `configure_hub` builds, smuggled out of an `FnOnce(&mut App)` that
/// cannot return a value directly.
type Configured = (Arc<Store>, Arc<Engine>, tokio::task::JoinHandle<()>);

/// Everything `spawn_kit`/`spawn_kit_in` share: build the engine/store the
/// way `main.rs` does, from the token and secret key the kit itself just
/// resolved (`TestApp` generates them; a restart passes them back in
/// through `extra_env` so the state directory is opened with the same
/// key it was written with), then `mount` onto the kit's `App`. Runs
/// inside `TestApp`'s `configure` closure, before the kit starts — the
/// `Mutex` is only ever touched once, synchronously, from this one
/// closure; it exists to smuggle the store/engine/sweeper out of that
/// closure.
fn configure_hub(app: &mut App, out: Arc<Mutex<Option<Configured>>>) {
    let loaded = app.loaded.as_ref().expect("a start loads configuration");
    let state_dir = loaded.state_dir.clone();
    let token = loaded
        .get("token")
        .expect("the kit resolves an admin token")
        .to_string();
    let secret_key = loaded
        .get("secret_key")
        .expect("the kit resolves a secret key")
        .to_string();
    // `Config::from_kit` reads the hub's own variables from the process
    // environment; the harness keeps them in the kit's own map instead, so
    // the door is set here explicitly, from what the kit itself resolved.
    let mut config =
        Config::from_kit(&state_dir, app.limits.max_body_bytes as u64).expect("a config");
    config.auth = Auth::parse(Some(&token), Some(&secret_key)).expect("a protected hub");
    let store = Arc::new(Store::open(&config.data_dir).expect("a store"));
    let heartbeat = Heartbeat::starting_at(SystemClock.now_ms());
    let engine = Arc::new(Engine::with_defaults(
        store.clone(),
        Arc::new(SystemClock),
        config.defaults,
    ));
    let state = AppState::with_auth(
        engine.clone(),
        Limits {
            max_body_bytes: 65_536,
            default_wait_s: 1,
            max_wait_s: 300,
            recheck_interval: std::time::Duration::from_millis(100),
        },
        heartbeat.clone(),
        config.auth.clone(),
    );
    let key = config.auth.key().expect("a protected hub").clone();
    import_app_tokens(&state_dir, &engine, &key, &secret_key).expect("the import runs");
    mount(app, state.clone(), engine.clone(), heartbeat.clone());
    let notifiers = state.notifiers.clone();
    let sweeper = sweeper::spawn(engine.clone(), heartbeat, move |woken| {
        for (topic, subscription) in woken {
            notifiers.wake(topic, std::slice::from_ref(subscription));
        }
    });
    *out.lock().expect("configure runs once, uncontended") = Some((store, engine, sweeper));
}

pub async fn spawn_kit() -> KitHub {
    let out = Arc::new(Mutex::new(None));
    let out_for_closure = out.clone();
    let mut app = TestApp::start_with(spec(), assets(), move |app| {
        configure_hub(app, out_for_closure);
    })
    .await;
    let addr = app.addr();
    app.login().await;
    let (store, engine, sweeper) = out
        .lock()
        .expect("configure ran synchronously before start")
        .take()
        .expect("configure_hub always fills the slot");
    KitHub {
        app,
        addr,
        store,
        engine,
        sweeper,
        _dir: None,
    }
}

/// The hub on an existing state directory (for the import and restart
/// cases): the token and secret key are fixed rather than generated, so a
/// file pre-written into `dir` under a known key opens cleanly once the
/// kit starts. `TestApp::login()` works against this overridden token since
/// chassis-rs 2.0.0 (CF-12) — before that fix it read the value `TestApp`
/// itself generated at launch, which `extra_env` had already made stale.
pub const TOKEN: &str = "a-login-token-that-is-long-enough";
pub const KEY: &str = "abababababababababababababababababababababababababababababababab";

pub async fn spawn_kit_in(dir: tempfile::TempDir) -> KitHub {
    let out = Arc::new(Mutex::new(None));
    let out_for_closure = out.clone();
    let state_dir = dir.path().display().to_string();
    let mut app = TestApp::start_with_env(
        spec(),
        assets(),
        &[
            ("KYU_STATE_DIR", state_dir.as_str()),
            ("KYU_TOKEN", TOKEN),
            ("KYU_SECRET_KEY", KEY),
        ],
        move |app| configure_hub(app, out_for_closure),
    )
    .await;
    let addr = app.addr();
    app.login().await;
    let (store, engine, sweeper) = out
        .lock()
        .expect("configure ran synchronously before start")
        .take()
        .expect("configure_hub always fills the slot");
    KitHub {
        app,
        addr,
        store,
        engine,
        sweeper,
        _dir: Some(dir),
    }
}
