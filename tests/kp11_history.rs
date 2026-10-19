//! [kp-themes 11] The history the redesigned dashboard reads (schema 7):
//! attempts, hourly and daily counters, hub-total samples, their caps, and
//! the JSON routes and JSON answers the pages fetch.
//!
//! The engine half runs on the mock clock; the HTTP half goes through the
//! real kit app like every other dashboard test.

mod common;

use std::sync::Arc;

use common::{KitHub, body_json, spawn_kit};
use kyu::engine::clock::MockClock;
use kyu::engine::days;
use kyu::engine::{Defaults, Engine};
use kyu::store::Store;
use kyu::store::migrations::{MIGRATIONS, migrate_with};
use kyu::store::queries::{self, StoredPolicy};
use serde_json::{Value, json};

const START: i64 = 1_700_000_000_000;
const MINUTE: i64 = 60_000;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;

struct Fixture {
    engine: Engine,
    clock: Arc<MockClock>,
    store: Arc<Store>,
}

fn fixture() -> Fixture {
    let store = Arc::new(Store::open_in_memory().expect("an in-memory store"));
    let clock = Arc::new(MockClock::new(START));
    let engine = Engine::with_defaults(store.clone(), clock.clone(), Defaults::default());
    Fixture {
        engine,
        clock,
        store,
    }
}

impl Fixture {
    /// The topic and the subscription exist, with a policy that makes a
    /// message die after `max_attempts` tries.
    fn bootstrap(&self, max_attempts: i64) {
        self.engine
            .publish("jobs", b"{}", Some("application/json"))
            .expect("the bootstrap publish");
        self.engine
            .claim_next("jobs", "worker", false)
            .expect("the first poll");
        self.engine
            .set_policy(
                "jobs",
                "worker",
                StoredPolicy {
                    max_attempts: Some(max_attempts),
                    backoff_ms: Some(1),
                    lease_ms: Some(1_000),
                    ..StoredPolicy::default()
                },
            )
            .expect("a policy");
    }

    fn publish(&self) -> String {
        self.engine
            .publish("jobs", b"{\"n\":1}", Some("application/json"))
            .expect("a publish")
            .id
    }

    fn claim(&self, app: Option<&str>) -> String {
        self.engine
            .claim_next_as("jobs", "worker", false, app)
            .expect("a claim")
            .claimed
            .expect("something to claim")
            .message
            .id
    }

    /// `(n, outcome, app, reason)` for each attempt, oldest first.
    fn attempts(&self, id: &str) -> Vec<(i64, String, Option<String>, Option<String>)> {
        self.store
            .read(|conn| {
                let topic_id = queries::topic_id_by_name_conn(conn, "jobs")?.unwrap();
                let sub_id =
                    queries::subscription_id_by_name_conn(conn, topic_id, "worker")?.unwrap();
                Ok(queries::attempts_for(conn, sub_id, id)?
                    .expect("the delivery exists")
                    .into_iter()
                    .map(|a| (a.n, a.outcome, a.app, a.reason))
                    .collect())
            })
            .expect("the attempts")
    }

    fn count(&self, sql: &str) -> i64 {
        self.store
            .with_conn(|conn| conn.query_row(sql, [], |row| row.get(0)))
            .expect("a count")
    }
}

#[test]
fn kp11_an_acked_claim_is_one_attempt_with_the_app_that_took_it() {
    let f = fixture();
    f.bootstrap(3);
    let id = f.publish();
    assert_eq!(f.claim(Some("kyu-runner")), id);

    let attempts = f.attempts(&id);
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].1, "in_flight");
    assert_eq!(attempts[0].2.as_deref(), Some("kyu-runner"));

    f.clock.advance(250);
    f.engine.ack("jobs", "worker", &id).expect("the ack");
    let attempts = f.attempts(&id);
    assert_eq!(attempts[0].1, "acked");
    assert_eq!(
        f.count("SELECT delivered FROM stats_daily WHERE topic = 'jobs'"),
        1
    );
    assert_eq!(
        f.count("SELECT ended_at - claimed_at FROM attempts"),
        250,
        "the attempt records how long it was held"
    );
}

