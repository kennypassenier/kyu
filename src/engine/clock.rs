//! Injected time (AR7). Nothing in the engine reads the wall clock
//! directly: a lease that expires after 30 seconds, a TTL of 10 minutes
//! and a retention window of 7 days all have to be testable in
//! milliseconds, not by waiting.

use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Milliseconds since the Unix epoch, UTC — the single time
/// representation shared by the engine and the store (AR7). SQLite holds
/// these as INTEGER.
pub type Millis = i64;

pub trait Clock: Send + Sync + 'static {
    fn now_ms(&self) -> Millis;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> Millis {
        // A pre-1970 reading means a dead RTC after a long power cut. It is
        // not fatal: delivery order comes from insertion order, not from
        // this value (AR7), so report the epoch instead of panicking.
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis() as Millis)
            .unwrap_or(0)
    }
}

/// Test clock: time moves only when a test moves it — including
/// *backwards*, which is the case AR7 exists for.
#[derive(Debug)]
pub struct MockClock {
    now_ms: AtomicI64,
}

impl MockClock {
    pub fn new(start_ms: Millis) -> Self {
        Self {
            now_ms: AtomicI64::new(start_ms),
        }
    }

    pub fn advance(&self, delta_ms: Millis) {
        self.now_ms.fetch_add(delta_ms, Ordering::SeqCst);
    }

    /// Jumps to an arbitrary point, earlier ones included: after a power
    /// cut a host can boot with a drifted RTC and have NTP step it back.
    pub fn set(&self, now_ms: Millis) {
        self.now_ms.store(now_ms, Ordering::SeqCst);
    }
}

impl Clock for MockClock {
    fn now_ms(&self) -> Millis {
        self.now_ms.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l1_mock_clock_moves_only_when_told() {
        let clock = MockClock::new(1_000);
        assert_eq!(clock.now_ms(), 1_000);
        assert_eq!(clock.now_ms(), 1_000, "reading time must not advance it");

        clock.advance(500);
        assert_eq!(clock.now_ms(), 1_500);
    }

    #[test]
    fn l1_mock_clock_can_step_backwards() {
        let clock = MockClock::new(10_000);
        clock.set(4_000);
        assert_eq!(clock.now_ms(), 4_000);
    }

    #[test]
    fn l1_system_clock_is_after_2020() {
        // Sanity only: proves the epoch conversion is not off by orders of
        // magnitude (seconds mistaken for millis, say).
        let after_2020_ms = 1_577_836_800_000;
        assert!(SystemClock.now_ms() > after_2020_ms);
    }
}
