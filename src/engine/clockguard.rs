//! A guard against a wrong clock deleting data (Kenny, 2026-11-02).
//!
//! Retention, history and dedup pruning delete by age. If the host clock
//! jumps forward (a bad RTC, a VM resumed with a drifted clock) everything
//! looks ancient and the next sweep would delete it. The guard compares
//! each sweep's clock reading with the previous one and with the newest
//! message already stored; when the clock jumped more than the allowed
//! step, or is behind a message that was already published, the sweep keeps
//! settling leases but skips every deletion by age.

use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

use super::clock::Millis;

/// The largest forward step of the clock between two sweeps that is still
/// believed: one hour. Sweeps run every second, so a bigger step means the
/// clock was set, not that time passed (a host that slept is the rare
/// exception, and skipping one round of deletions costs nothing).
pub const CLOCK_JUMP_LIMIT_MS: Millis = 60 * 60 * 1_000;

/// Why a sweep refused to delete by age.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClockSkew {
    pub reason: String,
}

/// Remembers the previous sweep's clock reading across sweeps.
#[derive(Debug, Default)]
pub struct ClockGuard {
    /// `0` until the first sweep: there is nothing to compare with yet.
    last_now: AtomicI64,
    /// Whether the last sweep was already skewed, so the persistent
    /// "behind the newest message" case warns once, not every second.
    skewed: AtomicBool,
}

impl ClockGuard {
    /// Compares `now` with the previous sweep and with `newest_published`.
    /// Warns loudly the first time a skew is seen.
    pub fn inspect(
        &self,
        now: Millis,
        max_jump_ms: Millis,
        newest_published: Option<Millis>,
    ) -> Option<ClockSkew> {
        let previous = self.last_now.swap(now, Ordering::Relaxed);
        let skew = describe(previous, now, max_jump_ms, newest_published);
        let was_skewed = self.skewed.swap(skew.is_some(), Ordering::Relaxed);
        if let Some(skew) = &skew
            && !was_skewed
        {
            tracing::warn!(
                reason = %skew.reason,
                "the clock looks wrong: this sweep skips retention, history and dedup pruning"
            );
        }
        skew
    }
}

fn describe(
    previous: Millis,
    now: Millis,
    max_jump_ms: Millis,
    newest_published: Option<Millis>,
) -> Option<ClockSkew> {
    if previous > 0 && now.saturating_sub(previous) > max_jump_ms {
        return Some(ClockSkew {
            reason: format!(
                "the clock jumped forward {} s since the previous sweep (limit {} s)",
                (now - previous) / 1_000,
                max_jump_ms / 1_000
            ),
        });
    }
    let newest = newest_published?;
    (now < newest).then(|| ClockSkew {
        reason: format!(
            "the clock is {} s behind the newest published message",
            (newest - now) / 1_000
        ),
    })
}