#[test]
fn kp11_nacks_then_exhaustion_leave_three_attempts_and_one_dead_letter() {
    let f = fixture();
    f.bootstrap(3);
    let id = f.publish();
    for _ in 0..3 {
        f.claim(None);
        f.engine.nack("jobs", "worker", &id, false).expect("a nack");
        f.clock.advance(50);
    }
    let outcomes: Vec<String> = f.attempts(&id).into_iter().map(|a| a.1).collect();
    assert_eq!(outcomes, ["nacked", "nacked", "dead"]);
    assert_eq!(f.attempts(&id)[2].0, 3, "attempts are numbered in order");

    assert_eq!(f.count("SELECT sum(dead) FROM stats_hourly"), 1);
    assert_eq!(f.count("SELECT sum(dead) FROM stats_daily"), 1);
    // The bootstrap message and the one above.
    assert_eq!(f.count("SELECT sum(published) FROM stats_hourly"), 2);
}

#[test]
fn kp11_a_nack_straight_to_dead_ends_the_attempt_as_dead() {
    let f = fixture();
    f.bootstrap(5);
    let id = f.publish();
    f.claim(None);
    f.engine.nack("jobs", "worker", &id, true).expect("a nack");
    let attempts = f.attempts(&id);
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].1, "dead");
    assert_eq!(f.count("SELECT sum(dead) FROM stats_daily"), 1);
}

#[test]
fn kp11_a_lease_that_runs_out_is_an_expired_attempt_and_the_next_claim_is_n_two() {
    let f = fixture();
    f.bootstrap(3);
    let id = f.publish();
    f.claim(None);
    f.clock.advance(1_000);
    let report = f.engine.sweep(100).expect("a sweep");
    assert_eq!(report.redelivered, 1);
    f.clock.advance(10);
    f.claim(None);

    let attempts = f.attempts(&id);
    assert_eq!(attempts[0].1, "expired");
    assert_eq!(attempts[1].0, 2);
    assert_eq!(attempts[1].1, "in_flight");
}

#[test]
fn kp11_requeueing_a_dead_letter_continues_the_numbering() {
    let f = fixture();
    f.bootstrap(5);
    let id = f.publish();
    f.claim(None);
    f.engine.nack("jobs", "worker", &id, true).expect("a nack");
    f.engine
        .requeue_dead("jobs", "worker", &id)
        .expect("a requeue");
    f.claim(None);
    let numbers: Vec<i64> = f.attempts(&id).into_iter().map(|a| a.0).collect();
    assert_eq!(numbers, [1, 2]);
}

#[test]
fn kp11_deleting_a_claimed_delivery_does_not_leave_an_attempt_in_flight() {
    let f = fixture();
    f.bootstrap(5);
    let id = f.publish();
    f.claim(None);
    f.engine
        .delete_delivery("jobs", "worker", &id)
        .expect("a delete");
    assert_ne!(f.attempts(&id)[0].1, "in_flight");
}

#[test]
fn kp11_retention_takes_the_attempts_with_the_message() {
    let f = fixture();
    f.bootstrap(5);
    let id = f.publish();
    f.claim(None);
    f.engine.ack("jobs", "worker", &id).expect("the ack");
    assert_eq!(f.count("SELECT count(*) FROM attempts"), 1);

    f.clock.advance(8 * DAY);
    f.engine.sweep(100).expect("a sweep");
    assert_eq!(
        f.count(&format!("SELECT count(*) FROM messages WHERE id = '{id}'")),
        0,
        "retention collected the message"
    );
    assert_eq!(f.count("SELECT count(*) FROM attempts"), 0);
}

#[test]
fn kp11_the_sweeper_samples_every_five_minutes_and_keeps_48_hours() {
    let f = fixture();
    f.bootstrap(3);
    f.publish();
    f.engine.sweep(100).expect("a sweep");
    assert_eq!(f.count("SELECT count(*) FROM stats_samples"), 1);

    f.clock.advance(4 * MINUTE);
    f.engine.sweep(100).expect("a sweep");
    assert_eq!(f.count("SELECT count(*) FROM stats_samples"), 1, "not yet");

    f.clock.advance(MINUTE);
    f.engine.sweep(100).expect("a sweep");
    assert_eq!(f.count("SELECT count(*) FROM stats_samples"), 2);
    assert_eq!(
        f.count("SELECT backlog FROM stats_samples ORDER BY at LIMIT 1"),
        1,
        "the sample counts what is waiting"
    );

    f.clock.advance(49 * HOUR);
    f.engine.sweep(100).expect("a sweep");
    assert_eq!(
        f.count("SELECT count(*) FROM stats_samples"),
        1,
        "the old samples are pruned, the new one is written"
    );
}

