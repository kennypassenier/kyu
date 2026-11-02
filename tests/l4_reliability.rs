//! [L4] The delivery state machine (AR9) driven by a mock clock, so leases,
//! backoff and TTL are tested in milliseconds instead of by waiting.
//!
//! Every transition AR9 pins down has a test named after it here. The
//! engine is used directly: these are statements about semantics, and the
//! HTTP surface is exercised separately in `l4_reliability_http`.

use std::sync::Arc;

use chassis::storage::AppDb;
#[path = "common/db.rs"]
mod db;
use db::RawStore;
use kyu::engine::clock::MockClock;
use kyu::engine::policy::{DEFAULT_MAX_ATTEMPTS, Policy};
use kyu::engine::{Engine, Settled};
use kyu::store::queries::StoredPolicy;

const START: i64 = 1_700_000_000_000;

struct Fixture {
    engine: Engine,
    clock: Arc<MockClock>,
    store: AppDb,
}

fn fixture() -> Fixture {
    let store = kyu::storage::open_in_memory().expect("an in-memory store");
    let clock = Arc::new(MockClock::new(START));
    let engine = Engine::new(store.clone(), clock.clone());
    Fixture {
        engine,
        clock,
        store,
    }
}

impl Fixture {
    /// Publishes once so the topic exists, then polls so the subscription
    /// does (G7). Returns nothing: that first poll is empty by design.
    fn bootstrap(&self, topic: &str, subscription: &str) {
        self.engine
            .publish(topic, b"{}", Some("application/json"))
            .expect("the bootstrap publish");
        let received = self
            .engine
            .claim_next(topic, subscription, false)
            .expect("the first poll");
        assert!(received.claimed.is_none());
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

    fn state_of(&self, message_id: &str, subscription: &str) -> String {
        self.store.with_conn(|conn| {
            conn.query_row(
                "SELECT d.state
                   FROM deliveries d
                   JOIN messages m ON m.seq = d.msg_seq
                   JOIN subscriptions s ON s.id = d.sub_id
                  WHERE m.id = ?1 AND s.name = ?2",
                (message_id, subscription),
                |row| row.get::<_, String>(0),
            )
            .expect("the delivery must exist")
        })
    }

    fn attempts_of(&self, message_id: &str, subscription: &str) -> i64 {
        self.store.with_conn(|conn| {
            conn.query_row(
                "SELECT d.attempts
                   FROM deliveries d
                   JOIN messages m ON m.seq = d.msg_seq
                   JOIN subscriptions s ON s.id = d.sub_id
                  WHERE m.id = ?1 AND s.name = ?2",
                (message_id, subscription),
                |row| row.get::<_, i64>(0),
            )
            .expect("the delivery must exist")
        })
    }
}

// ─── AR9: pending -> claimed -> acked ───────────────────────────────────────

#[test]
fn l4_ar9_pending_to_claimed_to_acked() {
    let f = fixture();
    f.bootstrap("notify.kenny", "printer");
    let id = f.publish("notify.kenny", r#"{"n":1}"#);

    assert_eq!(f.state_of(&id, "printer"), "pending");
    assert_eq!(
        f.claim("notify.kenny", "printer").as_deref(),
        Some(id.as_str())
    );
    assert_eq!(f.state_of(&id, "printer"), "claimed");

    f.engine
        .ack("notify.kenny", "printer", &id)
        .expect("the ack");
    assert_eq!(f.state_of(&id, "printer"), "acked");
}

// ─── AR9: claimed -> pending on lease expiry ────────────────────────────────

#[test]
fn l4_ar9_claimed_to_pending_when_the_lease_expires() {
    let f = fixture();
    f.bootstrap("jobs.transcode", "worker");
    let id = f.publish("jobs.transcode", r#"{"job":1}"#);
    f.claim("jobs.transcode", "worker");

    // The worker has died. Nothing happens until the lease runs out.
    f.clock.advance(Policy::default().lease_ms - 1);
    let report = f.engine.sweep(100).expect("a sweep");
    assert_eq!(report.changed(), 0, "the lease has not expired yet");
    assert_eq!(f.state_of(&id, "worker"), "claimed");

    f.clock.advance(1);
    let report = f.engine.sweep(100).expect("a sweep");
    assert_eq!(report.redelivered, 1);
    assert_eq!(f.state_of(&id, "worker"), "pending");
    assert_eq!(
        f.attempts_of(&id, "worker"),
        1,
        "the failed attempt is counted"
    );
    assert_eq!(
        report.wake,
        vec![("jobs.transcode".to_string(), "worker".to_string())],
        "the subscription is woken so a waiting poll answers at once"
    );
}

#[test]
fn l4_a_redelivered_message_waits_out_its_backoff_first() {
    let f = fixture();
    f.bootstrap("jobs.transcode", "worker");
    let id = f.publish("jobs.transcode", r#"{"job":1}"#);
    f.claim("jobs.transcode", "worker");

    f.clock.advance(Policy::default().lease_ms);
    f.engine.sweep(100).expect("a sweep");

    // Backoff after one failure is one second by default.
    assert_eq!(
        f.claim("jobs.transcode", "worker"),
        None,
        "a message inside its backoff window is not offered again yet"
    );
    f.clock.advance(1_000);
    assert_eq!(
        f.claim("jobs.transcode", "worker").as_deref(),
        Some(id.as_str()),
        "and it is offered once the backoff has passed"
    );
}

// ─── AR9: pending -> dead when the attempts run out ─────────────────────────

#[test]
fn l4_ar9_pending_to_dead_when_attempts_run_out() {
    let f = fixture();
    f.bootstrap("print.receipt", "printer");
    let id = f.publish("print.receipt", r#"{"receipt":1}"#);

    // Claim and abandon until the attempts are gone.
    for attempt in 1..=DEFAULT_MAX_ATTEMPTS {
        let claimed = f.claim("print.receipt", "printer");
        assert_eq!(
            claimed.as_deref(),
            Some(id.as_str()),
            "attempt {attempt} must be offered"
        );
        f.clock.advance(Policy::default().lease_ms);
        f.engine.sweep(100).expect("a sweep");
        f.clock.advance(Policy::default().retry_delay_ms(attempt));
    }

    assert_eq!(
        f.state_of(&id, "printer"),
        "dead",
        "after {DEFAULT_MAX_ATTEMPTS} failed attempts the message is dead-lettered"
    );
    assert_eq!(f.claim("print.receipt", "printer"), None);

    let dead = f
        .engine
        .dead_letters("print.receipt", "printer", 10)
        .expect("the dead-letter list");
    assert_eq!(dead.len(), 1);
    assert_eq!(dead[0].id, id);
    assert_eq!(dead[0].attempts, DEFAULT_MAX_ATTEMPTS);
    assert!(dead[0].dead_at.is_some(), "the moment it died is recorded");
}

// ─── AR9: dead -> pending on requeue ────────────────────────────────────────

#[test]
fn l4_ar9_dead_to_pending_on_requeue_with_attempts_reset() {
    let f = fixture();
    f.bootstrap("print.receipt", "printer");
    let id = f.publish("print.receipt", r#"{"receipt":1}"#);

    // Straight to the dead list via a poison-pill nack (W5).
    f.claim("print.receipt", "printer");
    f.engine
        .nack("print.receipt", "printer", &id, true)
        .expect("the nack");
    assert_eq!(f.state_of(&id, "printer"), "dead");

    f.engine
        .requeue_dead("print.receipt", "printer", &id)
        .expect("the requeue");
    assert_eq!(f.state_of(&id, "printer"), "pending");
    assert_eq!(
        f.attempts_of(&id, "printer"),
        0,
        "a requeued message gets a full set of attempts, because the reason \
         it failed has usually been fixed by hand in between"
    );
    assert_eq!(
        f.claim("print.receipt", "printer").as_deref(),
        Some(id.as_str())
    );
}

// ─── AR9: pending -> expired past the TTL ───────────────────────────────────

#[test]
fn l4_ar9_pending_to_expired_past_the_ttl() {
    let f = fixture();
    f.bootstrap("speak.kenny_pc", "tts");
    f.engine
        .set_policy(
            "speak.kenny_pc",
            "tts",
            StoredPolicy {
                ttl_ms: Some(600_000),
                ..StoredPolicy::default()
            },
        )
        .expect("the policy");

    let id = f.publish("speak.kenny_pc", r#"{"say":"de was is klaar"}"#);

    f.clock.advance(599_999);
    assert_eq!(f.engine.sweep(100).expect("a sweep").expired, 0);

    f.clock.advance(1);
    let report = f.engine.sweep(100).expect("a sweep");
    assert_eq!(report.expired, 1);
    assert_eq!(f.state_of(&id, "tts"), "expired");
    assert_eq!(
        f.claim("speak.kenny_pc", "tts"),
        None,
        "an expired message is settled, not delivered late"
    );
}

// ─── AR9 amendment: TTL is re-checked when a delivery returns to pending ────

#[test]
fn l4_a_stale_message_expires_on_re_pend_instead_of_being_delivered_late() {
    let f = fixture();
    f.bootstrap("speak.kenny_pc", "tts");
    f.engine
        .set_policy(
            "speak.kenny_pc",
            "tts",
            StoredPolicy {
                ttl_ms: Some(600_000),
                lease_ms: Some(1_800_000),
                ..StoredPolicy::default()
            },
        )
        .expect("the policy");

    let id = f.publish("speak.kenny_pc", r#"{"say":"er staat iemand aan de deur"}"#);
    f.claim("speak.kenny_pc", "tts");

    // The worker hangs for half an hour, well past the ten-minute TTL, and
    // only then does its lease run out.
    f.clock.advance(1_800_000);
    let report = f.engine.sweep(100).expect("a sweep");

    assert_eq!(
        report.expired, 1,
        "the doorbell has nothing to announce now"
    );
    assert_eq!(report.redelivered, 0);
    assert_eq!(f.state_of(&id, "tts"), "expired");
    assert_eq!(
        f.claim("speak.kenny_pc", "tts"),
        None,
        "without the re-pend check this would announce a half-hour-old doorbell"
    );
}

// ─── W5 · nack ──────────────────────────────────────────────────────────────

#[test]
fn l4_a_nack_returns_the_message_without_waiting_for_the_lease() {
    let f = fixture();
    f.bootstrap("jobs.transcode", "worker");
    let id = f.publish("jobs.transcode", r#"{"job":1}"#);
    f.claim("jobs.transcode", "worker");

    let settled = f
        .engine
        .nack("jobs.transcode", "worker", &id, false)
        .expect("the nack");

    assert_eq!(settled, Settled::Redelivered);
    assert_eq!(f.state_of(&id, "worker"), "pending");
    assert_eq!(f.attempts_of(&id, "worker"), 1);
    // The lease had not come close to running out.
    assert!(Policy::default().lease_ms > 1_000);
}

#[test]
fn l4_a_poison_pill_nack_skips_the_remaining_attempts() {
    let f = fixture();
    f.bootstrap("jobs.transcode", "worker");
    let id = f.publish("jobs.transcode", r#"not json at all"#);
    f.claim("jobs.transcode", "worker");

    let settled = f
        .engine
        .nack("jobs.transcode", "worker", &id, true)
        .expect("the nack");

    assert_eq!(settled, Settled::DeadLettered);
    assert_eq!(
        f.state_of(&id, "worker"),
        "dead",
        "a payload that can never work should not spend four more attempts"
    );
}

#[test]
fn l4_nacking_a_message_that_is_not_claimed_is_refused() {
    let f = fixture();
    f.bootstrap("jobs.transcode", "worker");
    let id = f.publish("jobs.transcode", r#"{"job":1}"#);

    let error = f
        .engine
        .nack("jobs.transcode", "worker", &id, false)
        .expect_err("nacking an unclaimed message must fail");
    assert!(format!("{error}").contains("not claimed"));
    assert!(error.remedy().len() > 20);
}

// ─── K7 · policy ────────────────────────────────────────────────────────────

#[test]
fn l4_a_fresh_subscription_runs_on_the_defaults() {
    let f = fixture();
    f.bootstrap("notify.kenny", "printer");

    let (effective, explicit) = f
        .engine
        .policy("notify.kenny", "printer")
        .expect("a policy");
    assert_eq!(effective, Policy::default());
    assert_eq!(
        explicit,
        StoredPolicy::default(),
        "nothing is stored until someone sets it, so defaults can move later"
    );
}

#[test]
fn l4_policy_is_per_subscription_not_per_topic() {
    let f = fixture();
    f.bootstrap("notify.kenny", "tts");
    f.bootstrap("notify.kenny", "printer");

    // G6's worked example: two consumers of one topic, opposite needs.
    f.engine
        .set_policy(
            "notify.kenny",
            "tts",
            StoredPolicy {
                ttl_ms: Some(600_000),
                ..StoredPolicy::default()
            },
        )
        .expect("the TTS policy");

    let id = f.publish("notify.kenny", r#"{"title":"de was is klaar"}"#);
    f.clock.advance(600_000);
    f.engine.sweep(100).expect("a sweep");

    assert_eq!(
        f.state_of(&id, "tts"),
        "expired",
        "the TTS subscription treats it as relevant now or never"
    );
    assert_eq!(
        f.state_of(&id, "printer"),
        "pending",
        "while the printer's copy of the same message waits indefinitely"
    );
}

#[test]
fn l4_a_policy_write_replaces_every_field() {
    let f = fixture();
    f.bootstrap("notify.kenny", "printer");

    f.engine
        .set_policy(
            "notify.kenny",
            "printer",
            StoredPolicy {
                ttl_ms: Some(600_000),
                max_attempts: Some(9),
                ..StoredPolicy::default()
            },
        )
        .expect("the first policy");

    // A second write that omits ttl_ms puts it back to the default, rather
    // than leaving the previous value in place.
    let effective = f
        .engine
        .set_policy(
            "notify.kenny",
            "printer",
            StoredPolicy {
                max_attempts: Some(9),
                ..StoredPolicy::default()
            },
        )
        .expect("the second policy");

    assert_eq!(effective.max_attempts, 9);
    assert_eq!(
        effective.ttl_ms, None,
        "omitted means default, not unchanged"
    );
}

// ─── AR5 · the sweep stays inside its batch ─────────────────────────────────

#[test]
fn l4_a_sweep_never_exceeds_its_batch_bound() {
    let f = fixture();
    f.bootstrap("jobs.transcode", "worker");

    for n in 0..25 {
        f.publish("jobs.transcode", &format!(r#"{{"job":{n}}}"#));
    }
    for _ in 0..25 {
        assert!(f.claim("jobs.transcode", "worker").is_some());
    }

    f.clock.advance(Policy::default().lease_ms);

    let report = f.engine.sweep(10).expect("a bounded sweep");
    assert_eq!(
        report.changed(),
        10,
        "a sweep settles at most its batch and no more, so the single writer \
         is never held for a whole table (AR5)"
    );
    assert!(
        report.more_work,
        "and it says there is more to do, so the caller comes straight back"
    );

    let second = f.engine.sweep(10).expect("a bounded sweep");
    assert_eq!(second.changed(), 10);
    let third = f.engine.sweep(10).expect("a bounded sweep");
    assert_eq!(third.changed(), 5);
    assert!(
        !third.more_work,
        "and stops asking once the backlog is clear"
    );
}

// ─── The guard: a race must not undo an ack ─────────────────────────────────

#[test]
fn l4_an_ack_that_lands_during_a_sweep_wins() {
    let f = fixture();
    f.bootstrap("jobs.transcode", "worker");
    let id = f.publish("jobs.transcode", r#"{"job":1}"#);
    f.claim("jobs.transcode", "worker");

    // The lease has expired, but the consumer was slow rather than dead and
    // its ack arrives first. The state-guarded update must leave it acked.
    f.clock.advance(Policy::default().lease_ms);
    f.engine
        .ack("jobs.transcode", "worker", &id)
        .expect("the late ack");

    let report = f.engine.sweep(100).expect("a sweep");
    assert_eq!(report.changed(), 0);
    assert_eq!(
        f.state_of(&id, "worker"),
        "acked",
        "an acked message must never be resurrected by the sweeper"
    );
}

// ─── Announcements: a nack settles like a sweep and says so ─────────────────

impl Fixture {
    /// How many hub events of `kind` sit on `kyu.events`.
    fn events_of(&self, kind: &str) -> i64 {
        self.store.with_conn(|conn| {
            conn.query_row(
                "SELECT count(*) FROM messages m JOIN topics t ON t.id = m.topic_id
                  WHERE t.name = 'kyu.events'
                    AND CAST(m.payload AS TEXT) LIKE ?1",
                [format!("%\"{kind}\"%")],
                |row| row.get::<_, i64>(0),
            )
            .expect("an event count")
        })
    }

    fn unannounced_expiries(&self, subscription: &str) -> i64 {
        self.store.with_conn(|conn| {
            conn.query_row(
                "SELECT expired_unannounced FROM subscriptions WHERE name = ?1",
                [subscription],
                |row| row.get::<_, i64>(0),
            )
            .expect("the subscription must exist")
        })
    }
}

#[test]
fn l4_a_nack_to_the_dead_letters_is_announced_like_a_sweep_would() {
    let f = fixture();
    f.bootstrap("jobs.transcode", "worker");
    let id = f.publish("jobs.transcode", "not json");
    f.claim("jobs.transcode", "worker");

    f.engine
        .nack("jobs.transcode", "worker", &id, true)
        .expect("the nack");

    assert_eq!(f.events_of("message.dead_lettered"), 1);
}

#[test]
fn l4_a_nack_on_the_last_attempt_is_announced_as_dead_lettered() {
    let f = fixture();
    f.bootstrap("jobs.transcode", "worker");
    f.engine
        .set_policy(
            "jobs.transcode",
            "worker",
            StoredPolicy {
                max_attempts: Some(1),
                ..StoredPolicy::default()
            },
        )
        .expect("the policy");
    let id = f.publish("jobs.transcode", r#"{"job":1}"#);
    f.claim("jobs.transcode", "worker");

    let settled = f
        .engine
        .nack("jobs.transcode", "worker", &id, false)
        .expect("the nack");

    assert_eq!(settled, Settled::DeadLettered);
    assert_eq!(f.events_of("message.dead_lettered"), 1);
}

#[test]
fn l4_a_nack_past_the_ttl_counts_towards_the_expiry_announcement() {
    let f = fixture();
    f.bootstrap("speak.kenny_pc", "tts");
    f.engine
        .set_policy(
            "speak.kenny_pc",
            "tts",
            StoredPolicy {
                ttl_ms: Some(600_000),
                lease_ms: Some(1_800_000),
                ..StoredPolicy::default()
            },
        )
        .expect("the policy");
    let id = f.publish("speak.kenny_pc", r#"{"say":"hallo"}"#);
    f.claim("speak.kenny_pc", "tts");
    f.clock.advance(700_000);

    let settled = f
        .engine
        .nack("speak.kenny_pc", "tts", &id, false)
        .expect("the nack");

    assert_eq!(settled, Settled::Expired);
    assert_eq!(f.unannounced_expiries("tts"), 1);
    f.engine.sweep(100).expect("a sweep");
    assert_eq!(f.events_of("message.expired"), 1);
}

// ─── A message past its TTL is never claimed, even before the sweeper runs ──

#[test]
fn l4_a_message_past_its_ttl_cannot_be_claimed_before_the_sweeper_expires_it() {
    let f = fixture();
    f.bootstrap("speak.kenny_pc", "tts");
    f.engine
        .set_policy(
            "speak.kenny_pc",
            "tts",
            StoredPolicy {
                ttl_ms: Some(600_000),
                ..StoredPolicy::default()
            },
        )
        .expect("the policy");
    let id = f.publish("speak.kenny_pc", r#"{"say":"hallo"}"#);

    f.clock.advance(599_999);
    assert_eq!(f.claim("speak.kenny_pc", "tts"), Some(id.clone()));
    f.engine
        .nack("speak.kenny_pc", "tts", &id, false)
        .expect("the nack puts it back");

    f.clock.advance(60_001);
    assert_eq!(
        f.claim("speak.kenny_pc", "tts"),
        None,
        "no sweep has run, yet the stale message must not be handed out"
    );
}

// ─── Policy bounds: lease and TTL are added to timestamps in SQL ────────────

#[test]
fn l4_a_lease_or_ttl_that_would_overflow_a_timestamp_is_refused() {
    let f = fixture();
    f.bootstrap("jobs.transcode", "worker");
    for (field, policy) in [
        (
            "lease_ms",
            StoredPolicy {
                lease_ms: Some(i64::MAX),
                ..StoredPolicy::default()
            },
        ),
        (
            "ttl_ms",
            StoredPolicy {
                ttl_ms: Some(i64::MAX),
                ..StoredPolicy::default()
            },
        ),
    ] {
        let error = f
            .engine
            .set_policy("jobs.transcode", "worker", policy)
            .expect_err("must be refused");
        assert!(
            matches!(&error, kyu::engine::EngineError::InvalidPolicy { field: got, .. } if *got == field),
            "{field}: {error:?}"
        );
    }
}

// ─── The sweeper heartbeat reports a failing sweep ──────────────────────────

#[test]
fn l4_a_sweeper_that_keeps_failing_reports_failing_with_the_last_error() {
    use chassis::Subsystem;
    use kyu::kit::SweeperSubsystem;
    use kyu::sweeper::{FAILING_AFTER, Heartbeat};

    let f = fixture();
    let heartbeat = Heartbeat::starting_at(f.engine.now_ms());
    let subsystem = SweeperSubsystem {
        engine: Arc::new(Engine::new(f.store.clone(), f.clock.clone())),
        heartbeat: heartbeat.clone(),
    };

    for n in 1..FAILING_AFTER {
        heartbeat.record_failure(format!("disk on fire {n}"));
        assert!(subsystem.check().ok, "a single failure is not yet an alarm");
    }
    heartbeat.record_failure("disk on fire last".to_string());
    let status = subsystem.check();
    assert!(!status.ok);
    assert!(
        status.detail.contains("disk on fire last"),
        "{}",
        status.detail
    );

    heartbeat.beat(f.engine.now_ms());
    assert!(subsystem.check().ok, "a successful pass ends the streak");
}

// ─── TTL starts when the message is due, not when it was published ──────────

#[test]
fn l4_a_delayed_message_gets_its_full_ttl_from_the_moment_it_is_due() {
    let f = fixture();
    f.bootstrap("speak.kenny_pc", "tts");
    f.engine
        .set_policy(
            "speak.kenny_pc",
            "tts",
            StoredPolicy {
                ttl_ms: Some(600_000),
                ..StoredPolicy::default()
            },
        )
        .expect("the policy");
    let hour = 3_600_000;
    let id = f
        .engine
        .publish_due("speak.kenny_pc", b"{}", None, Some(START + hour))
        .expect("a delayed publish")
        .id;

    // An hour after publication the message is due; counted from
    // publication it would already be six times past its TTL.
    f.clock.advance(hour + 599_999);
    assert_eq!(f.engine.sweep(100).expect("a sweep").expired, 0);
    assert_eq!(f.claim("speak.kenny_pc", "tts"), Some(id.clone()));

    f.engine
        .nack("speak.kenny_pc", "tts", &id, false)
        .expect("the nack");
    f.clock.advance(1);
    assert_eq!(f.engine.sweep(100).expect("a sweep").expired, 1);
    assert_eq!(f.state_of(&id, "tts"), "expired");
}

// ─── Retention keeps a message while it is a dead letter ────────────────────

const DAY: i64 = 86_400_000;

#[test]
fn l4_retention_never_collects_a_message_that_is_still_a_dead_letter() {
    let f = fixture();
    f.bootstrap("jobs.transcode", "worker");
    let id = f.publish("jobs.transcode", "not json");
    f.claim("jobs.transcode", "worker");
    f.engine
        .nack("jobs.transcode", "worker", &id, true)
        .expect("the nack");

    f.clock.advance(8 * DAY);
    f.engine.sweep(100).expect("a sweep");
    let letters = f
        .engine
        .dead_letters("jobs.transcode", "worker", 10)
        .expect("the dead letters");
    assert_eq!(letters.len(), 1, "the dead letter keeps its payload");

    f.engine
        .delete_delivery("jobs.transcode", "worker", &id)
        .expect("the delete");
    f.clock.advance(1_000);
    assert!(
        f.engine.sweep(100).expect("a sweep").collected >= 1,
        "once nothing needs the message, retention takes it"
    );
}

// ─── A wrong clock must not delete by age ───────────────────────────────────

#[test]
fn l4_a_clock_jump_skips_retention_for_that_sweep_and_says_so() {
    use chassis::Subsystem;
    use kyu::kit::SweeperSubsystem;
    use kyu::sweeper::Heartbeat;

    let f = fixture();
    f.bootstrap("jobs.transcode", "worker");
    let id = f.publish("jobs.transcode", r#"{"job":1}"#);
    f.claim("jobs.transcode", "worker");
    f.engine
        .ack("jobs.transcode", "worker", &id)
        .expect("the ack");
    assert!(
        f.engine
            .sweep(100)
            .expect("a first sweep")
            .clock_skew
            .is_none()
    );

    // A year forward in one step: everything now looks ancient.
    f.clock.advance(365 * DAY);
    let jumped = f.engine.sweep(100).expect("the sweep after the jump");
    assert_eq!(jumped.collected, 0, "no deletion on a clock nobody trusts");
    let reason = jumped.clock_skew.expect("the jump is reported");
    assert!(reason.contains("jumped forward"), "{reason}");

    f.clock.advance(1_000);
    let settled = f.engine.sweep(100).expect("the next sweep");
    assert!(settled.clock_skew.is_none());
    assert!(settled.collected >= 1, "a sane step resumes the collection");

    let heartbeat = Heartbeat::starting_at(f.engine.now_ms());
    let subsystem = SweeperSubsystem {
        engine: Arc::new(Engine::new(f.store.clone(), f.clock.clone())),
        heartbeat: heartbeat.clone(),
    };
    assert!(subsystem.check().ok);
    heartbeat.note_clock_skew(f.engine.now_ms(), reason.clone());
    let status = subsystem.check();
    assert!(!status.ok);
    assert!(status.detail.contains(&reason), "{}", status.detail);
}

#[test]
fn l4_a_clock_behind_the_newest_message_skips_retention() {
    let f = fixture();
    f.bootstrap("jobs.transcode", "worker");
    f.publish("jobs.transcode", r#"{"job":1}"#);
    f.clock.advance(-60_000);
    let report = f.engine.sweep(100).expect("a sweep");
    let reason = report.clock_skew.expect("the skew is reported");
    assert!(reason.contains("behind the newest"), "{reason}");
}
