//! Process configuration (AR6): environment variables only, because a
//! config file would be a second place to look. Everything per-topic or
//! per-subscription is *policy* (K7, K9) and lives in the database
//! instead, set through the API or the dashboard.

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};

use crate::engine::Defaults;

pub const DEFAULT_LISTEN: &str = "0.0.0.0:8080";
pub const DEFAULT_DATA_DIR: &str = "/data";
pub const DEFAULT_MAX_BODY_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub listen: SocketAddr,
    pub data_dir: PathBuf,
    pub max_body_bytes: u64,
    /// Hub-wide defaults for things a topic or subscription can override
    /// (AR6): retention and the idle thresholds.
    pub defaults: Defaults,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let listen = std::env::var("MAILBOX_LISTEN").ok();
        let data_dir = std::env::var("MAILBOX_DATA_DIR").ok();
        let max_body = std::env::var("MAILBOX_MAX_BODY_BYTES").ok();
        let mut config = Self::parse(listen.as_deref(), data_dir.as_deref(), max_body.as_deref())?;

        config.defaults = Defaults {
            retention_ms: duration_from_env("MAILBOX_RETENTION_MS", config.defaults.retention_ms)?,
            idle_flag_ms: duration_from_env(
                "MAILBOX_IDLE_FLAG_MS",
                Some(config.defaults.idle_flag_ms),
            )?
            .unwrap_or(config.defaults.idle_flag_ms),
            idle_archive_ms: duration_from_env(
                "MAILBOX_IDLE_ARCHIVE_MS",
                Some(config.defaults.idle_archive_ms),
            )?
            .unwrap_or(config.defaults.idle_archive_ms),
        };
        Ok(config)
    }

    /// Parses configuration from raw values, so tests never have to mutate
    /// the process environment. Absent values take the documented
    /// defaults; present-but-invalid values fail loudly with a remedy
    /// (AR4, standing rules 11 and 12) rather than falling back.
    pub fn parse(
        listen: Option<&str>,
        data_dir: Option<&str>,
        max_body_bytes: Option<&str>,
    ) -> Result<Self> {
        let listen_raw = listen.unwrap_or(DEFAULT_LISTEN);
        let listen: SocketAddr = listen_raw.parse().with_context(|| {
            format!(
                "MAILBOX_LISTEN is not a socket address: {listen_raw:?}. \
                 Set it as HOST:PORT (for example 0.0.0.0:8080), \
                 or unset it to use the default {DEFAULT_LISTEN}."
            )
        })?;

        let data_dir = PathBuf::from(data_dir.unwrap_or(DEFAULT_DATA_DIR));
        if data_dir.as_os_str().is_empty() {
            bail!(
                "MAILBOX_DATA_DIR is empty. Set it to the directory holding \
                 the store (for example /data), or unset it to use the \
                 default {DEFAULT_DATA_DIR}."
            );
        }

        let max_body_bytes = match max_body_bytes {
            None => DEFAULT_MAX_BODY_BYTES,
            Some(raw) => {
                let parsed: u64 = raw.parse().with_context(|| {
                    format!(
                        "MAILBOX_MAX_BODY_BYTES is not a whole number of bytes: {raw:?}. \
                         Set it to a byte count (for example 1048576 for 1 MiB), \
                         or unset it to use the default {DEFAULT_MAX_BODY_BYTES}."
                    )
                })?;
                if parsed == 0 {
                    bail!(
                        "MAILBOX_MAX_BODY_BYTES is 0, which would reject every \
                         message. Set it to a byte count above zero (for example \
                         1048576 for 1 MiB), or unset it to use the default \
                         {DEFAULT_MAX_BODY_BYTES}."
                    );
                }
                parsed
            }
        };

        Ok(Self {
            listen,
            data_dir,
            max_body_bytes,
            defaults: Defaults::default(),
        })
    }
}

/// Reads a millisecond duration from the environment. The literal `never`
/// means "no limit" — spelled out rather than encoded as 0, which would read
/// like "immediately" to anyone skimming the compose file.
fn duration_from_env(name: &str, fallback: Option<i64>) -> Result<Option<i64>> {
    let Some(raw) = std::env::var(name).ok() else {
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
    fn l0_defaults_apply_when_nothing_is_set() {
        let config = Config::parse(None, None, None).expect("defaults must parse");
        assert_eq!(config.listen.to_string(), DEFAULT_LISTEN);
        assert_eq!(config.data_dir, PathBuf::from(DEFAULT_DATA_DIR));
        assert_eq!(config.max_body_bytes, DEFAULT_MAX_BODY_BYTES);
    }

    #[test]
    fn l0_explicit_values_are_used() {
        let config = Config::parse(Some("127.0.0.1:9000"), Some("/srv/mailbox"), Some("2048"))
            .expect("explicit values must parse");
        assert_eq!(config.listen.to_string(), "127.0.0.1:9000");
        assert_eq!(config.data_dir, PathBuf::from("/srv/mailbox"));
        assert_eq!(config.max_body_bytes, 2048);
    }

    #[test]
    fn l0_invalid_listen_fails_with_a_remedy() {
        let error = Config::parse(Some("not-an-address"), None, None)
            .expect_err("an invalid address must not fall back to the default");
        let message = format!("{error:#}");
        assert!(message.contains("MAILBOX_LISTEN"), "names the variable");
        assert!(message.contains("HOST:PORT"), "carries a remedy: {message}");
    }

    #[test]
    fn l0_invalid_max_body_fails_with_a_remedy() {
        let error = Config::parse(None, None, Some("1 MiB"))
            .expect_err("an unparseable byte count must not fall back");
        assert!(
            format!("{error:#}").contains("byte count"),
            "carries a remedy"
        );
    }

    #[test]
    fn l0_zero_max_body_is_refused() {
        let error = Config::parse(None, None, Some("0"))
            .expect_err("zero would reject every message and must be refused");
        assert!(
            format!("{error:#}").contains("above zero"),
            "carries a remedy"
        );
    }
}
