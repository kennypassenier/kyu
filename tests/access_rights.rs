//! Who may call what on the machine API: an app token publishes and
//! consumes, the login token manages, and the consuming verbs need the
//! bearer header rather than a browser session.

mod common;

use common::{body_json, spawn_kit};
use reqwest::Method;

#[tokio::test]
async fn an_app_token_cannot_manage_the_hub_but_the_login_token_can() {
    let hub = spawn_kit().await;
    hub.publish("notify.kenny", "{}").await;
    let app = hub.issue_app("printer").await;

    for (method, path) in [
        (Method::POST, "/api/backup"),
        (Method::GET, "/api/t/notify.kenny/retention"),
        (Method::POST, "/api/t/notify.kenny/subs/s/unarchive"),
    ] {
        let response = hub
            .bearer(method, path, &app)
            .send()
            .await
            .expect("a response");
        assert_eq!(response.status(), 403, "{path}");
        let body = body_json(response).await;
        assert!(
            body["remedy"].as_str().unwrap_or("").contains("KYU_TOKEN"),
            "{body}"
        );
    }

    let admin = hub
        .bearer(Method::GET, "/api/t/notify.kenny/retention", hub.token())
        .send()
        .await
        .expect("a response");
    assert_eq!(admin.status(), 200, "the login token still manages");

    let polled = hub
        .bearer(Method::GET, "/t/notify.kenny/next?as=printer&wait=0", &app)
        .send()
        .await
        .expect("a response");
    assert!(
        polled.status() == 200 || polled.status() == 204,
        "an app token still consumes"
    );
    hub.shutdown().await;
}

#[tokio::test]
async fn consuming_verbs_refuse_a_browser_session_and_ask_for_the_bearer() {
    let hub = spawn_kit().await;
    hub.publish("notify.kenny", "{}").await;

    for (method, path) in [
        (Method::GET, "/t/notify.kenny/next?as=printer&wait=0"),
        (Method::POST, "/t/notify.kenny/ack/x?as=printer"),
        (Method::POST, "/t/notify.kenny/nack/x?as=printer"),
    ] {
        // `request` carries the admin session cookie and no bearer.
        let response = hub
            .session_request(method, path)
            .send()
            .await
            .expect("a response");
        assert_eq!(response.status(), 401, "{path}");
        let body = body_json(response).await;
        assert!(
            body["remedy"]
                .as_str()
                .unwrap_or("")
                .contains("Authorization: Bearer"),
            "{body}"
        );
    }

    let bearer = hub
        .bearer(
            Method::GET,
            "/t/notify.kenny/next?as=printer&wait=0",
            hub.token(),
        )
        .send()
        .await
        .expect("a response");
    assert!(
        bearer.status() == 200 || bearer.status() == 204,
        "the bearer is accepted"
    );
    hub.shutdown().await;
}
