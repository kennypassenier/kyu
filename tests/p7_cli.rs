//! [P7] What the binary does when it is called wrong.
//!
//! Found while deploying 1.0.0 onto its LXC: `mailbox --version` did not
//! print a version and did not complain — it started the hub, which then sat
//! there until someone noticed. Every unknown flag did the same, silently.
//!
//! That is a fail-open in the one place a fail-open is expensive: a typo in a
//! systemd unit or a deploy script starts a second hub on the same store
//! instead of refusing. Standing rule 12 says no silent fallbacks, and rule
//! 11 says every error carries a remedy; this suite pins both for the
//! command line.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Runs the binary and returns (exit code, stdout+stderr), killing it if it
/// decides to become a server instead of answering.
///
/// The timeout is the whole point: the bug this suite exists for turns a
/// question into a running process, so a test that simply waited would hang
/// rather than fail.
fn run(args: &[&str]) -> (Option<i32>, String) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut child = Command::new(env!("CARGO_BIN_EXE_mailbox"))
        .args(args)
        .env("MAILBOX_DATA_DIR", dir.path())
        // Port 0 is not bindable as a listener address here, so pick
        // something out of the way: if the binary wrongly starts serving, it
        // must not collide with anything real.
        .env("MAILBOX_LISTEN", "127.0.0.1:59999")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the binary must start");

    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        match child.try_wait().expect("waiting on the child must work") {
            Some(status) => break Some(status),
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    };

    let mut output = String::new();
    if let Some(mut out) = child.stdout.take() {
        let _ = out.read_to_string(&mut output);
    }
    if let Some(mut err) = child.stderr.take() {
        let _ = err.read_to_string(&mut output);
    }
    // `None` for the code means it never exited — it became a server.
    (status.and_then(|s| s.code()), output)
}

#[tokio::test]
async fn p7_version_prints_a_version_and_exits() {
    let (code, output) = run(&["--version"]);
    assert_eq!(
        code,
        Some(0),
        "--version must answer and exit, not start the hub. Output: {output}"
    );
    assert!(
        output.contains(env!("CARGO_PKG_VERSION")),
        "it must print the actual version: {output}"
    );
}

#[tokio::test]
async fn p7_help_lists_what_the_binary_accepts() {
    let (code, output) = run(&["--help"]);
    assert_eq!(code, Some(0), "--help must answer and exit: {output}");
    for expected in ["--healthcheck", "--version", "MAILBOX_TOKEN"] {
        assert!(
            output.contains(expected),
            "help must mention {expected}: {output}"
        );
    }
}

#[tokio::test]
async fn p7_an_unknown_flag_is_refused_with_a_remedy() {
    let (code, output) = run(&["--serve-forever"]);
    assert_eq!(
        code,
        Some(2),
        "an unknown flag must be refused, not ignored. Output: {output}"
    );
    assert!(
        output.contains("--serve-forever"),
        "the refusal names the flag it did not understand: {output}"
    );
    assert!(
        output.contains("--help"),
        "and carries a remedy (standing rule 11): {output}"
    );
}

#[tokio::test]
async fn p7_a_stray_positional_argument_is_refused_too() {
    // The shape a deploy script gets wrong: `mailbox /etc/mailbox.conf`,
    // written by someone who assumed a config file. Starting the hub and
    // ignoring the path is the worst of the available answers.
    let (code, output) = run(&["/etc/mailbox.conf"]);
    assert_eq!(code, Some(2), "a stray argument must be refused: {output}");
    assert!(
        output.contains("/etc/mailbox.conf"),
        "the refusal names it: {output}"
    );
    assert!(
        output.contains("MAILBOX_"),
        "and points at the environment, which is where configuration lives: {output}"
    );
}

#[tokio::test]
async fn p7_no_arguments_still_starts_the_hub() {
    // The regression guard for the fix: refusing unknown flags must not
    // start refusing the ordinary case.
    let (code, output) = run(&[]);
    assert_eq!(
        code, None,
        "with no arguments the binary serves until killed, and does not exit: {output}"
    );
}
