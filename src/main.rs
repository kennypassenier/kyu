//! Thin binary (AR1): read configuration, start logging, serve. Every
//! behaviour worth testing lives in the library.

use anyhow::{Context, Result};
use mailbox::config::Config;
use mailbox::http;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let config = Config::from_env()?;
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
