//! App-token encryption and token generation (W2, AR11 amendment).
//!
//! App tokens are stored encrypted rather than hashed. That is a deliberate
//! reversal of the usual advice, and it is the only way to keep the promise
//! the dashboard is built on (S1): the page renders a command you can paste,
//! and a hash cannot be turned back into one. The key lives in the compose
//! file (`MAILBOX_SECRET_KEY`) and never in the store, so a stolen store
//! file on its own yields nothing.
//!
//! ChaCha20-Poly1305 rather than AES-GCM: no hardware acceleration is
//! assumed, which matters on whatever the LXC lands on, and a nonce reuse
//! bug is the failure mode we are least likely to survive — so each seal
//! draws a fresh random nonce and stores it alongside the ciphertext.

use anyhow::{Result, bail};
use chacha20poly1305::aead::rand_core::RngCore;
use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};

/// Length of the secret key, in bytes, before hex encoding.
pub const KEY_BYTES: usize = 32;
/// Length of the nonce ChaCha20-Poly1305 uses, in bytes.
const NONCE_BYTES: usize = 12;
/// Bytes of randomness behind a generated token. 24 bytes is 192 bits,
/// which is far past guessing and still short enough to fit on one line of
/// a compose file.
const TOKEN_BYTES: usize = 24;

/// The encryption key, parsed once at startup so a malformed key fails
/// before the hub answers a single request.
#[derive(Clone)]
pub struct SecretKey([u8; KEY_BYTES]);

impl std::fmt::Debug for SecretKey {
    /// Never print the key. `Config` is `Debug`-formatted in tests and could
    /// reach a log line by accident; this makes that harmless.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SecretKey(redacted)")
    }
}

impl PartialEq for SecretKey {
    fn eq(&self, other: &Self) -> bool {
        constant_time_eq(&self.0, &other.0)
    }
}

impl Eq for SecretKey {}

impl SecretKey {
    /// Parses a key from its hex form, which is what a human pastes into
    /// compose. Anything else is refused with the remedy attached, because
    /// a key that is "close enough" silently produces an unopenable store.
    pub fn parse_hex(raw: &str) -> Result<Self> {
        let trimmed = raw.trim();
        if trimmed.len() != KEY_BYTES * 2 {
            bail!(
                "MAILBOX_SECRET_KEY must be {} hex characters ({KEY_BYTES} bytes); \
                 this one is {}. Generate one with: openssl rand -hex {KEY_BYTES}",
                KEY_BYTES * 2,
                trimmed.len()
            );
        }
        let bytes = trimmed.as_bytes();
        let mut key = [0u8; KEY_BYTES];
        for index in 0..KEY_BYTES {
            let pair = &bytes[index * 2..index * 2 + 2];
            let text = std::str::from_utf8(pair).unwrap_or("??");
            key[index] = u8::from_str_radix(text, 16).map_err(|_| {
                anyhow::anyhow!(
                    "MAILBOX_SECRET_KEY contains {text:?}, which is not hexadecimal. \
                     Generate a valid key with: openssl rand -hex {KEY_BYTES}"
                )
            })?;
        }
        Ok(Self(key))
    }

    /// A fresh random key, in the hex form the error messages tell people to
    /// paste. Used when refusing to start, so the remedy is a copy away.
    pub fn generate_hex() -> String {
        let mut key = [0u8; KEY_BYTES];
        OsRng.fill_bytes(&mut key);
        to_hex(&key)
    }

    fn cipher(&self) -> ChaCha20Poly1305 {
        ChaCha20Poly1305::new(Key::from_slice(&self.0))
    }

    /// Encrypts `plaintext`, returning nonce ‖ ciphertext.
    pub fn seal(&self, plaintext: &str) -> Result<Vec<u8>> {
        let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);
        let ciphertext = self
            .cipher()
            .encrypt(&nonce, plaintext.as_bytes())
            .map_err(|_| anyhow::anyhow!("cannot encrypt an app token"))?;
        let mut sealed = Vec::with_capacity(NONCE_BYTES + ciphertext.len());
        sealed.extend_from_slice(&nonce);
        sealed.extend_from_slice(&ciphertext);
        Ok(sealed)
    }

    /// Reverses [`Self::seal`].
    ///
    /// The error deliberately names the likely cause rather than the crypto:
    /// in practice this fails because the key in compose changed, and
    /// "authentication failed" would send someone hunting in the wrong place.
    pub fn open(&self, sealed: &[u8]) -> Result<String> {
        if sealed.len() <= NONCE_BYTES {
            bail!("a stored app token is truncated and cannot be decrypted");
        }
        let (nonce, ciphertext) = sealed.split_at(NONCE_BYTES);
        let plaintext = self
            .cipher()
            .decrypt(Nonce::from_slice(nonce), ciphertext)
            .map_err(|_| {
                anyhow::anyhow!(
                    "a stored app token cannot be decrypted with the current \
                     MAILBOX_SECRET_KEY. If the key was changed, the tokens \
                     created under the old one are unreadable: revoke them on \
                     the apps page and generate replacements."
                )
            })?;
        String::from_utf8(plaintext)
            .map_err(|_| anyhow::anyhow!("a stored app token is not valid text"))
    }
}

