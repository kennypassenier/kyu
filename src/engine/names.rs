//! Topic and subscription naming (AR8).
//!
//! Names are lowercase, dot-namespaced and short: `notify.kenny`,
//! `jobs.transcode`. The pattern is deliberately narrow — names appear in
//! URLs, log lines and dashboard tables, and a name that needs escaping
//! anywhere is a name that will eventually be wrong somewhere.

/// The prefix the hub keeps for its own event topics (W11). Publishing
/// there from outside is refused, so nothing can forge hub events.
pub const RESERVED_PREFIX: &str = "mailbox.";

pub const MAX_NAME_LEN: usize = 64;

/// `^[a-z0-9._-]{1,64}$`, and no leading, trailing or doubled dot — a
/// segment must actually have a name.
pub fn is_valid(name: &str) -> bool {
    if name.is_empty() || name.len() > MAX_NAME_LEN {
        return false;
    }
    if !name.bytes().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
    }) {
        return false;
    }
    !(name.starts_with('.') || name.ends_with('.') || name.contains(".."))
}

pub fn is_reserved(topic: &str) -> bool {
    topic.starts_with(RESERVED_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l2_accepts_the_names_the_docs_use() {
        for name in [
            "notify.kenny",
            "jobs.transcode",
            "print.receipt",
            "speak.kenny_pc",
            "ha-forwarder",
            "a",
            "x9",
        ] {
            assert!(is_valid(name), "{name} must be accepted");
        }
    }

    #[test]
    fn l2_rejects_names_that_would_need_escaping_or_confuse_a_reader() {
        for name in [
            "",
            "Notify.Kenny",  // uppercase: two names that look the same
            "notify kenny",  // space
            "notify/kenny",  // would split the URL path
            "notify.kenny!", // punctuation
            ".notify",       // empty leading segment
            "notify.",       // empty trailing segment
            "notify..kenny", // empty middle segment
            "naïve",         // non-ASCII
        ] {
            assert!(!is_valid(name), "{name:?} must be rejected");
        }
    }

    #[test]
    fn l2_rejects_a_name_longer_than_the_limit() {
        assert!(is_valid(&"a".repeat(MAX_NAME_LEN)));
        assert!(!is_valid(&"a".repeat(MAX_NAME_LEN + 1)));
    }

    #[test]
    fn l2_recognises_the_reserved_prefix() {
        assert!(is_reserved("mailbox.events"));
        assert!(
            !is_reserved("mailboxes.kenny"),
            "only the dotted prefix is reserved"
        );
        assert!(!is_reserved("notify.kenny"));
    }
}