#[test]
fn kp11_counters_and_attempts_have_their_caps() {
    let f = fixture();
    f.bootstrap(3);
    let id = f.publish();
    f.claim(None);
    f.engine.nack("jobs", "worker", &id, true).expect("a nack");
    assert!(f.count("SELECT count(*) FROM stats_hourly") > 0);
    assert!(f.count("SELECT count(*) FROM stats_daily") > 0);

    // Hourly counters go after 30 days, daily ones after 400; the
    // attempts after 30 even though the message is still held.
    f.engine
        .set_retention("jobs", None)
        .expect("keep the topic forever");
    f.clock.advance(31 * DAY);
    f.engine.sweep(100).expect("a sweep");
    assert_eq!(f.count("SELECT count(*) FROM stats_hourly"), 0);
    assert_eq!(f.count("SELECT count(*) FROM attempts"), 0);
    assert!(f.count("SELECT count(*) FROM stats_daily") > 0);

    f.clock.advance(400 * DAY);
    f.engine.sweep(100).expect("a sweep");
    assert_eq!(f.count("SELECT count(*) FROM stats_daily"), 0);
}

#[test]
fn kp11_a_day_is_a_brussels_day() {
    // 2026-01-15 23:30 UTC is 00:30 on the 16th in Brussels (CET)...
    let winter = 20_468 * DAY + 23 * HOUR + 30 * MINUTE;
    assert_eq!(days::day_label(days::day_of(winter)), "2026-01-16");
    // ...and in summer (CEST, UTC+2) 21:30 UTC is still 23:30 on the day.
    let summer = 20_650 * DAY + 21 * HOUR + 30 * MINUTE;
    assert_eq!(days::day_label(days::day_of(summer)), "2026-07-16");
    assert_eq!(days::day_label(days::day_of(summer + HOUR)), "2026-07-17");
    assert_eq!(days::day_label(0), "1970-01-01");
}

#[test]
fn kp11_migration_seven_keeps_every_row_of_a_schema_six_store() {
    let mut conn = rusqlite::Connection::open_in_memory().expect("a database");
    migrate_with(&mut conn, &MIGRATIONS[..6], None).expect("schema 6");
    conn.execute(
        "INSERT INTO topics (name, created_at) VALUES ('old', 1)",
        [],
    )
    .expect("a topic");
    conn.execute(
        "INSERT INTO messages (id, topic_id, payload, published_at)
         VALUES ('m1', (SELECT id FROM topics WHERE name = 'old'), x'7b7d', 5)",
        [],
    )
    .expect("a message");

    migrate_with(&mut conn, MIGRATIONS, None).expect("schema 7");
    let version: u32 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("a version");
    assert_eq!(version as usize, MIGRATIONS.len());
    let messages: i64 = conn
        .query_row("SELECT count(*) FROM messages WHERE id = 'm1'", [], |row| {
            row.get(0)
        })
        .expect("a count");
    assert_eq!(messages, 1);
    for table in ["attempts", "stats_hourly", "stats_daily", "stats_samples"] {
        conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap_or_else(|error| panic!("{table} must exist: {error}"));
    }
}

// ─── HTTP · the routes the pages fetch ──────────────────────────────────────

/// A dashboard JSON post as the page's script sends it.
async fn post_json(hub: &KitHub, path: &str, body: Value) -> reqwest::Response {
    reqwest::Client::new()
        .post(hub.url(path))
        .header("cookie", hub.session_cookie())
        .header("content-type", "application/json")
        .header("accept", "application/json")
        .body(body.to_string())
        .send()
        .await
        .expect("a response")
}