/// A fresh app token. Hex rather than base64 so it survives being pasted
/// into a shell, a YAML file or a URL without quoting surprises.
pub fn generate_token() -> String {
    let mut bytes = [0u8; TOKEN_BYTES];
    OsRng.fill_bytes(&mut bytes);
    to_hex(&bytes)
}

fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

/// Compares two secrets without leaking, through timing, how much of a
/// guess was correct. Hand-rolled rather than pulling in another crate for
/// six lines (T6).
pub fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0u8, |difference, (a, b)| difference | (a ^ b))
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn p7_a_sealed_token_comes_back_intact() {
        let key = SecretKey::parse_hex(&SecretKey::generate_hex()).expect("a generated key parses");
        let sealed = key.seal("hunter2-but-longer").expect("sealing works");
        assert_eq!(
            key.open(&sealed).expect("opening works"),
            "hunter2-but-longer"
        );
    }

    #[test]
    fn p7_the_plaintext_never_appears_in_the_sealed_bytes() {
        let key = SecretKey::parse_hex(&SecretKey::generate_hex()).expect("a generated key parses");
        let secret = "a-very-recognisable-token";
        let sealed = key.seal(secret).expect("sealing works");
        assert!(
            !sealed.windows(secret.len()).any(|w| w == secret.as_bytes()),
            "the token must not survive in the clear inside the ciphertext"
        );
    }

    #[test]
    fn p7_sealing_twice_gives_different_bytes() {
        // A fixed nonce would make identical tokens produce identical rows,
        // which leaks that two apps share a token and, worse, breaks the
        // cipher outright.
        let key = SecretKey::parse_hex(&SecretKey::generate_hex()).expect("a generated key parses");
        assert_ne!(
            key.seal("same").expect("first seal"),
            key.seal("same").expect("second seal"),
            "each seal must draw a fresh nonce"
        );
    }

    #[test]
    fn p7_another_key_cannot_open_it_and_says_what_to_do() {
        let key = SecretKey::parse_hex(&SecretKey::generate_hex()).expect("a generated key parses");
        let other = SecretKey::parse_hex(&SecretKey::generate_hex()).expect("a second key parses");
        let sealed = key.seal("secret").expect("sealing works");
        let error = other
            .open(&sealed)
            .expect_err("a different key must not decrypt");
        let message = format!("{error:#}");
        assert!(message.contains("MAILBOX_SECRET_KEY"), "names the variable");
        assert!(message.contains("revoke"), "carries a remedy: {message}");
    }

    #[test]
    fn p7_tampered_ciphertext_is_refused() {
        let key = SecretKey::parse_hex(&SecretKey::generate_hex()).expect("a generated key parses");
        let mut sealed = key.seal("secret").expect("sealing works");
        let last = sealed.len() - 1;
        sealed[last] ^= 0xff;
        assert!(
            key.open(&sealed).is_err(),
            "the authentication tag must reject a flipped bit"
        );
    }

    #[test]
    fn p7_a_short_key_is_refused_with_the_command_to_generate_one() {
        let error = SecretKey::parse_hex("abcd").expect_err("a short key must be refused");
        let message = format!("{error:#}");
        assert!(
            message.contains("openssl rand -hex 32"),
            "carries a remedy: {message}"
        );
    }

    #[test]
    fn p7_a_non_hex_key_is_refused() {
        let key = "z".repeat(KEY_BYTES * 2);
        assert!(
            SecretKey::parse_hex(&key).is_err(),
            "non-hexadecimal characters must be refused rather than coerced"
        );
    }

    #[test]
    fn p7_a_key_of_multibyte_characters_is_refused_without_panicking() {
        // 32 two-byte characters is 64 BYTES, which passes the length check.
        // Slicing that as text would land mid-character and panic; the parse
        // walks bytes for exactly this reason.
        let key = "é".repeat(KEY_BYTES);
        assert_eq!(key.len(), KEY_BYTES * 2, "the length check is satisfied");
        assert!(SecretKey::parse_hex(&key).is_err());
    }

    #[test]
    fn p7_a_key_survives_surrounding_whitespace() {
        // Compose files and shell exports pick up trailing newlines; failing
        // on one would be a baffling first-run experience.
        let hex = SecretKey::generate_hex();
        let padded = format!("  {hex}\n");
        assert_eq!(
            SecretKey::parse_hex(&padded).expect("padded key parses"),
            SecretKey::parse_hex(&hex).expect("bare key parses")
        );
    }

    #[test]
    fn p7_generated_tokens_are_long_and_distinct() {
        let first = generate_token();
        let second = generate_token();
        assert_eq!(first.len(), TOKEN_BYTES * 2);
        assert_ne!(first, second, "two generated tokens must not collide");
    }

    #[test]
    fn p7_a_secret_key_never_prints_itself() {
        let key = SecretKey::parse_hex(&SecretKey::generate_hex()).expect("a generated key parses");
        assert_eq!(format!("{key:?}"), "SecretKey(redacted)");
    }

    #[test]
    fn p7_constant_time_eq_still_compares_correctly() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
    }
}
