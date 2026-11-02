//! An empty poll is a read; the poll time is written at most every 30 s.

use std::sync::Arc;

use kyu::engine::Engine;
use kyu::engine::clock::MockClock;

const START: i64 = 1_700_000_000_000;

fn last_poll(engine: &Engine) -> i64 {
    engine
        .read(|conn| {
            Ok(conn.query_row(
                "SELECT last_poll_at FROM subscriptions WHERE name = 'worker'",
                [],
                |row| row.get(0),
            )?)
        })
        .expect("a read")
}

#[test]
fn empty_polls_write_the_poll_time_at_most_every_thirty_seconds() {
    let store = kyu::storage::open_in_memory().expect("a store");
    let clock = Arc::new(MockClock::new(START));
    let engine = Engine::new(store, clock.clone());
    engine.publish("jobs", b"{}", None).expect("a topic");
    engine
        .poll_next_as("jobs", "worker", false, None)
        .expect("creates the subscription");
    assert_eq!(last_poll(&engine), START);

    clock.advance(10_000);
    let idle = engine
        .poll_next_as("jobs", "worker", false, None)
        .expect("an empty poll");
    assert!(idle.claimed.is_none());
    assert_eq!(last_poll(&engine), START, "10 s later nothing was written");

    clock.advance(25_000);
    engine
        .poll_next_as("jobs", "worker", false, None)
        .expect("an empty poll");
    assert_eq!(
        last_poll(&engine),
        START + 35_000,
        "past 30 s the time moves"
    );

    engine
        .publish("jobs", b"{\"n\":1}", None)
        .expect("a message");
    let claimed = engine
        .poll_next_as("jobs", "worker", false, None)
        .expect("a poll");
    assert!(
        claimed.claimed.is_some(),
        "a waiting message is still claimed"
    );
}
