//! [L6] History and lifecycle (K8, K9, K11, W11) on the mock clock.
//!
//! Retention, replay and the idle lifecycle all turn on durations measured
//! in days, so every test here drives time by hand rather than waiting.

use std::sync::Arc;

use kyu::engine::clock::MockClock;
use kyu::engine::policy::Policy;
use kyu::engine::{Defaults, Engine};
use kyu::events::EVENTS_TOPIC;
use kyu::store::Store;
use kyu::store::queries::StoredPolicy;

const START: i64 = 1_700_000_000_000;
const DAY: i64 = 24 * 60 * 60 * 1_000;

struct Fixture {
    engine: Engine,
    clock: Arc<MockClock>,
    store: Arc<Store>,
}

fn fixture_with(defaults: Defaults) -> Fixture {
    let store = Arc::new(Store::open_in_memory().expect("an in-memory store"));
    let clock = Arc::new(MockClock::new(START));
    let engine = Engine::with_defaults(store.clone(), clock.clone(), defaults);
    Fixture {
        engine,
        clock,
        store,
    }
}

fn fixture() -> Fixture {
    fixture_with(Defaults::default())
}

impl Fixture {
    fn bootstrap(&self, topic: &str, subscription: &str) {
        self.engine
            .publish(topic, b"{}", Some("application/json"))
            .expect("the bootstrap publish");
        self.engine
            .claim_next(topic, subscription, false)
            .expect("the first poll");
    }

    fn publish(&self, topic: &str, body: &str) -> String {
        self.engine
            .publish(topic, body.as_bytes(), Some("application/json"))
            .expect("a publish")
            .id
    }

    fn claim(&self, topic: &str, subscription: &str) -> Option<String> {
        self.engine
            .claim_next(topic, subscription, false)
            .expect("a claim")
            .claimed
            .map(|claimed| claimed.message.id)
    }

    fn count(&self, sql: &str) -> i64 {
        self.store
            .with_conn(|conn| conn.query_row(sql, [], |row| row.get(0)))
            .expect("a count")
    }

    fn state_of(&self, subscription: &str) -> String {
        self.store
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT state FROM subscriptions WHERE name = ?1",
                    [subscription],
                    |row| row.get::<_, String>(0),
                )
            })
            .expect("the subscription must exist")
    }

    /// Drains the hub's own event topic through a subscription created for
    /// the purpose, returning the event names in order.
    fn event_kinds(&self, subscription: &str) -> Vec<String> {
        let mut kinds = Vec::new();
        while let Some(id) = self.claim(EVENTS_TOPIC, subscription) {
            let payload: Vec<u8> = self
                .store
                .with_conn(|conn| {
                    conn.query_row("SELECT payload FROM messages WHERE id = ?1", [&id], |row| {
                        row.get(0)
                    })
                })
                .expect("the event payload");
            let value: serde_json::Value =
                serde_json::from_slice(&payload).expect("an event is JSON");
            kinds.push(value["event"].as_str().expect("an event name").to_string());
            self.engine
                .ack(EVENTS_TOPIC, subscription, &id)
                .expect("the ack");
        }
        kinds
    }
}

// ─── K9 · retention ─────────────────────────────────────────────────────────

