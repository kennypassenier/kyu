//! A payload as the dashboard shows it: capped, text or announced binary (AR11).

use serde::Serialize;

/// How much of a payload the dashboard shows. Enough to recognise a
/// message; the rest is announced rather than dropped in silence (AR11).
pub const PAYLOAD_DISPLAY_LIMIT: usize = 4096;

/// How a payload is shown: text as text, binary announced as binary, and
/// anything oversized marked as truncated with its real size.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PayloadView {
    pub text: String,
    pub bytes: usize,
    pub truncated: bool,
    pub binary: bool,
}

impl PayloadView {
    pub fn of(payload: &[u8]) -> Self {
        let bytes = payload.len();
        let shown = bytes.min(PAYLOAD_DISPLAY_LIMIT);
        let truncated = bytes > shown;

        let text = match std::str::from_utf8(&payload[..shown]) {
            Ok(text) => Some(text),
            // `error_len() == None` means the slice ends in the middle of a
            // character, which is what truncation does to any non-ASCII
            // payload. Backing off to the last whole character is the
            // difference between a readable message and one that claims to
            // be binary.
            Err(error) if truncated && error.error_len().is_none() => {
                std::str::from_utf8(&payload[..error.valid_up_to()]).ok()
            }
            Err(_) => None,
        };

        match text {
            Some(text) => Self {
                text: text.to_string(),
                bytes,
                truncated,
                binary: false,
            },
            None => Self {
                text: String::new(),
                bytes,
                truncated,
                binary: true,
            },
        }
    }

    /// What a human should read next to the payload. Never empty, because a
    /// blank space where bytes used to be is the silence G8 forbids.
    pub fn note(&self) -> Option<String> {
        match (self.binary, self.truncated) {
            (true, _) => Some(format!("binary payload ({} bytes)", self.bytes)),
            (false, true) => Some(format!(
                "showing the first {PAYLOAD_DISPLAY_LIMIT} of {} bytes",
                self.bytes
            )),
            (false, false) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn p7_a_character_split_by_the_display_cap_does_not_look_like_binary() {
        // 4095 ASCII bytes then a two-byte character: the cut lands in the
        // middle of it. Reported as binary, a perfectly ordinary Dutch
        // message would be unreadable on the page.
        let mut payload = "x".repeat(PAYLOAD_DISPLAY_LIMIT - 1).into_bytes();
        payload.extend_from_slice("é".as_bytes());

        let view = PayloadView::of(&payload);

        assert!(
            !view.binary,
            "text must stay text even when truncation lands mid-character"
        );
        assert!(view.truncated);
        assert_eq!(
            view.text.len(),
            PAYLOAD_DISPLAY_LIMIT - 1,
            "the display backs off to the last whole character"
        );
        assert_eq!(view.bytes, PAYLOAD_DISPLAY_LIMIT + 1);
    }

    #[test]
    fn p7_genuinely_invalid_bytes_are_still_reported_as_binary() {
        // The fix for the boundary case must not turn every binary payload
        // into a half-rendered string.
        let payload = vec![0x41, 0x42, 0xff, 0xfe];
        let view = PayloadView::of(&payload);
        assert!(view.binary, "invalid bytes in the middle are not text");
    }

    #[test]
    fn l7_a_text_payload_is_shown_as_text() {
        let view = PayloadView::of(br#"{"title":"Backup klaar"}"#);
        assert_eq!(view.text, r#"{"title":"Backup klaar"}"#);
        assert!(!view.binary);
        assert!(!view.truncated);
        assert_eq!(
            view.note(),
            None,
            "nothing to announce about a plain payload"
        );
    }

    #[test]
    fn l7_a_binary_payload_is_announced_rather_than_mangled() {
        let view = PayloadView::of(&[0x00, 0xff, 0x1b, 0x80]);
        assert!(view.binary);
        assert_eq!(view.text, "");
        assert_eq!(view.note().as_deref(), Some("binary payload (4 bytes)"));
    }

    #[test]
    fn l7_an_oversized_payload_says_how_much_is_hidden() {
        let payload = "x".repeat(PAYLOAD_DISPLAY_LIMIT * 2);
        let view = PayloadView::of(payload.as_bytes());

        assert!(view.truncated);
        assert_eq!(view.text.len(), PAYLOAD_DISPLAY_LIMIT);
        assert_eq!(view.bytes, PAYLOAD_DISPLAY_LIMIT * 2);
        let note = view.note().expect("a truncated payload must say so");
        assert!(
            note.contains(&PAYLOAD_DISPLAY_LIMIT.to_string()) && note.contains("8192"),
            "the note carries both numbers: {note}"
        );
    }
}
