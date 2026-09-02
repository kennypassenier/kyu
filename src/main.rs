//! Thin binary (AR1): read configuration, start logging, serve. Every
//! behaviour worth testing lives in the library.

use std::sync::Arc;

use anyhow::{Context, Result};
use kyu::cli;
use kyu::config::Config;
use kyu::engine::Engine;
use kyu::engine::clock::{Clock, SystemClock};
use kyu::http::{AppState, Limits, router};
use kyu::shutdown;
use kyu::store::Store;
use kyu::sweeper::{self, Heartbeat};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    // Anything the command line does not recognise is refused rather than
    // ignored (cli, 1.0.1): before that, `kyu --version` started the hub
    // and sat there, and a typo in a unit file would have started a second
    // one on the same store.
    match cli::parse(std::env::args().skip(1)) {
        Ok(cli::Action::Serve) => {}
        // The runtime image has no shell, so the container healthcheck cannot
        // call curl: the binary probes itself instead (W6, T9).
        Ok(cli::Action::Healthcheck) => return healthcheck(),
        Ok(cli::Action::Version) => {
            println!("kyu {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Ok(cli::Action::Help) => {
            println!("{}", cli::HELP);
            return Ok(());
        }
        Err(refused) => {
            eprintln!("{refused}");
            // 2, the conventional "you used me wrong", so a script can tell
            // this apart from the hub failing at runtime.
            std::process::exit(2);
        }
    }

    init_tracing();

    let config = Config::from_env()?;

    // Opening the store migrates it forward, snapshotting first if there is
    // anything to lose (AR10). Failing here is correct: serving requests
    // without somewhere durable to put them would break K1's promise that a
    // confirmed publish is a kept one.
    let store = Arc::new(Store::open(&config.data_dir)?);
    // Kept past the move into the engine so the stop path can settle the
    // write-ahead log (W12); the engine owns the store, this is a handle.
    let store_for_shutdown = store.clone();
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
        tracing::info!("this hub requires a token (KYU_TOKEN)");
    } else {
        tracing::warn!(
            "this hub has NO token: anyone who can reach it can read every \
             message, publish, and use the dashboard buttons. Set KYU_TOKEN \
             and KYU_SECRET_KEY to protect it."
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
                 container maps the port, or set KYU_LISTEN to another \
                 HOST:PORT.",
                config.listen
            )
        })?;

    tracing::info!(
        listen = %config.listen,
        data_dir = %config.data_dir.display(),
        max_body_bytes = config.max_body_bytes,
        "kyu started"
    );

    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown::requested())
        .await
        .context("the HTTP server stopped unexpectedly")?;

    // Past this point no new request will arrive and the ones in flight have
    // finished, so the store can be settled without racing anything (W12).
    shutdown::settle(
        store_for_shutdown,
        std::time::Duration::from_millis(config.shutdown_timeout_ms),
    )
    .await;
    Ok(())
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
        TcpStream::connect(&target).with_context(|| format!("cannot reach kyu on {target}"))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.write_all(b"GET /healthz HTTP/1.0\r\nHost: localhost\r\n\r\n")?;

    let mut response = String::new();
    stream.read_to_string(&mut response)?;

    let status_line = response.lines().next().unwrap_or_default();
    anyhow::ensure!(
        status_line.contains(" 200"),
        "kyu reports itself unhealthy: {}",
        response.trim()
    );
    Ok(())
}

/// W7 · structured logging.
///
/// `KYU_LOG_FORMAT=json` emits one JSON object per line, so Loki can
/// filter on topic, subscription or message id rather than on substrings.
/// The default stays human-readable, because the first person reading these
/// logs is usually someone at a terminal.
fn init_tracing() {
    let filter = EnvFilter::try_from_env("KYU_LOG").unwrap_or_else(|_| "info".into());
    let json = std::env::var("KYU_LOG_FORMAT")
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
