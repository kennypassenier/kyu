//! The door policy (W2): no token at all, or a token with its sealing key.

use anyhow::{Result, bail};

use crate::crypto::SecretKey;

/// Shortest bootstrap token we accept. Long enough that a guess is not a
/// realistic attack, short enough to type once into a compose file.
pub const MIN_TOKEN_LEN: usize = 16;

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