/// A topic `jobs` whose subscription `worker` holds one dead letter.
async fn hub_with_a_dead_letter() -> (KitHub, String) {
    let hub = spawn_kit().await;
    hub.bootstrap("jobs", "worker").await;
    let id = hub.publish("jobs", r#"{"n":1}"#).await;
    let claimed = hub.receive("jobs", "as=worker&wait=0").await;
    assert_eq!(claimed.status(), 200);
    let nack = hub
        .post_api(&format!("/t/jobs/nack/{id}?as=worker&dead=true"))
        .await;
    assert_eq!(nack.status(), 200);
    (hub, id)
}

#[tokio::test]
async fn kp11_the_attempts_route_tells_the_story_of_one_message() {
    let (hub, id) = hub_with_a_dead_letter().await;
    let response = hub
        .get(&format!("/t/jobs/dashboard/subs/worker/attempts?id={id}"))
        .await;
    assert_eq!(response.status(), 200);
    let body = body_json(response).await;
    assert_eq!(body["message"], id);
    let attempts = body["attempts"].as_array().expect("a list");
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0]["n"], 1);
    assert_eq!(attempts[0]["outcome"], "dead");
    assert_eq!(attempts[0]["app"], "admin", "the login token polled");
    assert!(attempts[0]["claimed_at"].is_i64());
    assert!(attempts[0]["ended_at"].is_i64());
    assert!(attempts[0]["reason"].is_string());

    let missing = hub.get("/t/jobs/dashboard/subs/worker/attempts").await;
    assert_eq!(missing.status(), 400);
    assert!(body_json(missing).await["remedy"].is_string());
    let unknown = hub
        .get("/t/jobs/dashboard/subs/worker/attempts?id=nope")
        .await;
    assert_eq!(unknown.status(), 404);
    let no_sub = hub.get("/t/jobs/dashboard/subs/ghost/attempts?id=x").await;
    assert_eq!(no_sub.status(), 404);
    hub.shutdown().await;
}

#[tokio::test]
async fn kp11_the_daily_route_fills_every_day_of_the_window() {
    let (hub, _) = hub_with_a_dead_letter().await;
    let body = body_json(hub.get("/t/jobs/dashboard/subs/worker/daily?days=7").await).await;
    let days = body["days"].as_array().expect("a list");
    assert_eq!(days.len(), 7);
    let today = days.last().unwrap();
    assert_eq!(today["dead"], 1);
    assert_eq!(today["delivered"], 0);
    assert_eq!(days[0]["dead"], 0);
    assert!(days[0]["day"].as_str().unwrap().len() == 10);
    assert_eq!(
        hub.get("/t/jobs/dashboard/subs/ghost/daily").await.status(),
        404
    );
    hub.shutdown().await;
}

