//! [L4] The delivery state machine (AR9) driven by a mock clock, so leases,
//! backoff and TTL are tested in milliseconds instead of by waiting.
//!
//! Every transition AR9 pins down has a test named after it here. The
//! engine is used directly: these are statements about semantics, and the
//! HTTP surface is exercised separately in `l4_reliability_http`.

use std::sync::Arc;

use mailbox::engine::clock::MockClock;
use mailbox::engine::policy::{DEFAULT_MAX_ATTEMPTS, Policy};
use mailbox::engine::{Engine, Settled};
use mailbox::store::Store;
use mailbox::store::queries::StoredPolicy;

const START: i64 = 1_700_000_000_000;

struct Fixture {
    engine: Engine,
    clock: Arc<MockClock>,
    store: Arc<Store>,
}

fn fixture() -> Fixture {
    let store = Arc::new(Store::open_in_memory().expect("an in-memory store"));
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
