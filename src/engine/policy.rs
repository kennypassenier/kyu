//! Per-subscription delivery policy (K7, G6).
//!
//! Policy belongs to the *consumer*, not to the topic: on one
//! `notify.kenny` a text-to-speech subscription can carry a ten-minute TTL
//! ("relevant now or never") while a printer keeps its messages forever.
//! Both read the same topic.
//!
//! Every field is optional in storage, where `None` means "use the
//! default" — so the defaults live here, in one place, and a fresh
//! subscription needs no configuration at all.

use crate::store::queries::StoredPolicy;

use super::clock::Millis;

/// How long a claimed message stays claimed before the sweeper may hand it
/// to someone else. Long enough for a slow consumer, short enough that a
/// crashed one is noticed quickly.
pub const DEFAULT_LEASE_MS: i64 = 30_000;

/// Delivery attempts before a message is dead-lettered, counting the first
/// one. Five failures is enough to ride out a restart without hiding a
/// payload that can never work.
pub const DEFAULT_MAX_ATTEMPTS: i64 = 5;

/// Base delay before a failed delivery is offered again.
pub const DEFAULT_BACKOFF_MS: i64 = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    pub lease_ms: i64,
    pub max_attempts: i64,
    pub backoff_ms: i64,
    /// `None` means keep trying regardless of age.
    pub ttl_ms: Option<i64>,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            lease_ms: DEFAULT_LEASE_MS,
            max_attempts: DEFAULT_MAX_ATTEMPTS,
            backoff_ms: DEFAULT_BACKOFF_MS,
            ttl_ms: None,
        }
    }
}

impl Policy {
    pub fn effective(stored: StoredPolicy) -> Self {
        let defaults = Self::default();
        Self {
            lease_ms: stored.lease_ms.unwrap_or(defaults.lease_ms),
            max_attempts: stored.max_attempts.unwrap_or(defaults.max_attempts),
            backoff_ms: stored.backoff_ms.unwrap_or(defaults.backoff_ms),
            ttl_ms: stored.ttl_ms,
        }
    }

    /// How long to wait before offering a failed delivery again.
    ///
    /// Linear rather than exponential, deliberately: `backoff_ms` times the
    /// attempt number is a schedule you can read off the dashboard and
    /// predict in your head ("1s, 2s, 3s…"), and it needs no maximum — an
    /// exponential one would need a cap, and a cap is one more invisible
    /// rule to rediscover later.
    pub fn retry_delay_ms(&self, attempts: i64) -> i64 {
        self.backoff_ms.saturating_mul(attempts.max(1))
    }

    /// Whether a message published at `published_at` has outlived this
    /// subscription's TTL by `now`.
    pub fn is_past_ttl(&self, published_at: Millis, now: Millis) -> bool {
        match self.ttl_ms {
            None => false,
            Some(ttl) => published_at.saturating_add(ttl) <= now,
        }
    }
}

/// Rejects a policy that would break delivery rather than storing it and
/// failing mysteriously later (fail-closed, standing rule 12).
pub fn validate(stored: StoredPolicy) -> Result<(), (&'static str, String)> {
    if let Some(lease) = stored.lease_ms
        && lease <= 0
    {
        return Err((
            "lease_ms",
            "a lease must be longer than zero milliseconds, or a message would be \
             handed to the next consumer the instant it is claimed"
                .to_string(),
        ));
    }
    if let Some(attempts) = stored.max_attempts
        && attempts < 1
    {
        return Err((
            "max_attempts",
            "at least one delivery attempt is needed; a value below 1 would \
             dead-letter every message without ever offering it"
                .to_string(),
        ));
    }
    if let Some(backoff) = stored.backoff_ms
        && backoff < 0
    {
        return Err((
            "backoff_ms",
            "a backoff cannot be negative; use 0 for an immediate retry".to_string(),
        ));
    }
    if let Some(ttl) = stored.ttl_ms
        && ttl <= 0
    {
        return Err((
            "ttl_ms",
            "a TTL must be longer than zero milliseconds; omit it entirely to keep \
             messages regardless of age"
                .to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l4_defaults_apply_to_a_subscription_with_no_policy() {
        let policy = Policy::effective(StoredPolicy::default());
        assert_eq!(policy.lease_ms, DEFAULT_LEASE_MS);
        assert_eq!(policy.max_attempts, DEFAULT_MAX_ATTEMPTS);
        assert_eq!(policy.backoff_ms, DEFAULT_BACKOFF_MS);
        assert_eq!(
            policy.ttl_ms, None,
            "messages are kept regardless of age by default"
        );
    }

    #[test]
    fn l4_stored_values_override_the_defaults_one_field_at_a_time() {
        let policy = Policy::effective(StoredPolicy {
            ttl_ms: Some(600_000),
            ..StoredPolicy::default()
        });
        assert_eq!(policy.ttl_ms, Some(600_000), "the TTS case from G6");
        assert_eq!(
            policy.lease_ms, DEFAULT_LEASE_MS,
            "setting one field must not disturb the others"
        );
    }

    #[test]
    fn l4_the_retry_schedule_is_readable() {
        let policy = Policy::default();
        assert_eq!(policy.retry_delay_ms(1), 1_000);
        assert_eq!(policy.retry_delay_ms(2), 2_000);
        assert_eq!(policy.retry_delay_ms(3), 3_000);
    }

    #[test]
    fn l4_ttl_is_measured_from_publication() {
        let policy = Policy {
            ttl_ms: Some(600_000),
            ..Policy::default()
        };
        let published = 1_000_000;
        assert!(!policy.is_past_ttl(published, published + 599_999));
        assert!(policy.is_past_ttl(published, published + 600_000));
    }

    #[test]
    fn l4_a_subscription_without_a_ttl_never_expires_a_message() {
        let policy = Policy::default();
        assert!(!policy.is_past_ttl(0, i64::MAX / 2));
    }

    #[test]
    fn l4_a_policy_that_would_break_delivery_is_refused() {
        for (broken, field) in [
            (
                StoredPolicy {
                    lease_ms: Some(0),
                    ..StoredPolicy::default()
                },
                "lease_ms",
            ),
            (
                StoredPolicy {
                    max_attempts: Some(0),
                    ..StoredPolicy::default()
                },
                "max_attempts",
            ),
            (
                StoredPolicy {
                    backoff_ms: Some(-1),
                    ..StoredPolicy::default()
                },
                "backoff_ms",
            ),
            (
                StoredPolicy {
                    ttl_ms: Some(0),
                    ..StoredPolicy::default()
                },
                "ttl_ms",
            ),
        ] {
            let error = validate(broken).expect_err("must be refused");
            assert_eq!(error.0, field);
            assert!(error.1.len() > 20, "the reason must explain itself");
        }
    }
}