#[tokio::test]
async fn kp11_the_hourly_route_lines_every_topic_up_with_the_hours() {
    let (hub, _) = hub_with_a_dead_letter().await;
    let body = body_json(hub.get("/dashboard/stats/hourly?hours=6").await).await;
    let hours = body["hours"].as_array().expect("hour starts");
    assert_eq!(hours.len(), 6);
    let jobs = body["topics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|topic| topic["topic"] == "jobs")
        .expect("the topic is listed");
    assert_eq!(jobs["published"].as_array().unwrap().len(), 6);
    let published: i64 = jobs["published"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_i64().unwrap())
        .sum();
    let dead: i64 = jobs["dead"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_i64().unwrap())
        .sum();
    assert_eq!((published, dead), (2, 1));
    hub.shutdown().await;
}

#[tokio::test]
async fn kp11_the_totals_route_answers_a_list_of_samples() {
    let hub = spawn_kit().await;
    let body = body_json(hub.get("/dashboard/stats/totals?hours=24").await).await;
    assert!(body["samples"].is_array());
    hub.shutdown().await;
}

#[tokio::test]
async fn kp11_the_retention_route_says_how_long_dead_letters_stay() {
    let (hub, _) = hub_with_a_dead_letter().await;
    let body = body_json(hub.get("/t/jobs/dashboard/retention").await).await;
    assert_eq!(body["dead_letter_days"], 7, "the hub default is a week");
    assert_eq!(hub.get("/t/ghost/dashboard/retention").await.status(), 404);
    hub.shutdown().await;
}

#[tokio::test]
async fn kp11_bulk_requeue_reports_what_it_did_and_what_it_refused() {
    let (hub, id) = hub_with_a_dead_letter().await;
    let response = post_json(
        &hub,
        "/t/jobs/dashboard/bulk/requeue",
        json!({ "subscription": "worker", "ids": [id, "never-delivered"] }),
    )
    .await;
    assert_eq!(response.status(), 200);
    let body = body_json(response).await;
    assert_eq!(body["done"], json!([id]));
    let refused = body["refused"].as_array().unwrap();
    assert_eq!(refused.len(), 1);
    assert_eq!(refused[0]["id"], "never-delivered");
    assert!(refused[0]["reason"].is_string());

    // The requeued letter can be claimed again.
    assert_eq!(hub.receive("jobs", "as=worker&wait=0").await.status(), 200);

    let bad = post_json(&hub, "/t/jobs/dashboard/bulk/requeue", json!({ "ids": [] })).await;
    assert_eq!(bad.status(), 400);
    let ghost = post_json(
        &hub,
        "/t/jobs/dashboard/bulk/requeue",
        json!({ "subscription": "ghost", "ids": ["x"] }),
    )
    .await;
    assert_eq!(ghost.status(), 404);
    hub.shutdown().await;
}

#[tokio::test]
async fn kp11_bulk_delete_removes_deliveries_and_refuses_strangers() {
    let (hub, id) = hub_with_a_dead_letter().await;
    let response = post_json(
        &hub,
        "/t/jobs/dashboard/bulk/delete",
        json!({ "subscription": "worker", "ids": [id, "stranger"] }),
    )
    .await;
    assert_eq!(response.status(), 200);
    let body = body_json(response).await;
    assert_eq!(body["done"], json!([id]));
    assert_eq!(body["refused"].as_array().unwrap().len(), 1);
    assert_eq!(hub.receive("jobs", "as=worker&wait=0").await.status(), 204);
    hub.shutdown().await;
}

#[tokio::test]
async fn kp11_replay_publishes_a_stored_message_again_and_refuses_a_missing_one() {
    let hub = spawn_kit().await;
    hub.bootstrap("jobs", "worker").await;
    let original = hub.publish("jobs", r#"{"n":7}"#).await;
    let response = post_json(
        &hub,
        &format!("/t/jobs/dashboard/messages/{original}/replay"),
        json!({}),
    )
    .await;
    assert_eq!(response.status(), 200);
    let new_id = body_json(response).await["id"]
        .as_str()
        .expect("a new id")
        .to_string();
    assert_ne!(new_id, original);

    let first = hub.receive("jobs", "as=worker&wait=0").await;
    assert_eq!(first.text().await.unwrap(), r#"{"n":7}"#);
    let second = hub.receive("jobs", "as=worker&wait=0").await;
    assert_eq!(second.text().await.unwrap(), r#"{"n":7}"#);

    let gone = post_json(
        &hub,
        "/t/jobs/dashboard/messages/01NOSUCHMESSAGE/replay",
        json!({}),
    )
    .await;
    assert_eq!(gone.status(), 404);
    let body = body_json(gone).await;
    assert!(body["error"].is_string() && body["remedy"].is_string());
    hub.shutdown().await;
}

#[tokio::test]
async fn kp11_the_form_posts_answer_json_when_asked_and_redirect_otherwise() {
    let (hub, id) = hub_with_a_dead_letter().await;

    let json_answer = reqwest::Client::new()
        .post(hub.url("/t/jobs/dashboard/requeue"))
        .header("cookie", hub.session_cookie())
        .header("content-type", "application/x-www-form-urlencoded")
        .header("accept", "application/json")
        .body(format!("subscription=worker&id={id}"))
        .send()
        .await
        .expect("a response");
    assert_eq!(json_answer.status(), 200);
    assert_eq!(body_json(json_answer).await, json!({ "ok": true }));

    let plain = hub
        .form("/t/jobs/dashboard/publish", "payload=%7B%7D")
        .await;
    assert!(plain.status().is_redirection(), "the no-script path stays");

    let failing = reqwest::Client::new()
        .post(hub.url("/t/jobs/dashboard/requeue"))
        .header("cookie", hub.session_cookie())
        .header("content-type", "application/x-www-form-urlencoded")
        .header("accept", "application/json")
        .body("subscription=worker&id=never")
        .send()
        .await
        .expect("a response");
    assert!(failing.status().is_client_error());
    let body = body_json(failing).await;
    assert!(body["error"].is_string() && body["remedy"].is_string());
    hub.shutdown().await;
}
