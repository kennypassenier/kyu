//! Process configuration (AR6): environment variables only, because a
//! config file would be a second place to look. Everything per-topic or
//! per-subscription is *policy* (K7, K9) and lives in the database
//! instead, set through the API or the dashboard.

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};

use crate::crypto::SecretKey;
use crate::engine::Defaults;

pub const DEFAULT_LISTEN: &str = "0.0.0.0:8080";
pub const DEFAULT_DATA_DIR: &str = "/data";
pub const DEFAULT_MAX_BODY_BYTES: u64 = 1024 * 1024;
/// Shortest bootstrap token we accept. Long enough that a guess is not a
/// realistic attack, short enough to type once into a compose file.
pub const MIN_TOKEN_LEN: usize = 16;
/// How long the dashboard reveals a token before hiding it again (W2,
/// Kenny's choice at the 2026-08-28 mini-round).
pub const REVEAL_SECONDS: u32 = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub listen: SocketAddr,
    pub data_dir: PathBuf,
    pub max_body_bytes: u64,
    /// Hub-wide defaults for things a topic or subscription can override
    /// (AR6): retention and the idle thresholds.
    pub defaults: Defaults,
    /// Who may talk to the hub (W2).
    pub auth: Auth,
}

/// The door policy (W2).
///
/// Two states, and the difference between them is deliberately visible
/// everywhere: `Unprotected` is a legitimate deployment choice for a hub
/// nobody else can reach, but it must never be something you *think* you
/// left behind. It warns on every startup and banners every dashboard page.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Auth {
    /// No token configured. Anyone who can reach the hub can use it.
    #[default]
    Unprotected,
    /// A bootstrap token from the environment, plus the key that app tokens
    /// are encrypted with (AR11 amendment).
    Protected { token: String, key: SecretKey },
}

impl Auth {
    pub fn is_protected(&self) -> bool {
        matches!(self, Self::Protected { .. })
    }

    pub fn key(&self) -> Option<&SecretKey> {
        match self {
            Self::Protected { key, .. } => Some(key),
            Self::Unprotected => None,
        }
    }

    /// The bootstrap token, which is what you log in with and what the
    /// dashboard falls back to when no app is selected.
    pub fn token(&self) -> Option<&str> {
        match self {
            Self::Protected { token, .. } => Some(token),
            Self::Unprotected => None,
        }
    }

    /// True when `candidate` is the bootstrap token. Constant-time, so a
    /// caller cannot learn the token one character at a time.
    pub fn matches_bootstrap(&self, candidate: &str) -> bool {
        match self {
            Self::Protected { token, .. } => {
                crate::crypto::constant_time_eq(token.as_bytes(), candidate.as_bytes())
            }
            Self::Unprotected => false,
        }
    }

    /// Parses the pair, refusing every half-configured combination.
    ///
    /// The two rejections are the whole point of this function. A token
    /// without a key would leave app-token encryption undefined; a key
    /// without a token would look configured while the door stood open,
    /// which is the exact failure this feature exists to prevent.
    pub fn parse(token: Option<&str>, key: Option<&str>) -> Result<Self> {
        let token = token.map(str::trim).filter(|value| !value.is_empty());
        let key = key.map(str::trim).filter(|value| !value.is_empty());

        match (token, key) {
            (None, None) => Ok(Self::Unprotected),
            (None, Some(_)) => bail!(
                "KYU_SECRET_KEY is set but KYU_TOKEN is not, so the hub                  would run with its door open while looking configured. Set                  KYU_TOKEN as well, or unset KYU_SECRET_KEY to run                  deliberately unprotected."
            ),
            (Some(_), None) => bail!(
                "KYU_TOKEN is set but KYU_SECRET_KEY is not. App tokens \
                 are encrypted with that key, and deriving it from KYU_TOKEN \
                 would mean that rotating a leaked token silently destroys every \
                 app token you have. Add this line and keep it with the token:\n\
                 \n    KYU_SECRET_KEY={}\n",
                SecretKey::generate_hex()
            ),
            (Some(token), Some(key)) => {
                if token.len() < MIN_TOKEN_LEN {
                    bail!(
                        "KYU_TOKEN is {} characters, which is short enough to \
                         guess. Use at least {MIN_TOKEN_LEN}; generate one with: \
                         openssl rand -hex 24",
                        token.len()
                    );
                }
                Ok(Self::Protected {
                    token: token.to_string(),
                    key: SecretKey::parse_hex(key)?,
                })
            }
        }
    }
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let listen = std::env::var("KYU_LISTEN").ok();
        let data_dir = std::env::var("KYU_DATA_DIR").ok();
        let max_body = std::env::var("KYU_MAX_BODY_BYTES").ok();
        let mut config = Self::parse(listen.as_deref(), data_dir.as_deref(), max_body.as_deref())?;

