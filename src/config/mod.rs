//! Process configuration (AR6, amended 3.0.0): the hub's own settings are
//! environment variables only. The transport knobs — listen address, state
//! directory, body limit, shutdown budget, logging — moved to chassis,
//! which reads them from the same environment (and, optionally, from the
//! kit's `config.toml`). Everything per-topic or per-subscription is
//! *policy* (K7, K9) and lives in the database instead, set through the
//! API or the dashboard.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::engine::Defaults;

mod auth;
mod retired;

pub use auth::{Auth, MIN_TOKEN_LEN};
pub use retired::refuse_retired_data_dir;

/// What `--help` says beyond the kit's knobs: the hub's own environment.
pub const HELP_EXTRA: &str = "The hub's own environment (read next to the knobs above):
  KYU_TOKEN            the login token (16+ chars) — required since 3.0.0, the kit refuses to start without it
  KYU_SECRET_KEY       64 hex chars sealing app tokens and sessions; set together with KYU_TOKEN
  KYU_RETENTION_MS     default message retention in ms, or `never`
  KYU_IDLE_FLAG_MS     idle-subscription flag threshold in ms
  KYU_IDLE_ARCHIVE_MS  idle-subscription archive threshold in ms
  KYU_EXPIRED_EVENT_WINDOW_MS  one message.expired per subscription per this many ms (default a day)
  KYU_DATA_DIR         retired in 4.0.0 (the 2.x name of KYU_STATE_DIR); set, it refuses to start
Long polls (`GET /t/{topic}/next?wait=`) are exempt from the request timeout.
--check reads the store (quick_check, schema) and writes nothing: a pending migration is
reported, and applied only at start, after a snapshot.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// The state root the kit resolved (`KYU_STATE_DIR`).
    pub data_dir: PathBuf,
    /// The kit's body limit, mirrored so the publish handler can name it.
    pub max_body_bytes: u64,
    /// Hub-wide defaults for things a topic or subscription can override
    /// (AR6): retention and the idle thresholds.
    pub defaults: Defaults,
    /// Who may talk to the hub (W2).
    pub auth: Auth,
}

impl Config {
    /// The hub's own settings from the environment, on top of what the kit
    /// resolved (3.0.0). Present-but-invalid values fail loudly with a
    /// remedy (AR4, standing rules 11 and 12) rather than falling back.
    pub fn from_kit(state_dir: &Path, max_body_bytes: u64) -> Result<Self> {
        Self::from_values(
            state_dir,
            max_body_bytes,
            std::env::var("KYU_TOKEN").ok().as_deref(),
            std::env::var("KYU_SECRET_KEY").ok().as_deref(),
        )
    }

    /// `from_kit` with the door pair passed in, so tests never mutate the
    /// process environment.
    pub fn from_values(
        state_dir: &Path,
        max_body_bytes: u64,
        token: Option<&str>,
        key: Option<&str>,
    ) -> Result<Self> {
        let defaults = Defaults::default();
        let defaults = Defaults {
            retention_ms: duration_from_env("KYU_RETENTION_MS", defaults.retention_ms)?,
            idle_flag_ms: duration_from_env("KYU_IDLE_FLAG_MS", Some(defaults.idle_flag_ms))?
                .unwrap_or(defaults.idle_flag_ms),
            idle_archive_ms: duration_from_env(
                "KYU_IDLE_ARCHIVE_MS",
                Some(defaults.idle_archive_ms),
            )?
            .unwrap_or(defaults.idle_archive_ms),
            expired_event_window_ms: expired_event_window(
                std::env::var("KYU_EXPIRED_EVENT_WINDOW_MS").ok().as_deref(),
                defaults.expired_event_window_ms,
            )?,
        };
        Ok(Self {
            data_dir: state_dir.to_path_buf(),
            max_body_bytes,
            defaults,
            auth: Auth::parse(token, key)?,
        })
    }
}

/// `KYU_EXPIRED_EVENT_WINDOW_MS` (W11, amended 2026-09-18): how long one
/// `message.expired` announcement stands for a subscription. `never` is
/// refused here, unlike the other durations: a window that never closes
/// would announce the first expiry and then nothing, for ever.
fn expired_event_window(raw: Option<&str>, fallback: i64) -> Result<i64> {
    match duration_from_value("KYU_EXPIRED_EVENT_WINDOW_MS", raw, Some(fallback))? {
        Some(window) => Ok(window),
        None => bail!(
            "KYU_EXPIRED_EVENT_WINDOW_MS is \"never\", which would announce the \
             first expired message and then never another. Set a millisecond \
             count (86400000 is a day); 1 announces every sweep."
        ),
    }
}

/// Reads a millisecond duration from the environment. The literal `never`
/// means "no limit" — spelled out rather than encoded as 0, which would read
/// like "immediately" to anyone skimming the compose file.
fn duration_from_env(name: &str, fallback: Option<i64>) -> Result<Option<i64>> {
    duration_from_value(name, std::env::var(name).ok().as_deref(), fallback)
}

/// [`duration_from_env`] with the raw value passed in, so tests never
/// mutate the process environment.
fn duration_from_value(
    name: &str,
    raw: Option<&str>,
    fallback: Option<i64>,
) -> Result<Option<i64>> {
    let Some(raw) = raw else {
        return Ok(fallback);
    };
    if raw.eq_ignore_ascii_case("never") {
        return Ok(None);
    }
    let parsed: i64 = raw.parse().with_context(|| {
        format!(
            "{name} is not a whole number of milliseconds: {raw:?}. Set it to a \
             millisecond count (604800000 is a week), or to \"never\" to switch the \
             limit off entirely."
        )
    })?;
    if parsed <= 0 {
        bail!(
            "{name} is {parsed}, which is not a usable duration. Set a positive \
             millisecond count, or \"never\" to switch the limit off."
        );
    }
    Ok(Some(parsed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn w11_the_expiry_window_takes_a_count_and_refuses_never() {
        let day = 24 * 60 * 60 * 1_000;
        assert_eq!(
            expired_event_window(None, day).expect("absent means the default"),
            day
        );
        assert_eq!(
            expired_event_window(Some("3600000"), day).expect("an hour"),
            3_600_000
        );
        let error = expired_event_window(Some("never"), day)
            .expect_err("a window that never closes announces once and then nothing");
        assert!(
            format!("{error:#}").contains("86400000"),
            "carries a remedy with a usable value: {error:#}"
        );
        let error = expired_event_window(Some("0"), day).expect_err("zero is not a duration");
        assert!(format!("{error:#}").contains("positive"), "{error:#}");
    }
}
