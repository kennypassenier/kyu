//! Thin binary (AR1): read configuration, start logging, serve. Every
//! behaviour worth testing lives in the library.

use anyhow::{Context, Result};
use mailbox::config::Config;
use mailbox::http;
use mailbox::store::Store;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let config = Config::from_env()?;

    // Opening the store migrates it forward, snapshotting first if there is
    // anything to lose (AR10). Failing here is correct: serving requests
    // without somewhere durable to put them would break K1's promise that a
    // confirmed publish is a kept one.
    let store = Store::open(&config.data_dir)?;
    tracing::info!(
        store = %store.path().map(|path| path.display().to_string()).unwrap_or_default(),
        "store ready"
    );

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

    axum::serve(listener, http::router())
        .await
        .context("the HTTP server stopped unexpectedly")
}

/// JSON output is W7 (rated Desired, built in L8); the spine is wired now
/// so every later milestone logs through it.
fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_env("MAILBOX_LOG").unwrap_or_else(|_| "info".into()))
        .init();
}
