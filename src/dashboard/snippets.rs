//! The copy-paste curl examples, rendered from the topic's own data (S1).

use serde::Serialize;

use super::payload::PayloadView;

/// The copy-paste examples, rendered with this topic's own real payload and
/// a subscription name that actually exists. Generic documentation is what
/// you skim; your own data is what you trust (S1).
#[derive(Debug, Clone, Serialize)]
pub struct Snippets {
    pub publish: Snippet,
    pub receive_raw: Snippet,
    pub receive_envelope: Snippet,
    pub ack: Snippet,
    pub bootstrap_note: Option<String>,
    /// Which app's token the commands carry, for the selector on the page.
    pub app: Option<String>,
}

/// One copy-paste command in two forms (W2).
///
/// `shown` is what the page displays, with the token masked; `full` is what
/// the copy button puts on the clipboard. Keeping them apart is the whole
/// point of Kenny's requirement: pasting works immediately, but someone
/// glancing at the screen over your shoulder learns nothing.
#[derive(Debug, Clone, Serialize)]
pub struct Snippet {
    pub shown: String,
    pub full: String,
}

impl Snippet {
    /// Builds both forms from a command that may carry a token.
    ///
    /// The masked form is produced by substituting the token, not by
    /// re-rendering: any future edit to the command shape then cannot make
    /// the two drift apart and accidentally print a real token.
    fn new(full: String, token: Option<&str>) -> Self {
        let shown = match token {
            Some(token) if !token.is_empty() => full.replace(token, &mask_token(token)),
            _ => full.clone(),
        };
        Self { shown, full }
    }
}

impl Snippets {
    pub fn build(
        base_url: &str,
        topic: &str,
        subscription: Option<&str>,
        example_payload: Option<&PayloadView>,
        content_type: Option<&str>,
        token: Option<&str>,
        app: Option<&str>,
    ) -> Self {
        let payload = match example_payload {
            Some(view) if !view.binary && !view.text.is_empty() => {
                let single_line: String = view.text.lines().collect::<Vec<_>>().join("");
                single_line.replace('\'', "'\\''")
            }
            _ => r#"{"hello":"world"}"#.to_string(),
        };
        // The content type is whatever a publisher put in the header, and
        // this string is pasted into a shell. A quote in it closes the
        // argument and everything after it runs on the operator's machine —
        // HTML escaping protects the browser, not the clipboard. Anything
        // that is not an ordinary media type is replaced rather than
        // escaped, because a snippet nobody can read is no use either.
        let content_type = content_type
            .filter(|value| is_plain_media_type(value))
            .unwrap_or("application/json");
        let name = subscription.unwrap_or("my-consumer");

        // One extra argument line when the hub has a door, and none at all
        // when it does not — an unprotected hub must not print an
        // authorization header that would only confuse whoever pastes it.
        let auth = match token {
            Some(token) if !token.is_empty() => {
                format!("-H 'authorization: Bearer {token}' \\\n     ")
            }
            _ => String::new(),
        };

        Self {
            publish: Snippet::new(
                format!(
                    "curl -s {auth}-H 'content-type: {content_type}' \\\n     -d '{payload}' \\\n     {base_url}/t/{topic}"
                ),
                token,
            ),
            receive_raw: Snippet::new(
                format!(
                    "curl -s {auth}-D- -o message.body \"{base_url}/t/{topic}/next?as={name}\""
                ),
                token,
            ),
            receive_envelope: Snippet::new(
                format!("curl -s {auth}\"{base_url}/t/{topic}/next?as={name}&envelope=json\""),
                token,
            ),
            ack: Snippet::new(
                format!("curl -s -X POST {auth}\"{base_url}/t/{topic}/ack/<id>?as={name}\""),
                token,
            ),
            app: app.map(str::to_string),
            bootstrap_note: subscription.is_none().then(|| {
                format!(
                    "no subscription has polled {topic} yet. A subscription starts existing \
                     when it first polls, and it receives what is published after that — so \
                     run the receive command once before publishing the message you want it \
                     to see, or add &from=beginning to pick up what the topic still retains."
                )
            }),
        }
    }
}

/// Shows enough of a token to tell two of them apart, and no more.
pub fn mask_token(token: &str) -> String {
    const SHOWN: usize = 4;
    if token.len() <= SHOWN {
        return "\u{2022}".repeat(8);
    }
    format!("{}{}", "\u{2022}".repeat(8), &token[token.len() - SHOWN..])
}

/// Whether a stored content type is safe to print inside a shell command:
/// a token, a slash, a token, and optional `; key=value` parameters. No
/// quotes, no spaces beyond the parameter separators, no shell metacharacters.
fn is_plain_media_type(value: &str) -> bool {
    fn is_token(part: &str) -> bool {
        !part.is_empty()
            && part.bytes().all(|byte| {
                byte.is_ascii_alphanumeric()
                    || matches!(
                        byte,
                        b'!' | b'#' | b'$' | b'&' | b'^' | b'_' | b'.' | b'+' | b'-'
                    )
            })
    }

    let mut parts = value.split(';');
    let Some((kind, subtype)) = parts.next().and_then(|media| media.split_once('/')) else {
        return false;
    };
    if !is_token(kind) || !is_token(subtype) {
        return false;
    }
    parts.all(|parameter| {
        parameter
            .trim()
            .split_once('=')
            .is_some_and(|(name, argument)| is_token(name) && is_token(argument.trim_matches('"')))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l7_snippets_use_the_topic_and_a_real_subscription() {
        let payload = PayloadView::of(br#"{"title":"Backup klaar"}"#);
        let snippets = Snippets::build(
            "http://hub.lan",
            "notify.kenny",
            Some("printer"),
            Some(&payload),
            Some("application/json"),
            None,
            None,
        );

        assert!(snippets.publish.full.contains("notify.kenny"));
        assert!(
            snippets
                .publish
                .full
                .contains(r#"{"title":"Backup klaar"}"#)
        );
        assert!(snippets.receive_raw.full.contains("as=printer"));
        assert!(snippets.receive_envelope.full.contains("envelope=json"));
        assert!(snippets.ack.full.contains("as=printer"));
        assert!(snippets.bootstrap_note.is_none());
    }

    #[test]
    fn l7_a_topic_without_consumers_explains_the_bootstrap_order() {
        let snippets = Snippets::build(
            "http://hub.lan",
            "notify.kenny",
            None,
            None,
            None,
            None,
            None,
        );
        let note = snippets
            .bootstrap_note
            .expect("a topic nobody polls must explain the order");
        assert!(
            note.contains("first polls") && note.contains("from=beginning"),
            "the note has to teach the G7 rule, since that is the trap: {note}"
        );
    }

    #[test]
    fn l7_a_quote_in_a_payload_cannot_break_out_of_the_snippet() {
        let payload = PayloadView::of(b"it's a trap");
        let snippets = Snippets::build(
            "http://hub.lan",
            "t",
            Some("s"),
            Some(&payload),
            None,
            None,
            None,
        );
        assert!(
            snippets.publish.full.contains(r"'\''"),
            "a single quote must be escaped for the shell: {}",
            snippets.publish.full
        );
    }
}