#[test]
fn l6_retention_collects_messages_nobody_is_waiting_for() {
    let f = fixture();
    f.bootstrap("notify.kenny", "printer");
    let id = f.publish("notify.kenny", r#"{"n":1}"#);
    let claimed = f.claim("notify.kenny", "printer").expect("a claim");
    assert_eq!(claimed, id);
    f.engine
        .ack("notify.kenny", "printer", &id)
        .expect("the ack");

    // Settled, but still retained for replay and for the dashboard.
    f.clock.advance(6 * DAY);
    f.engine.sweep(100).expect("a sweep");
    assert!(
        f.count("SELECT count(*) FROM messages") > 0,
        "inside the window the message is kept"
    );

    f.clock.advance(2 * DAY);
    let report = f.engine.sweep(100).expect("a sweep");
    assert!(report.collected > 0, "past the window it is collected");
    assert_eq!(
        f.count(
            "SELECT count(*) FROM messages m JOIN topics t ON t.id = m.topic_id \
             WHERE t.name = 'notify.kenny'"
        ),
        0,
        "and the topic is empty again"
    );
}

#[test]
fn l6_retention_never_collects_a_backlog_an_active_subscription_still_needs() {
    let f = fixture();
    f.bootstrap("print.receipt", "printer");
    // The printer LXC is switched off for a fortnight; the messages pile up.
    for n in 0..5 {
        f.publish("print.receipt", &format!(r#"{{"n":{n}}}"#));
    }

    f.clock.advance(14 * DAY);
    f.engine.sweep(1000).expect("a sweep");

    assert_eq!(
        f.count("SELECT count(*) FROM deliveries WHERE state = 'pending'"),
        5,
        "backlogs win: a message an active subscription still needs is never \
         collected, however old it is"
    );

    // Once it is finally handled, retention may take it.
    while let Some(id) = f.claim("print.receipt", "printer") {
        f.engine
            .ack("print.receipt", "printer", &id)
            .expect("the ack");
    }
    f.clock.advance(1);
    let report = f.engine.sweep(1000).expect("a sweep");
    assert!(
        report.collected >= 5,
        "settled and past its window, it goes"
    );
}

#[test]
fn l6_a_topic_can_keep_its_messages_forever() {
    let f = fixture();
    f.bootstrap("notify.kenny", "printer");
    f.engine
        .set_retention("notify.kenny", Some(i64::MAX))
        .expect("keep forever");

    let id = f.publish("notify.kenny", r#"{"n":1}"#);
    let claimed = f.claim("notify.kenny", "printer").expect("a claim");
    f.engine
        .ack("notify.kenny", "printer", &claimed)
        .expect("the ack");

    f.clock.advance(365 * DAY);
    f.engine.sweep(1000).expect("a sweep");

    assert_eq!(
        f.count(&format!("SELECT count(*) FROM messages WHERE id = '{id}'")),
        1,
        "a topic set to keep forever is not collected"
    );
}

// ─── K8 · replay ────────────────────────────────────────────────────────────

#[test]
fn l6_replay_gives_a_subscription_the_history_it_missed() {
    let f = fixture();
    let mut published = Vec::new();
    for n in 0..4 {
        published.push(f.publish("notify.kenny", &format!(r#"{{"n":{n}}}"#)));
    }

    // A brand-new subscription normally starts at "now" (G7)...
    let received = f
        .engine
        .claim_next("notify.kenny", "latecomer", false)
        .expect("a poll");
    assert!(received.claimed.is_none());

    // ...but asking explicitly replays what the topic still retains (K8).
    let received = f
        .engine
        .claim_next("notify.kenny", "latecomer", true)
        .expect("a replay poll");
    assert_eq!(received.backfilled, 4);
    assert_eq!(
        received.claimed.map(|claimed| claimed.message.id),
        Some(published[0].clone()),
        "and it starts at the oldest retained message"
    );
}

#[test]
fn l6_replay_is_idempotent() {
    let f = fixture();
    for n in 0..3 {
        f.publish("notify.kenny", &format!(r#"{{"n":{n}}}"#));
    }

    let first = f
        .engine
        .claim_next("notify.kenny", "latecomer", true)
        .expect("a replay poll");
    assert_eq!(first.backfilled, 3);

    let second = f
        .engine
        .claim_next("notify.kenny", "latecomer", true)
        .expect("a second replay poll");
    assert_eq!(
        second.backfilled, 0,
        "asking twice must not duplicate anything"
    );
    assert_eq!(
        f.count("SELECT count(*) FROM deliveries"),
        3,
        "one delivery row per message, still"
    );
}

#[test]
fn l6_replay_sees_exactly_what_retention_kept() {
    let f = fixture();
    f.bootstrap("notify.kenny", "printer");
    let old = f.publish("notify.kenny", r#"{"age":"old"}"#);

    // Settle the printer's copies so retention is free to collect them.
    while let Some(id) = f.claim("notify.kenny", "printer") {
        f.engine
            .ack("notify.kenny", "printer", &id)
            .expect("the ack");
    }

    f.clock.advance(8 * DAY);
    let collected = f.engine.sweep(1000).expect("a sweep").collected;
    assert!(collected > 0, "the old messages are past the window");

    let fresh = f.publish("notify.kenny", r#"{"age":"fresh"}"#);

    let received = f
        .engine
        .claim_next("notify.kenny", "archaeologist", true)
        .expect("a replay poll");
    assert_eq!(
        received.backfilled, 1,
        "replay can only offer what is still there — the boundary is retention"
    );
    assert_eq!(
        received.claimed.map(|claimed| claimed.message.id),
        Some(fresh)
    );
    assert_ne!(
        f.count(&format!("SELECT count(*) FROM messages WHERE id = '{old}'")),
        1,
        "the collected message is genuinely gone"
    );
}

// ─── K11 · idle lifecycle ───────────────────────────────────────────────────

#[test]
fn l6_an_idle_subscription_is_flagged_then_archived() {
    // Retention off, so the lapsed rows stay visible: once a subscription is
    // archived its messages become collectable, and the two would otherwise
    // happen in the same sweep.
    let f = fixture_with(Defaults {
        retention_ms: None,
        ..Defaults::default()
    });
    f.bootstrap("notify.kenny", "forgotten");
    assert_eq!(f.state_of("forgotten"), "active");

    f.clock.advance(8 * DAY);
    let report = f.engine.sweep(100).expect("a sweep");
    assert_eq!(report.flagged, 1);
    assert_eq!(f.state_of("forgotten"), "flagged");

    // Still receiving while flagged: a warning is not a disconnection.
    let id = f.publish("notify.kenny", r#"{"n":1}"#);
    assert_eq!(
        f.count(&format!(
            "SELECT count(*) FROM deliveries d JOIN messages m ON m.seq = d.msg_seq \
             WHERE m.id = '{id}'"
        )),
        1,
        "a flagged subscription still receives: a warning is not a disconnection"
    );

    f.clock.advance(31 * DAY);
    let report = f.engine.sweep(100).expect("a sweep");
    assert_eq!(report.archived, 1);
    assert_eq!(f.state_of("forgotten"), "archived");
    assert!(
        report.lapsed >= 1,
        "what it was holding is settled as lapsed, not silently dropped"
    );
    assert_eq!(
        f.count("SELECT count(*) FROM deliveries WHERE state = 'lapsed'"),
        report.lapsed as i64
    );
}

#[test]
fn l6_polling_clears_a_flag_but_will_not_revive_an_archive() {
    let f = fixture();
    f.bootstrap("notify.kenny", "sleepy");

    f.clock.advance(8 * DAY);
    f.engine.sweep(100).expect("a sweep");
    assert_eq!(f.state_of("sleepy"), "flagged");

    // Polling is the definition of not-idle.
    f.claim("notify.kenny", "sleepy");
    assert_eq!(f.state_of("sleepy"), "active");

    f.clock.advance(31 * DAY);
    f.engine.sweep(100).expect("a sweep");
    assert_eq!(f.state_of("sleepy"), "archived");

    // An archived subscription refuses to be revived by accident.
    let error = f
        .engine
        .claim_next("notify.kenny", "sleepy", false)
        .expect_err("polling an archived subscription must not silently resume it");
    assert!(format!("{error}").contains("archived"));
    assert!(
        error.remedy().contains("unarchive"),
        "and it must point at the way back: {}",
        error.remedy()
    );

    f.engine
        .unarchive("notify.kenny", "sleepy")
        .expect("the unarchive");
    assert_eq!(f.state_of("sleepy"), "active");
    f.claim("notify.kenny", "sleepy");
}

#[test]
fn l6_an_archived_subscription_stops_accumulating() {
    let f = fixture();
    f.bootstrap("notify.kenny", "gone");
    f.clock.advance(31 * DAY);
    f.engine.sweep(100).expect("a sweep");
    assert_eq!(f.state_of("gone"), "archived");

    let before = f.count("SELECT count(*) FROM deliveries");
    f.publish("notify.kenny", r#"{"n":1}"#);
    assert_eq!(
        f.count("SELECT count(*) FROM deliveries"),
        before,
        "this is what keeps a forgotten subscription from filling the disk"
    );
}

// ─── W11 · the hub's own events ─────────────────────────────────────────────

#[test]
fn l6_the_hub_publishes_its_own_events() {
    let f = fixture();
    f.bootstrap("print.receipt", "printer");
    // Someone is listening to the hub itself.
    f.engine
        .claim_next(EVENTS_TOPIC, "ha-forwarder", false)
        .expect("subscribe to the events topic");

    // A dead letter.
    let id = f.publish("print.receipt", r#"{"receipt":"kapot"}"#);
    f.claim("print.receipt", "printer");
    f.engine
        .nack("print.receipt", "printer", &id, true)
        .expect("a poison-pill nack");

    // An idle subscription being flagged.
    f.clock.advance(8 * DAY);
    f.engine.sweep(100).expect("a sweep");

    let kinds = f.event_kinds("ha-forwarder");
    assert!(
        kinds.contains(&"subscription.flagged".to_string()),
        "flagging is announced: {kinds:?}"
    );
}

#[test]
fn l6_events_about_the_events_topic_are_logged_not_republished() {
    let f = fixture();
    // A consumer of the hub's own events that keeps failing.
    f.engine
        .claim_next(EVENTS_TOPIC, "broken", false)
        .expect("subscribe");
    f.engine
        .set_policy(
            EVENTS_TOPIC,
            "broken",
            StoredPolicy {
                max_attempts: Some(1),
                lease_ms: Some(1),
                backoff_ms: Some(0),
                ..StoredPolicy::default()
            },
        )
        .expect("a policy");

    // Cause a real event, so the events topic gains a message.
    f.bootstrap("notify.kenny", "someone");
    f.clock.advance(8 * DAY);
    f.engine.sweep(100).expect("a sweep");
    let before = f.count("SELECT count(*) FROM messages");

    // Now let the broken consumer dead-letter that event. Without the
    // loop-breaker, dead-lettering it would emit another event about the
    // events topic, which would dead-letter, for ever.
    for _ in 0..5 {
        f.claim(EVENTS_TOPIC, "broken");
        f.clock.advance(10);
        f.engine.sweep(100).expect("a sweep");
    }

    let after = f.count("SELECT count(*) FROM messages");
    assert!(
        after <= before + 1,
        "a dead letter on the events topic must not breed more events: \
         {before} messages before, {after} after"
    );
    assert!(
        f.count("SELECT count(*) FROM deliveries WHERE state = 'dead'") >= 1,
        "the event itself did dead-letter, which is what makes this a real test"
    );
}

#[test]
fn l6_the_sweep_stays_within_its_batch_for_lifecycle_work_too() {
    let f = fixture_with(Defaults {
        retention_ms: None,
        ..Defaults::default()
    });
    for n in 0..25 {
        f.bootstrap("notify.kenny", &format!("sub{n}"));
    }
    f.clock.advance(8 * DAY);

    let report = f.engine.sweep(10).expect("a bounded sweep");
    assert_eq!(report.flagged, 10, "at most one batch per pass");
    assert!(report.more_work);
}

#[test]
fn l6_defaults_are_hub_wide_and_configurable() {
    // A hub that flags after an hour and archives after two.
    let f = fixture_with(Defaults {
        retention_ms: None,
        idle_flag_ms: 60 * 60 * 1_000,
        idle_archive_ms: 2 * 60 * 60 * 1_000,
    });
    f.bootstrap("notify.kenny", "quick");

    f.clock.advance(61 * 60 * 1_000);
    f.engine.sweep(100).expect("a sweep");
    assert_eq!(f.state_of("quick"), "flagged");

    assert_eq!(
        Policy::default().lease_ms,
        30_000,
        "per-subscription policy is untouched by hub-wide defaults"
    );
}

// ─── K11 · idle thresholds are defaults, not laws ───────────────────────────

#[test]
fn l6_a_subscription_can_set_its_own_idle_thresholds() {
    let f = fixture_with(Defaults {
        retention_ms: None,
        ..Defaults::default()
    });
    f.bootstrap("reports.monthly", "monthly-report");
    f.bootstrap("notify.kenny", "ordinary");

    // A consumer whose normal rhythm is monthly would be archived by the
    // hub-wide 30 days. It says so instead.
    f.engine
        .set_policy(
            "reports.monthly",
            "monthly-report",
            StoredPolicy {
                idle_flag_ms: Some(60 * DAY),
                idle_archive_ms: Some(180 * DAY),
                ..StoredPolicy::default()
            },
        )
        .expect("its own thresholds");

    f.clock.advance(40 * DAY);
    f.engine.sweep(100).expect("a sweep");

    assert_eq!(
        f.state_of("ordinary"),
        "archived",
        "the hub default still applies to everyone who has not overridden it"
    );
    assert_eq!(
        f.state_of("monthly-report"),
        "active",
        "while a subscription that declared a slower rhythm is left alone"
    );

    f.clock.advance(25 * DAY);
    f.engine.sweep(100).expect("a sweep");
    assert_eq!(
        f.state_of("monthly-report"),
        "flagged",
        "and its own threshold is what eventually applies"
    );
}

#[test]
fn l6_an_idle_policy_that_cannot_work_is_refused() {
    let f = fixture();
    f.bootstrap("notify.kenny", "printer");

    let error = f
        .engine
        .set_policy(
            "notify.kenny",
            "printer",
            StoredPolicy {
                idle_flag_ms: Some(30 * DAY),
                idle_archive_ms: Some(7 * DAY),
                ..StoredPolicy::default()
            },
        )
        .expect_err("archiving before flagging must be refused");
    assert!(
        format!("{error}").contains("idle_archive_ms"),
        "it names the field: {error}"
    );
    assert!(
        error.remedy().len() > 20,
        "and says what to do: {}",
        error.remedy()
    );

    let zero = f
        .engine
        .set_policy(
            "notify.kenny",
            "printer",
            StoredPolicy {
                idle_flag_ms: Some(0),
                ..StoredPolicy::default()
            },
        )
        .expect_err("a zero threshold would flag it immediately");
    assert!(format!("{zero}").contains("idle_flag_ms"));
}

#[test]
fn p7_collecting_the_events_topic_does_not_feed_itself() {
    // Retention short enough that a hub event outlives its own window
    // during this test.
    let f = fixture_with(Defaults {
        retention_ms: Some(2 * DAY),
        ..Defaults::default()
    });

    // Cause one real event: a subscription goes idle and is flagged.
    f.bootstrap("notify.kenny", "someone");
    f.clock.advance(8 * DAY);
    f.engine.sweep(1000).expect("a sweep");

    let events_on_topic = || {
        f.count(
            "SELECT count(*) FROM messages m JOIN topics t ON t.id = m.topic_id \
             WHERE t.name = 'kyu.events'",
        )
    };
    assert!(events_on_topic() > 0, "an event was published");

    // Nobody consumes the events topic, so retention is free to collect the
    // events themselves once they age out.
    f.clock.advance(10 * DAY);
    f.engine.sweep(1000).expect("a collecting sweep");
    let after_first = events_on_topic();

    // Sweep repeatedly with nothing else happening. If collecting events
    // publishes an event about the collection, the topic refills itself for
    // ever and retention never reaches quiescence.
    for _ in 0..5 {
        f.clock.advance(10 * DAY);
        f.engine.sweep(1000).expect("a sweep");
    }
    let after_repeats = events_on_topic();

    assert_eq!(
        after_first, 0,
        "once collected, the events topic should be empty — it refilled itself"
    );
    assert_eq!(
        after_repeats, 0,
        "and repeated quiet sweeps must not keep producing new events"
    );
}
