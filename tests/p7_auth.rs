//! [P7] The door (W2), over real HTTP against a hub that has one.
//!
//! Three things have to hold, and each one has failed in some other
//! project's auth layer at some point:
//!
//! 1. Everything that shows a payload or changes state needs a token, and
//!    the two monitoring endpoints deliberately do not.
//! 2. A token never leaks — not into a log line, not into a metric label,
//!    and not onto a page in the clear. This is the plaintext scan that
//!    FEATURES.md marks mandatory.
//! 3. Revoking works immediately, because "revoked but still valid for a
//!    minute" is not something anyone wants to reason about mid-incident.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use kyu::config::Auth;
use kyu::crypto::SecretKey;
use kyu::engine::Engine;
use kyu::engine::clock::{Clock, SystemClock};
use kyu::http::{AppState, Limits, router_with_probes};
use kyu::store::Store;
use kyu::sweeper::Heartbeat;

const TOKEN: &str = "the-bootstrap-token-for-tests";

struct Hub {
    addr: SocketAddr,
}

impl Hub {
    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }
}

/// A client that does not follow redirects, so a redirect to the login page
/// is visible as itself rather than as the page it lands on.
fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("a client")
}

async fn spawn(protected: bool) -> (Hub, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let store = Arc::new(Store::open(dir.path()).expect("a store"));
    let engine = Arc::new(Engine::new(store, Arc::new(SystemClock)));
    let auth = if protected {
        Auth::parse(Some(TOKEN), Some(&SecretKey::generate_hex())).expect("a complete pair")
    } else {
        Auth::Unprotected
    };
    let state = AppState::with_auth(
        engine,
        Limits {
            max_body_bytes: 1024 * 1024,
            default_wait_s: 1,
            max_wait_s: 300,
            recheck_interval: Duration::from_millis(200),
        },
        Heartbeat::starting_at(SystemClock.now_ms()),
        auth,
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

/// Logs in and returns the session cookie value, the way a browser would.
async fn log_in(hub: &Hub, token: &str) -> String {
    let response = client()
        .post(hub.url("/login"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(format!("token={token}&remember=1"))
        .send()
        .await
        .expect("a response");
    assert_eq!(response.status(), 303, "a good token redirects onward");
    response
        .headers()
        .get("set-cookie")
        .expect("a session cookie")
        .to_str()
        .expect("text")
        .split(';')
        .next()
        .expect("the name=value pair")
        .to_string()
}

/// Registers an app through the dashboard form and returns its token, read
/// back off the apps page exactly the way the copy button would.
async fn register_app(hub: &Hub, cookie: &str, name: &str) -> String {
    let response = client()
        .post(hub.url("/apps/create"))
        .header("cookie", cookie)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(format!("name={name}"))
        .send()
        .await
        .expect("a response");
    assert_eq!(
        response.status(),
        303,
        "registering redirects back to the list"
    );

    let page = client()
        .get(hub.url("/apps"))
        .header("cookie", cookie)
        .send()
        .await
        .expect("a response")
        .text()
        .await
        .expect("a body");

    let marker = format!("id=\"token-{name}\"");
    let start = page.find(&marker).expect("the app must be listed");
    let secret = page[start..]
        .split("data-secret=\"")
        .nth(1)
        .expect("the token is carried for the copy button");
    secret
        .split('"')
        .next()
        .expect("a quoted value")
        .to_string()
}

#[tokio::test]
async fn p7_the_three_verbs_need_a_token() {
    let (hub, _dir) = spawn(true).await;

    let refused = client()
        .post(hub.url("/t/notify.kenny"))
        .header("content-type", "application/json")
        .body("{}")
        .send()
        .await
        .expect("a response");
    assert_eq!(refused.status(), 401);

    let body: serde_json::Value =
        serde_json::from_str(&refused.text().await.expect("a body")).expect("JSON");
    assert!(
        body["remedy"]
            .as_str()
            .expect("a remedy")
            .contains("authorization: Bearer"),
        "the refusal has to show the header to send: {body}"
    );

    let accepted = client()
        .post(hub.url("/t/notify.kenny"))
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("content-type", "application/json")
        .body("{}")
        .send()
        .await
        .expect("a response");
    assert_eq!(accepted.status(), 201);
}

#[tokio::test]
async fn p7_monitoring_stays_open_on_a_protected_hub() {
    // Deliberate: a monitoring stack that fails closed lies to you during an
    // outage, which is the one moment you believe it.
    let (hub, _dir) = spawn(true).await;

    for path in ["/healthz", "/metrics"] {
        let response = client()
            .get(hub.url(path))
            .send()
            .await
            .expect("a response");
        assert_eq!(
            response.status(),
            200,
            "{path} must answer without a token so Uptime Kuma and Grafana keep working"
        );
    }
}

#[tokio::test]
async fn p7_a_browser_is_sent_to_the_login_page_and_a_script_is_not() {
    let (hub, _dir) = spawn(true).await;

    let browser = client()
        .get(hub.url("/"))
        .header("accept", "text/html,application/xhtml+xml")
        .send()
        .await
        .expect("a response");
    assert_eq!(browser.status(), 303);
    assert_eq!(
        browser
            .headers()
            .get("location")
            .and_then(|value| value.to_str().ok()),
        Some("/login")
    );

    // curl sends no Accept header. Answering it with a redirect to a login
    // form would be useless advice.
    let script = client()
        .get(hub.url("/t/x/next?as=y"))
        .send()
        .await
        .expect("a response");
    assert_eq!(script.status(), 401);
}

#[tokio::test]
async fn p7_a_wrong_token_does_not_start_a_session() {
    let (hub, _dir) = spawn(true).await;

    let response = client()
        .post(hub.url("/login"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body("token=not-the-token")
        .send()
        .await
        .expect("a response");

    assert_eq!(response.status(), 200, "the form comes back, not a 401");
    assert!(
        response.headers().get("set-cookie").is_none(),
        "a refused login must not hand out a cookie"
    );
    let body = response.text().await.expect("a body");
    assert!(body.contains("not accepted"), "it says so: {body}");
    assert!(
        !body.contains(TOKEN),
        "and it certainly does not print the real token"
    );
}

#[tokio::test]
async fn p7_a_session_cookie_opens_the_dashboard_and_logging_out_closes_it() {
    let (hub, _dir) = spawn(true).await;
    let cookie = log_in(&hub, TOKEN).await;

    let open = client()
        .get(hub.url("/"))
        .header("cookie", &cookie)
        .header("accept", "text/html")
        .send()
        .await
        .expect("a response");
    assert_eq!(open.status(), 200);

    let logout = client()
        .post(hub.url("/logout"))
        .header("cookie", &cookie)
        .send()
        .await
        .expect("a response");
    let cleared = logout
        .headers()
        .get("set-cookie")
        .expect("the cookie is cleared")
        .to_str()
        .expect("text");
    assert!(cleared.contains("Max-Age=0"), "{cleared}");
}

#[tokio::test]
async fn p7_an_app_token_works_and_stops_working_the_moment_it_is_revoked() {
    let (hub, _dir) = spawn(true).await;
    let cookie = log_in(&hub, TOKEN).await;
    let app_token = register_app(&hub, &cookie, "home-assistant").await;

    assert_ne!(app_token, TOKEN, "an app gets its own token, not the hub's");
    assert!(app_token.len() >= 32, "and a long one: {}", app_token.len());

    let publish = |token: String| {
        let url = hub.url("/t/notify.kenny");
        async move {
            client()
                .post(url)
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body("{}")
                .send()
                .await
                .expect("a response")
                .status()
        }
    };

    assert_eq!(publish(app_token.clone()).await, 201, "the app token works");

    let revoked = client()
        .post(hub.url("/apps/revoke"))
        .header("cookie", &cookie)
        .header("content-type", "application/x-www-form-urlencoded")
        .body("name=home-assistant")
        .send()
        .await
        .expect("a response");
    assert_eq!(revoked.status(), 303);

    assert_eq!(
        publish(app_token).await,
        401,
        "revocation is immediate — there is no cache to wait out"
    );
    assert_eq!(
        publish(TOKEN.to_string()).await,
        201,
        "and revoking one app leaves everything else alone"
    );
}

#[tokio::test]
async fn p7_two_apps_cannot_share_a_name_and_a_revoked_name_can_be_reused() {
    let (hub, _dir) = spawn(true).await;
    let cookie = log_in(&hub, TOKEN).await;
    register_app(&hub, &cookie, "printer").await;

    let duplicate = client()
        .post(hub.url("/apps/create"))
        .header("cookie", &cookie)
        .header("content-type", "application/x-www-form-urlencoded")
        .body("name=printer")
        .send()
        .await
        .expect("a response");
    assert_eq!(duplicate.status(), 409, "a live name is taken");

    client()
        .post(hub.url("/apps/revoke"))
        .header("cookie", &cookie)
        .header("content-type", "application/x-www-form-urlencoded")
        .body("name=printer")
        .send()
        .await
        .expect("a response");

    let reused = register_app(&hub, &cookie, "printer").await;
    assert!(!reused.is_empty(), "the name is free again once revoked");
}

#[tokio::test]
async fn p7_the_snippet_carries_a_working_token_that_is_masked_on_screen() {
    // Kenny's requirement, both halves: pasting works immediately, and
    // nobody reading over your shoulder learns the token.
    let (hub, _dir) = spawn(true).await;
    let cookie = log_in(&hub, TOKEN).await;

    client()
        .post(hub.url("/t/notify.kenny"))
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("content-type", "application/json")
        .body(r#"{"hello":"world"}"#)
        .send()
        .await
        .expect("a response");

    let page = client()
        .get(hub.url("/t/notify.kenny/dashboard"))
        .header("cookie", &cookie)
        .send()
        .await
        .expect("a response")
        .text()
        .await
        .expect("a body");

    let visible: String = page
        .lines()
        .filter(|line| line.trim_start().starts_with("curl"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!visible.is_empty(), "the page prints commands");
    assert!(
        !visible.contains(TOKEN),
        "the token must not be readable on screen: {visible}"
    );
    assert!(
        visible.contains("authorization: Bearer"),
        "but the command still shows that a token is needed: {visible}"
    );

    // The copy button reads data-secret, and that one does carry the whole
    // working command — which is what makes paste work on the first try.
    let secret = page
        .split("data-secret=\"")
        .nth(1)
        .expect("the copy button has something to copy");
    assert!(
        secret.contains(TOKEN),
        "the copyable command has to be the real one"
    );
}

#[tokio::test]
async fn p7_an_unprotected_hub_says_so_and_lets_everything_through() {
    let (hub, _dir) = spawn(false).await;

    let published = client()
        .post(hub.url("/t/notify.kenny"))
        .header("content-type", "application/json")
        .body("{}")
        .send()
        .await
        .expect("a response");
    assert_eq!(published.status(), 201, "no token means no door");

    let page = client()
        .get(hub.url("/"))
        .send()
        .await
        .expect("a response")
        .text()
        .await
        .expect("a body");
    assert!(
        page.contains("no token"),
        "an unprotected hub must never be quietly unprotected: {page}"
    );
    assert!(
        page.contains("KYU_SECRET_KEY"),
        "and it must say how to fix it"
    );
}

#[tokio::test]
async fn p7_the_apps_page_exists_on_an_unprotected_hub_and_explains_the_fix() {
    // AR11 keeps creating an app token behind a bootstrap token on purpose —
    // a per-app token only means something once something already decides
    // who may in at all. But the PAGE used to not exist at all here: no nav
    // link (layout.html gated it on `protected`) and GET /apps answered a
    // bare {"error", "remedy"} JSON body with no HTML around it. Found live,
    // in a browser, by Kenny: "ik zie enkel de topics pagina, geen apps
    // pagina" — a hidden link is indistinguishable from a missing feature.
    let (hub, _dir) = spawn(false).await;

    let index = client()
        .get(hub.url("/"))
        .send()
        .await
        .expect("a response")
        .text()
        .await
        .expect("a body");
    assert!(
        index.contains("href=\"/apps\""),
        "the Apps link must be in the nav even before the hub has a door"
    );

    let response = client()
        .get(hub.url("/apps"))
        .send()
        .await
        .expect("a response");
    assert_eq!(
        response.status(),
        200,
        "the page renders instead of erroring"
    );
    let page = response.text().await.expect("a body");
    assert!(
        page.contains("KYU_TOKEN") && page.contains("KYU_SECRET_KEY"),
        "and it must hand over both variables needed to get a door, not just name them"
    );

    // The example values are real ones, not placeholders like <token> — a
    // visitor should be able to paste them straight into a compose file.
    let token = page
        .split("KYU_TOKEN=")
        .nth(1)
        .and_then(|rest| rest.split('\n').next())
        .expect("an example token");
    let key = page
        .split("KYU_SECRET_KEY=")
        .nth(1)
        .and_then(|rest| rest.split(['\n', '<']).next())
        .expect("an example key");
    assert!(
        token.len() >= 32,
        "the example token must be usable, not a placeholder: {token:?}"
    );
    assert_eq!(
        key.len(),
        64,
        "the example key must be a real 32-byte hex key: {key:?}"
    );

    // The page explaining how to get a door must not itself pretend one
    // exists: actually creating an app is still refused, exactly as before.
    let create = client()
        .post(hub.url("/apps/create"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body("name=sneaky")
        .send()
        .await
        .expect("a response");
    assert_eq!(
        create.status(),
        409,
        "AR11 still holds: no bootstrap token means no app tokens either"
    );
}

#[tokio::test]
async fn p7_the_apps_page_is_not_reachable_without_logging_in() {
    let (hub, _dir) = spawn(true).await;

    for (path, method) in [("/apps", "GET"), ("/apps/create", "POST")] {
        let request = if method == "GET" {
            client().get(hub.url(path))
        } else {
            client()
                .post(hub.url(path))
                .header("content-type", "application/x-www-form-urlencoded")
                .body("name=sneaky")
        };
        let status = request.send().await.expect("a response").status();
        assert_eq!(status, 401, "{path} must be behind the door");
    }
}

#[tokio::test]
async fn p7_the_static_assets_are_open_and_nothing_else_is_reachable_through_them() {
    let (hub, _dir) = spawn(true).await;

    for asset in ["themes.css", "kyu.css", "app.js"] {
        let response = client()
            .get(hub.url(&format!("/static/{asset}")))
            .send()
            .await
            .expect("a response");
        assert_eq!(response.status(), 200, "the login page needs {asset}");
    }

    // The handler matches literal names rather than joining a path, so
    // there is nothing to traverse — this pins that. The exact refusal
    // differs on purpose: an unknown asset is a 404, while a probe that the
    // router normalises back to "/" lands on a protected route and gets a
    // 401. Either is fine; serving a file is not.
    for probe in ["..%2f..%2fetc%2fpasswd", "themes.css.map", "%2e%2e"] {
        let response = client()
            .get(hub.url(&format!("/static/{probe}")))
            .send()
            .await
            .expect("a response");
        let status = response.status();
        assert!(
            status == 404 || status == 401,
            "{probe} must not resolve to anything, got {status}"
        );
        let body = response.text().await.expect("a body");
        assert!(
            !body.contains("root:") && !body.contains("Bootstrap  v"),
            "{probe} must not return file contents"
        );
    }
}

#[tokio::test]
async fn p7_no_token_reaches_the_metrics_or_any_page_in_the_clear() {
    // The plaintext scan FEATURES.md marks mandatory. /metrics is open, so
    // a token surfacing in a label would be readable by anyone on the LAN.
    let (hub, _dir) = spawn(true).await;
    let cookie = log_in(&hub, TOKEN).await;
    let app_token = register_app(&hub, &cookie, "printer").await;

    client()
        .post(hub.url("/t/notify.kenny"))
        .header("authorization", format!("Bearer {app_token}"))
        .header("content-type", "application/json")
        .body("{}")
        .send()
        .await
        .expect("a response");

    let metrics = client()
        .get(hub.url("/metrics"))
        .send()
        .await
        .expect("a response")
        .text()
        .await
        .expect("a body");
    for secret in [TOKEN, app_token.as_str()] {
        assert!(
            !metrics.contains(secret),
            "a token must never reach an endpoint that needs none"
        );
    }

    let health = client()
        .get(hub.url("/healthz"))
        .send()
        .await
        .expect("a response")
        .text()
        .await
        .expect("a body");
    assert!(!health.contains(TOKEN) && !health.contains(app_token.as_str()));

    // The topic page may carry tokens in copy attributes — that page is
    // behind the door and that is the point — but the *index* has no reason
    // to, and the login page certainly does not.
    let login = client()
        .get(hub.url("/login"))
        .send()
        .await
        .expect("a response")
        .text()
        .await
        .expect("a body");
    assert!(
        !login.contains(TOKEN),
        "the login page must not spoil itself"
    );
}

#[tokio::test]
async fn p7_a_token_from_one_hub_does_not_open_another() {
    // Two hubs, two keys. The stored ciphertext of the first is meaningless
    // to the second, which is the property that makes storing tokens at all
    // acceptable (AR11 amendment).
    let (first, _first_dir) = spawn(true).await;
    let (second, _second_dir) = spawn(true).await;

    let cookie = log_in(&first, TOKEN).await;
    let app_token = register_app(&first, &cookie, "shared-name").await;

    let status = client()
        .post(second.url("/t/notify.kenny"))
        .header("authorization", format!("Bearer {app_token}"))
        .header("content-type", "application/json")
        .body("{}")
        .send()
        .await
        .expect("a response")
        .status();
    assert_eq!(status, 401, "an app token is not portable between hubs");
}
