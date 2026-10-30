//! Damping of repeated notifications: a publisher that names a dedup key
//! gets at most one message per key and window on a topic.
//!
//! Pure rules live here (key validation, window parsing, the request type);
//! the SQL is `crate::store::dedup`, the header parsing `crate::http::dedup`.

use super::clock::Millis;

/// Longest window a publisher may ask for.
pub const MAX_WINDOW_MS: Millis = 7 * 24 * 60 * 60 * 1_000;
/// Window used when only a key is given.
pub const DEFAULT_WINDOW_MS: Millis = 24 * 60 * 60 * 1_000;
/// Key length bounds, in bytes.
pub const MAX_KEY_BYTES: usize = 200;

/// A publish's damping request: the key and how long its first message
/// suppresses repeats.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dedup {
    pub key: String,
    pub window_ms: Millis,
}

impl Dedup {
    pub fn new(key: &str, window_ms: Option<Millis>) -> Result<Self, String> {
        validate_key(key)?;
        let window_ms = window_ms.unwrap_or(DEFAULT_WINDOW_MS);
        if window_ms <= 0 || window_ms > MAX_WINDOW_MS {
            return Err(format!(
                "the window is {window_ms} ms; it must be between 1 ms and 7 days ({MAX_WINDOW_MS} ms)"
            ));
        }
        Ok(Self {
            key: key.to_string(),
            window_ms,
        })
    }
}

/// 1-200 bytes of printable ASCII (space allowed inside, not at the ends).
pub fn validate_key(key: &str) -> Result<(), String> {
    if key.is_empty() || key.len() > MAX_KEY_BYTES {
        return Err(format!(
            "the key is {} bytes; it must be 1 to {MAX_KEY_BYTES}",
            key.len()
        ));
    }
    if !key.bytes().all(|byte| (0x20..=0x7e).contains(&byte)) {
        return Err("the key may only contain printable ASCII characters".to_string());
    }
    if key.starts_with(' ') || key.ends_with(' ') {
        return Err("the key must not start or end with a space".to_string());
    }
    Ok(())
}

/// A window as text: whole milliseconds (`1200000`) or a whole number with a
/// unit: `ms`, `s`, `m`, `h`, `d` (`30m`, `20h`, `2d`).
pub fn parse_window(raw: &str) -> Result<Millis, String> {
    let raw = raw.trim();
    let split = raw.find(|c: char| !c.is_ascii_digit()).unwrap_or(raw.len());
    let (digits, unit) = raw.split_at(split);
    let number: i64 = digits
        .parse()
        .map_err(|_| format!("{raw:?} is not a duration"))?;
    let factor: i64 = match unit {
        "" | "ms" => 1,
        "s" => 1_000,
        "m" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        _ => return Err(format!("{raw:?} has an unknown unit {unit:?}")),
    };
    number
        .checked_mul(factor)
        .ok_or_else(|| format!("{raw:?} is too large"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_parse_in_every_unit() {
        assert_eq!(parse_window("1500"), Ok(1_500));
        assert_eq!(parse_window("250ms"), Ok(250));
        assert_eq!(parse_window("45s"), Ok(45_000));
        assert_eq!(parse_window("30m"), Ok(1_800_000));
        assert_eq!(parse_window("20h"), Ok(72_000_000));
        assert_eq!(parse_window("2d"), Ok(172_800_000));
        assert!(parse_window("").is_err());
        assert!(parse_window("-5m").is_err());
        assert!(parse_window("1.5h").is_err());
        assert!(parse_window("3w").is_err());
    }

    #[test]
    fn the_window_is_capped_and_defaults() {
        assert_eq!(Dedup::new("k", None).unwrap().window_ms, DEFAULT_WINDOW_MS);
        assert!(Dedup::new("k", Some(MAX_WINDOW_MS)).is_ok());
        assert!(Dedup::new("k", Some(MAX_WINDOW_MS + 1)).is_err());
        assert!(Dedup::new("k", Some(0)).is_err());
    }

    #[test]
    fn keys_are_bounded_printable_text() {
        assert!(validate_key("backup/ct109 failed").is_ok());
        assert!(validate_key("").is_err());
        assert!(validate_key(&"k".repeat(201)).is_err());
        assert!(validate_key("tab\there").is_err());
        assert!(validate_key("caf\u{e9}").is_err());
        assert!(validate_key(" lead").is_err());
    }
}
