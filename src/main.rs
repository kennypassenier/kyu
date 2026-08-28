//! Thin binary (AR1): read configuration, start logging, serve. Every
//! behaviour worth testing lives in the library.

use std::sync::Arc;

use anyhow::{Context, Result};
use mailbox::config::Config;
use mailbox::engine::Engine;
use mailbox::engine::clock::{Clock, SystemClock};
use mailbox::http::{AppState, Limits, router};
use mailbox::store::Store;
use mailbox::sweeper::{self, Heartbeat};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    // The runtime image has no shell, so the container healthcheck cannot
    // call curl: the binary probes itself instead (W6, T9).
    if std::env::args().any(|argument| argument == "--healthcheck") {
        return healthcheck();
    }

    init_tracing();

    let config = Config::from_env()?;

    // Opening the store migrates it forward, snapshotting first if there is
    // anything to lose (AR10). Failing here is correct: serving requests
    // without somewhere durable to put them would break K1's promise that a
    // confirmed publish is a kept one.
    let store = Arc::new(Store::open(&config.data_dir)?);
    tracing::info!(
        store = %store.path().map(|path| path.display().to_string()).unwrap_or_default(),
        "store ready"
    );

    let clock = SystemClock;
    let heartbeat = Heartbeat::starting_at(clock.now_ms());
    let engine = Arc::new(Engine::with_defaults(
        store,
        Arc::new(clock),
        config.defaults,
    ));
    // W2 · say out loud which of the two modes this is. An unprotected hub
    // is a legitimate choice; a hub you *think* is protected is not, and the
    // only defence against that is saying so on every single startup.
    if config.auth.is_protected() {
        tracing::info!("this hub requires a token (MAILBOX_TOKEN)");
    } else {
        tracing::warn!(
            "this hub has NO token: anyone who can reach it can read every \
             message, publish, and use the dashboard buttons. Set MAILBOX_TOKEN \
             and MAILBOX_SECRET_KEY to protect it."
        );
    }

    let state = AppState::with_auth(
        engine.clone(),
        Limits::from_config(&config),
        heartbeat.clone(),
        config.auth.clone(),
    );

    // The sweeper is what makes delivery at-least-once rather than
    // at-most-once: without it an expired lease would never come back.
    let notifiers = state.notifiers.clone();
    let _sweeper = sweeper::spawn(engine, heartbeat, move |woken| {
        for (topic, subscription) in woken {
            notifiers.wake(topic, std::slice::from_ref(subscription));
        }
    });

    let listener = tokio::net::TcpListener::bind(config.listen)
        .await
        .with_context(|| {
            format!(
                "cannot bind {}. Check that the address is free and that the \
                 container maps the port, or set MAILBOX_LISTEN to another \
                 HOST:PORT.",
                config.listen
            )
        })?;

    tracing::info!(
        listen = %config.listen,
        data_dir = %config.data_dir.display(),
        max_body_bytes = config.max_body_bytes,
        "mailbox started"
    );

    axum::serve(listener, router(state))
        .await
        .context("the HTTP server stopped unexpectedly")
}

/// Asks the running hub about its own health and exits 0 or 1.
///
/// Written against a raw socket rather than an HTTP client: a health probe
/// needs one request and one status line, and that is not worth carrying a
/// client library — with its TLS stack — into the runtime image for.
fn healthcheck() -> Result<()> {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::Duration;

    let config = Config::from_env()?;
    // A wildcard bind is not a usable destination, so probe the loopback.
    let target = if config.listen.ip().is_unspecified() {
        format!("127.0.0.1:{}", config.listen.port())
    } else {
        config.listen.to_string()
    };

    let mut stream =
        TcpStream::connect(&target).with_context(|| format!("cannot reach mailbox on {target}"))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.write_all(b"GET /healthz HTTP/1.0\r\nHost: localhost\r\n\r\n")?;

    let mut response = String::new();
    stream.read_to_string(&mut response)?;

    let status_line = response.lines().next().unwrap_or_default();
    anyhow::ensure!(
        status_line.contains(" 200"),
        "mailbox reports itself unhealthy: {}",
        response.trim()
    );
    Ok(())
}

/// W7 · structured logging.
///
/// `MAILBOX_LOG_FORMAT=json` emits one JSON object per line, so Loki can
/// filter on topic, subscription or message id rather than on substrings.
/// The default stays human-readable, because the first person reading these
/// logs is usually someone at a terminal.
fn init_tracing() {
    let filter = EnvFilter::try_from_env("MAILBOX_LOG").unwrap_or_else(|_| "info".into());
    let json = std::env::var("MAILBOX_LOG_FORMAT")
        .map(|format| format.eq_ignore_ascii_case("json"))
        .unwrap_or(false);

    if json {
        tracing_subscriber::fmt()
            .json()
            .with_current_span(false)
            .with_env_filter(filter)
            .init();
    } else {
        tracing_subscriber::fmt().with_env_filter(filter).init();
    }
}
