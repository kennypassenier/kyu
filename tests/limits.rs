//! The caps on topics and subscriptions: past the limit a new one is
//! refused with a remedy naming the setting, existing ones keep working.

use std::sync::Arc;

use kyu::engine::clock::SystemClock;
use kyu::engine::{Defaults, Engine, EngineError};

fn engine(defaults: Defaults) -> (Engine, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("a state dir");
    let store = kyu::storage::open(dir.path()).expect("a store");
    (
        Engine::with_defaults(store, Arc::new(SystemClock), defaults),
        dir,
    )
}

#[test]
fn a_publish_that_would_create_one_topic_too_many_is_refused() {
    let (engine, _dir) = engine(Defaults {
        max_topics: 2,
        ..Defaults::default()
    });
    engine.publish("a.one", b"x", None).expect("first topic");
    engine.publish("a.two", b"x", None).expect("second topic");
    engine
        .publish("a.one", b"y", None)
        .expect("an existing topic still takes messages");
    let refused = engine
        .publish("a.three", b"x", None)
        .expect_err("over the cap");
    assert!(
        matches!(refused, EngineError::LimitReached { limit: 2, .. }),
        "{refused}"
    );
    assert!(
        refused.remedy().contains("KYU_MAX_TOPICS"),
        "{}",
        refused.remedy()
    );
}

#[test]
fn a_poll_that_would_create_one_subscription_too_many_is_refused() {
    let (engine, _dir) = engine(Defaults {
        max_subscriptions_per_topic: 1,
        ..Defaults::default()
    });
    engine.publish("a.one", b"x", None).expect("a topic");
    engine
        .claim_next("a.one", "first", false)
        .expect("first subscription");
    engine
        .claim_next("a.one", "first", false)
        .expect("the same one polls again");
    let refused = engine
        .claim_next("a.one", "second", false)
        .expect_err("over the cap");
    assert!(
        matches!(refused, EngineError::LimitReached { limit: 1, .. }),
        "{refused}"
    );
    assert!(
        refused.remedy().contains("KYU_MAX_SUBSCRIPTIONS_PER_TOPIC"),
        "{}",
        refused.remedy()
    );
}
