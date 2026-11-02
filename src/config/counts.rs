//! Whole-number settings (`KYU_BACKUP_KEEP`, the caps on topics,
//! subscriptions and concurrent polls): a count of at least one, refused
//! with a remedy when it is anything else.

use anyhow::{Result, bail};

/// Reads one count setting from its raw environment value.
pub(super) fn count_setting(name: &str, raw: Option<&str>, default: usize) -> Result<usize> {
    let Some(raw) = raw else {
        return Ok(default);
    };
    match raw.parse::<usize>() {
        Ok(count) if count >= 1 => Ok(count),
        _ => bail!(
            "{name} is {raw:?}, which is not a usable count. Set a whole number of at \
             least 1 (the default is {default})."
        ),
    }
}

/// Reads a count setting from the process environment.
pub(super) fn count_from_env(name: &str, default: usize) -> Result<usize> {
    count_setting(name, std::env::var(name).ok().as_deref(), default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_count_defaults_when_absent_and_refuses_zero_or_text() {
        assert_eq!(count_setting("KYU_X", None, 7).expect("default"), 7);
        assert_eq!(count_setting("KYU_X", Some("3"), 7).expect("three"), 3);
        for bad in ["0", "-1", "many"] {
            let error = count_setting("KYU_X", Some(bad), 7).expect_err("unusable");
            assert!(format!("{error:#}").contains("KYU_X"), "{error:#}");
        }
    }
}
