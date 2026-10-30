//! Coarse, human ages for timestamps on the dashboard (W14).

use crate::engine::clock::Millis;

/// A coarse age. The rest of the dashboard prints raw millisecond stamps;
/// for "when did I create this token" the only question anyone actually has
/// is how long ago, and a 13-digit number does not answer it.
pub fn human_age(now: Millis, then: Millis) -> String {
    let elapsed = (now - then).max(0);
    let minutes = elapsed / 60_000;
    let hours = minutes / 60;
    let days = hours / 24;
    match (days, hours, minutes) {
        (0, 0, 0) => "just now".to_string(),
        (0, 0, 1) => "1 minute ago".to_string(),
        (0, 0, m) => format!("{m} minutes ago"),
        (0, 1, _) => "1 hour ago".to_string(),
        (0, h, _) => format!("{h} hours ago"),
        (1, _, _) => "yesterday".to_string(),
        (d, _, _) => format!("{d} days ago"),
    }
}
