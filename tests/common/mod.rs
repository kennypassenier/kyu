//! The in-process kit harness (K2, 2026-09-06): the hub assembled exactly as
//! the binary assembles it — `kyu::kit::mount` on a real `chassis::App` —
//! started on a free port with the kit's door in front. Tests that need the
//! dashboard or the token door go through here; tests about the engine and
//! the open API keep `router_with_probes`.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use chassis::{App, AppSpec, Running};
use kyu::config::{Auth, Config};
use kyu::engine::Engine;
use kyu::engine::clock::{Clock, SystemClock};
use kyu::http::{AppState, Limits, assets};
use kyu::kit::{import_app_tokens, mount};
use kyu::store::Store;
use kyu::sweeper::{self, Heartbeat};

pub const TOKEN: &str = "a-login-token-that-is-long-enough";
pub const KEY: &str = "abababababababababababababababababababababababababababababababab";

pub struct KitHub {
    pub addr: SocketAddr,
    pub store: Arc<Store>,
    pub engine: Arc<Engine>,
    /// The admin session cookie, `name=value`, from one login.
    pub cookie: String,
    running: Option<Running>,
    sweeper: tokio::task::JoinHandle<()>,
    _dir: tempfile::TempDir,
}

impl KitHub {
    pub fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    fn http() -> reqwest::Client {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("a client")
    }

    /// A browser with the admin session.
    pub async fn get(&self, path: &str) -> reqwest::Response {
        Self::http()
            .get(self.url(path))
            .header("cookie", &self.cookie)
            .send()
            .await
            .expect("a response")
    }

    /// A browser without a session.
    pub async fn get_anon(&self, path: &str) -> reqwest::Response {
        Self::http()
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
        Self::http()
            .post(self.url(path))
            .header("cookie", &self.cookie)
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
        Self::http()
            .request(method, self.url(path))
            .header("authorization", format!("Bearer {token}"))
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
        let response = self
            .publish_as(TOKEN, topic, "application/json", body.as_bytes().to_vec())
            .await;
        assert_eq!(response.status(), 201, "publish to {topic}");
        body_json(response).await["id"]
            .as_str()
            .expect("an id")
            .to_string()
    }

    pub async fn publish_bytes(&self, topic: &str, content_type: &str, body: Vec<u8>) -> u16 {
        self.publish_as(TOKEN, topic, content_type, body)
            .await
            .status()
            .as_u16()
    }

    pub async fn receive(&self, topic: &str, query: &str) -> reqwest::Response {
        self.bearer(
            reqwest::Method::GET,
            &format!("/t/{topic}/next?{query}"),
            TOKEN,
        )
        .send()
        .await
        .expect("a response")
    }

    pub async fn post_api(&self, path: &str) -> reqwest::Response {
        self.bearer(reqwest::Method::POST, path, TOKEN)
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
        if let Some(running) = self.running.take() {
            running.stop().await;
        }
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

pub async fn spawn_kit() -> KitHub {
    spawn_kit_in(tempfile::tempdir().expect("a temp dir")).await
}

/// The hub on an existing state directory (for the import and restart cases).
pub async fn spawn_kit_in(dir: tempfile::TempDir) -> KitHub {
    let mut env: BTreeMap<String, String> = BTreeMap::new();
    env.insert("KYU_STATE_DIR".into(), dir.path().display().to_string());
    env.insert("KYU_TOKEN".into(), TOKEN.into());
    env.insert("KYU_SECRET_KEY".into(), KEY.into());
    env.insert("KYU_LISTEN".into(), "127.0.0.1:0".into());
    env.insert("KYU_LOG".into(), "warn".into());
    let spec = AppSpec {
        name: "kyu",
        version: env!("CARGO_PKG_VERSION"),
        repository: Some("kennypassenier/kyu"),
        ..Default::default()
    };
    let mut app = App::from_args_with_env(spec, vec!["kyu".into()], env, assets())
        .expect("the kit accepts the test configuration");
    let state_dir = app
        .loaded
        .as_ref()
        .expect("a start loads configuration")
        .state_dir
        .clone();
    // `Config::from_kit` reads the hub's own variables from the process
    // environment; the harness keeps them in the kit's map instead, so the
    // door is set here explicitly.
    let mut config =
        Config::from_kit(&state_dir, app.limits.max_body_bytes as u64).expect("a config");
    config.auth = Auth::parse(Some(TOKEN), Some(KEY)).expect("a protected hub");
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
            recheck_interval: Duration::from_millis(100),
        },
        heartbeat.clone(),
        config.auth.clone(),
    );
    let key = config.auth.key().expect("a protected hub").clone();
    import_app_tokens(&state_dir, &engine, &key, KEY).expect("the import runs");
    mount(&mut app, state.clone(), engine.clone(), heartbeat.clone());
    let notifiers = state.notifiers.clone();
    let sweeper = sweeper::spawn(engine.clone(), heartbeat, move |woken| {
        for (topic, subscription) in woken {
            notifiers.wake(topic, std::slice::from_ref(subscription));
        }
    });
    let running = app.start().await.expect("the kit starts");
    let addr = running.addr;
    // Log in once; every admin request reuses the session cookie.
    let login = KitHub::http()
        .post(format!("http://{addr}/login"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(format!("token={TOKEN}"))
        .send()
        .await
        .expect("a login response");
    assert_eq!(login.status(), 303, "the right token logs in");
    let cookie = header(&login, "set-cookie")
        .expect("a session cookie")
        .split(';')
        .next()
        .expect("name=value")
        .to_string();
    KitHub {
        addr,
        store,
        engine,
        cookie,
        running: Some(running),
        sweeper,
        _dir: dir,
    }
}
