//! What the binary does with its command line.
//!
//! There is almost nothing here on purpose: configuration is environment
//! variables (AR6), so the command line only chooses *which of three things
//! this invocation is*. The reason it exists at all is the failure it
//! prevents — before 1.0.1 an unknown flag was silently ignored and the hub
//! started anyway, so `kyu --version` became a running server, and a
//! typo in a unit file would have started a second hub on the same store.
//! Standing rule 12: no silent fallbacks.

use std::fmt;

/// What this invocation is asking for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Run the hub. The ordinary case, and what no arguments means.
    Serve,
    /// Probe a running hub over a socket and exit (W6): the container
    /// healthcheck, because the distroless image has no shell.
    Healthcheck,
    Version,
    Help,
}

/// Why a command line was refused. Carries its own remedy (standing rule 11).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    pub argument: String,
    looks_like_a_flag: bool,
}

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "kyu does not understand {:?}.\n\n", self.argument)?;
        if self.looks_like_a_flag {
            write!(f, "Run kyu --help to see the flags it does accept.")
        } else {
            // The shape people actually get wrong: passing a config file,
            // because most daemons take one.
            write!(
                f,
                "kyu takes no positional arguments and no config file — \
                 everything is configured through KYU_* environment \
                 variables. Run kyu --help for the list."
            )
        }
    }
}

/// The text `--help` prints. Names the environment, because that is where a
/// reader who reached for `--help` is actually trying to go.
pub const HELP: &str = "\
kyu — a durable message hub

USAGE:
    kyu                 run the hub (configuration comes from the environment)
    kyu --healthcheck   probe a running hub and exit 0 if it is healthy
    kyu --version       print the version and exit
    kyu --help          print this and exit

CONFIGURATION (environment only — there is no config file):
    KYU_LISTEN          address to bind          default 0.0.0.0:8080
    KYU_DATA_DIR        where the store lives    default /data
    KYU_TOKEN           the token to require     default none (open hub)
    KYU_SECRET_KEY      encrypts per-app tokens  required with KYU_TOKEN
    KYU_MAX_BODY_BYTES  largest accepted payload default 1048576
    KYU_LOG             log filter               default info
    KYU_LOG_FORMAT      set to json for Loki     default human-readable
    KYU_SHUTDOWN_TIMEOUT_MS  how long a graceful stop may take, in ms
                                                 default 10000
    KYU_RETENTION_MS    default retention        default 604800000, or never
    KYU_IDLE_FLAG_MS    flag a quiet consumer    default 604800000
    KYU_IDLE_ARCHIVE_MS archive a quiet consumer default 2592000000

Full documentation: https://github.com/kennypassenier/kyu";

/// Decides what an invocation means, refusing anything it does not recognise.
///
/// Takes the arguments *after* the program name, so tests never have to
/// fabricate an argv[0].
pub fn parse<I, S>(arguments: I) -> Result<Action, Refused>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut action = Action::Serve;

    for argument in arguments {
        let argument = argument.as_ref();
        match argument {
            "--healthcheck" => action = Action::Healthcheck,
            "--version" | "-V" => action = Action::Version,
            "--help" | "-h" => return Ok(Action::Help),
            other => {
                return Err(Refused {
                    argument: other.to_string(),
                    looks_like_a_flag: other.starts_with('-'),
                });
            }
        }
    }

    Ok(action)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn p7_no_arguments_means_serve() {
        assert_eq!(parse(Vec::<String>::new()), Ok(Action::Serve));
    }

    #[test]
    fn p7_the_three_known_flags_are_recognised() {
        assert_eq!(parse(["--healthcheck"]), Ok(Action::Healthcheck));
        assert_eq!(parse(["--version"]), Ok(Action::Version));
        assert_eq!(parse(["-V"]), Ok(Action::Version));
        assert_eq!(parse(["--help"]), Ok(Action::Help));
        assert_eq!(parse(["-h"]), Ok(Action::Help));
    }

    #[test]
    fn p7_an_unknown_flag_is_refused_and_named() {
        let refused = parse(["--serve-forever"]).expect_err("must be refused");
        let message = refused.to_string();
        assert!(message.contains("--serve-forever"), "names it: {message}");
        assert!(message.contains("--help"), "carries a remedy: {message}");
    }

    #[test]
    fn p7_a_positional_argument_is_refused_and_points_at_the_environment() {
        // Someone assumed a config file, because most daemons take one.
        let refused = parse(["/etc/kyu.conf"]).expect_err("must be refused");
        let message = refused.to_string();
        assert!(message.contains("/etc/kyu.conf"), "names it: {message}");
        assert!(
            message.contains("KYU_") && message.contains("no config file"),
            "says where configuration actually lives: {message}"
        );
    }

    #[test]
    fn p7_help_wins_over_anything_after_it() {
        // Asking for help and getting a server would be the same bug again.
        assert_eq!(parse(["--help", "--nonsense"]), Ok(Action::Help));
    }

    #[test]
    fn p7_a_refusal_never_degrades_into_serving() {
        // The property this module exists for: no input silently produces
        // Serve except an input that actually means Serve.
        for bad in ["--verison", "-x", "start", "--", "‑‑healthcheck"] {
            assert!(
                parse([bad]).is_err(),
                "{bad:?} must be refused rather than ignored"
            );
        }
    }

    #[test]
    fn p7_help_text_documents_every_flag_parse_accepts() {
        for flag in ["--healthcheck", "--version", "--help"] {
            assert!(HELP.contains(flag), "help must document {flag}");
        }
    }
}
