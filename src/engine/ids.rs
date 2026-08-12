//! Public message identifiers (AR7): ULIDs, because they are readable in
//! curl output and sort by creation time.
//!
//! Identity only. Delivery *order* comes from the store's insertion
//! sequence, never from an id — see the module note in [`crate::store`]
//! and AR7.

use std::time::{Duration, UNIX_EPOCH};

use ulid::{Generator, Ulid};

use super::clock::Millis;

/// Hands out ids that never go backwards.
///
/// A ULID's high bits are a timestamp, so a clock that steps backwards —
/// a host booting with a drifted RTC after a power cut, then corrected by
/// NTP — would otherwise mint ids that sort before existing ones.
#[derive(Debug, Default)]
pub struct MessageIds {
    generator: Generator,
}

impl MessageIds {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns an id strictly greater than every id this generator has
    /// returned before, whatever `now_ms` does.
    pub fn next(&mut self, now_ms: Millis) -> Ulid {
        let datetime = UNIX_EPOCH + Duration::from_millis(now_ms.max(0) as u64);

        match self.generator.generate_from_datetime(datetime) {
            Ok(id) => id,
            // The random field is exhausted within this millisecond. Roll
            // into the next one instead of failing a publish: K1 promises
            // that a confirmed publish is stored, and an id is never a
            // good reason to break that.
            Err(overflow) => overflow.commit_overflow_increment(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l1_ids_increase_within_one_millisecond() {
        let mut ids = MessageIds::new();
        let first = ids.next(1_000);
        let second = ids.next(1_000);
        assert!(second > first, "{second} must sort after {first}");
    }

    #[test]
    fn l1_ids_stay_monotonic_when_the_clock_steps_backwards() {
        let mut ids = MessageIds::new();

        let before_outage = ids.next(1_700_000_000_000);
        // The host reboots with a dead RTC, then NTP corrects it: an hour
        // of apparent time travel backwards.
        let after_outage = ids.next(1_700_000_000_000 - 3_600_000);

        assert!(
            after_outage > before_outage,
            "an id minted after a backwards clock step ({after_outage}) must \
             still sort after the previous one ({before_outage})"
        );
        assert!(
            after_outage.to_string() > before_outage.to_string(),
            "the textual form must sort the same way, since that is what \
             appears in URLs and logs"
        );
    }

    #[test]
    fn l1_ids_round_trip_through_their_text_form() {
        let mut ids = MessageIds::new();
        let id = ids.next(1_700_000_000_000);
        let parsed: Ulid = id.to_string().parse().expect("a ULID must parse back");
        assert_eq!(parsed, id);
    }

    #[test]
    fn l1_a_thousand_ids_are_strictly_increasing_on_a_frozen_clock() {
        let mut ids = MessageIds::new();
        let mut previous = ids.next(5_000);
        for _ in 0..1_000 {
            let next = ids.next(5_000);
            assert!(next > previous, "{next} must sort after {previous}");
            previous = next;
        }
    }
}