        config.defaults = Defaults {
            retention_ms: duration_from_env("KYU_RETENTION_MS", config.defaults.retention_ms)?,
            idle_flag_ms: duration_from_env(
                "KYU_IDLE_FLAG_MS",
                Some(config.defaults.idle_flag_ms),
            )?
            .unwrap_or(config.defaults.idle_flag_ms),
            idle_archive_ms: duration_from_env(
                "KYU_IDLE_ARCHIVE_MS",
                Some(config.defaults.idle_archive_ms),
            )?
            .unwrap_or(config.defaults.idle_archive_ms),
        };
        config.auth = Auth::parse(
            std::env::var("KYU_TOKEN").ok().as_deref(),
            std::env::var("KYU_SECRET_KEY").ok().as_deref(),
        )?;
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
                "KYU_LISTEN is not a socket address: {listen_raw:?}. \
                 Set it as HOST:PORT (for example 0.0.0.0:8080), \
                 or unset it to use the default {DEFAULT_LISTEN}."
            )
        })?;

        let data_dir = PathBuf::from(data_dir.unwrap_or(DEFAULT_DATA_DIR));
        if data_dir.as_os_str().is_empty() {
            bail!(
                "KYU_DATA_DIR is empty. Set it to the directory holding \
                 the store (for example /data), or unset it to use the \
                 default {DEFAULT_DATA_DIR}."
            );
        }

        let max_body_bytes = match max_body_bytes {
            None => DEFAULT_MAX_BODY_BYTES,
            Some(raw) => {
                let parsed: u64 = raw.parse().with_context(|| {
                    format!(
                        "KYU_MAX_BODY_BYTES is not a whole number of bytes: {raw:?}. \
                         Set it to a byte count (for example 1048576 for 1 MiB), \
                         or unset it to use the default {DEFAULT_MAX_BODY_BYTES}."
                    )
                })?;
                if parsed == 0 {
                    bail!(
                        "KYU_MAX_BODY_BYTES is 0, which would reject every \
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
            auth: Auth::Unprotected,
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
        let config = Config::parse(Some("127.0.0.1:9000"), Some("/srv/kyu"), Some("2048"))
            .expect("explicit values must parse");
        assert_eq!(config.listen.to_string(), "127.0.0.1:9000");
        assert_eq!(config.data_dir, PathBuf::from("/srv/kyu"));
        assert_eq!(config.max_body_bytes, 2048);
    }

    #[test]
    fn l0_invalid_listen_fails_with_a_remedy() {
        let error = Config::parse(Some("not-an-address"), None, None)
            .expect_err("an invalid address must not fall back to the default");
        let message = format!("{error:#}");
        assert!(message.contains("KYU_LISTEN"), "names the variable");
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
    fn p7_no_token_and_no_key_runs_unprotected() {
        assert_eq!(
            Auth::parse(None, None).expect("nothing set parses"),
            Auth::Unprotected
        );
    }

    #[test]
    fn p7_empty_strings_count_as_absent() {
        // Compose writes KYU_TOKEN= when a variable is left blank, and
        // treating that as a token would protect the hub with "".
        assert_eq!(
            Auth::parse(Some("   "), Some("")).expect("blank values parse as absent"),
            Auth::Unprotected
        );
    }

    #[test]
    fn p7_a_token_without_a_key_refuses_to_start_and_hands_over_a_key() {
        let error = Auth::parse(Some("a-perfectly-long-token"), None)
            .expect_err("a half-configured door must not start");
        let message = format!("{error:#}");
        assert!(
            message.contains("KYU_SECRET_KEY="),
            "hands over a pastable line: {message}"
        );
        assert!(message.contains("rotating"), "explains why, not just what");
        let generated = message
            .rsplit("KYU_SECRET_KEY=")
            .next()
            .expect("the line is present")
            .trim();
        assert!(
            crate::crypto::SecretKey::parse_hex(generated).is_ok(),
            "the key it tells you to paste must itself be valid: {generated:?}"
        );
    }

    #[test]
    fn p7_a_key_without_a_token_refuses_to_start() {
        let error = Auth::parse(None, Some(&crate::crypto::SecretKey::generate_hex()))
            .expect_err("a key with an open door must not start");
        assert!(
            format!("{error:#}").contains("door open"),
            "says what is actually wrong"
        );
    }

    #[test]
    fn p7_a_short_token_is_refused() {
        let error = Auth::parse(
            Some("short"),
            Some(&crate::crypto::SecretKey::generate_hex()),
        )
        .expect_err("a guessable token must be refused");
        assert!(
            format!("{error:#}").contains("openssl rand"),
            "carries a remedy"
        );
    }

    #[test]
    fn p7_a_complete_pair_protects_the_hub() {
        let auth = Auth::parse(
            Some("a-perfectly-long-token"),
            Some(&crate::crypto::SecretKey::generate_hex()),
        )
        .expect("a complete pair parses");
        assert!(auth.is_protected());
        assert!(auth.matches_bootstrap("a-perfectly-long-token"));
        assert!(!auth.matches_bootstrap("a-perfectly-long-toke"));
        assert!(!auth.matches_bootstrap(""));
    }

    #[test]
    fn p7_an_unprotected_hub_matches_no_token_at_all() {
        // Including the empty string, which is what an absent header parses to.
        let auth = Auth::parse(None, None).expect("unprotected parses");
        assert!(!auth.matches_bootstrap(""));
        assert!(!auth.matches_bootstrap("anything"));
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
