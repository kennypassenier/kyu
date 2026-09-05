//! Thin binary (AR1), on chassis since 3.0.0: the kit owns the command
//! line, the configuration knobs, logging, `/healthz`, `/metrics`,
//! readiness, the graceful stop and signed self-update. This file assembles
//! the hub on top of it. The hub's door policy (W2: unprotected, or a
//! bootstrap token plus sealed app tokens) and its dashboard stay the hub's.

use std::collections::BTreeMap;
use std::process::ExitCode;
use std::sync::Arc;

use axum::Router;
use chassis::{App, AppSpec};
use kyu::config::Config;
use kyu::engine::Engine;
use kyu::engine::clock::{Clock, SystemClock};
use kyu::http::{AppState, Limits, router};
use kyu::kit::{KyuMetrics, StoreSubsystem, SweeperSubsystem};
use kyu::store::Store;
use kyu::sweeper::{self, Heartbeat};

/// What `--help` says beyond the kit's knobs: the hub's own environment.
const HELP_EXTRA: &str = "The hub's own environment (read next to the knobs above):
  KYU_TOKEN            bootstrap token (16+ chars); unset = an UNPROTECTED hub (W2)
  KYU_SECRET_KEY       64 hex chars sealing the app tokens; set exactly when KYU_TOKEN is
  KYU_RETENTION_MS     default message retention in ms, or `never`
  KYU_IDLE_FLAG_MS     idle-subscription flag threshold in ms
  KYU_IDLE_ARCHIVE_MS  idle-subscription archive threshold in ms
  KYU_DATA_DIR         the 2.x name of KYU_STATE_DIR; still honoured, with a warning
Long polls (`GET /t/{topic}/next?wait=`) are exempt from the request timeout.";

#[tokio::main]
async fn main() -> ExitCode {
    // 2.x called the state root KYU_DATA_DIR. Honour it until every
    // environment file has moved, and say so on every start (standing rule
    // 12: no silent substitution). Done on the kit's environment snapshot
    // rather than with set_var, which is unsound once threads exist.
    let mut env: BTreeMap<String, String> = std::env::vars().collect();
    let legacy_data_dir = match env.get("KYU_STATE_DIR") {
        Some(_) => None,
        None => env.get("KYU_DATA_DIR").cloned(),
    };
    if let Some(dir) = &legacy_data_dir {
        env.insert("KYU_STATE_DIR".to_string(), dir.clone());
    }

    let spec = AppSpec {
        name: "kyu",
        version: env!("CARGO_PKG_VERSION"),
        repository: Some("kennypassenier/kyu"),
        help_extra: Some(HELP_EXTRA),
        ..Default::default()
    };
    let args: Vec<String> = std::env::args().collect();
    // The hub's routes need the store, which needs the state directory the
    // kit resolves — so the router is attached below, as public routes with
    // the hub's own door policy (W2) inside them.
    let mut app = match App::from_args_with_env(spec, args, env, Router::new()) {
        Ok(app) => app,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    if !app.needs_project_config() {
        return app.run().await;
    }
    let loaded = app
        .loaded
        .as_ref()
        .expect("a start or --check loads configuration");

    let config = match Config::from_kit(&loaded.state_dir, app.limits.max_body_bytes as u64) {
        Ok(config) => config,
        Err(e) => {
            eprintln!("kyu: {e:#}");
            return ExitCode::FAILURE;
        }
    };

    // Opening the store migrates it forward, snapshotting first if there is
    // anything to lose (AR10). Failing here is correct: serving requests
    // without somewhere durable to put them would break K1's promise that a
    // confirmed publish is a kept one. `--check` opens it too: a store that
    // will not open is exactly what a pre-start check exists to catch.
    let store = match Store::open(&config.data_dir) {
        Ok(store) => Arc::new(store),
        Err(e) => {
            eprintln!("kyu: {e:#}");
            return ExitCode::FAILURE;
        }
    };
    let store_for_flush = store.clone();
    let store_path = store
        .path()
        .map(|path| path.display().to_string())
        .unwrap_or_default();

    let clock = SystemClock;
    let heartbeat = Heartbeat::starting_at(clock.now_ms());
    let engine = Arc::new(Engine::with_defaults(
        store,
        Arc::new(clock),
        config.defaults,
    ));
    let protected = config.auth.is_protected();
    app.on_check(move || {
        println!(
            "store OK at {store_path}; door: {}",
            if protected { "token" } else { "UNPROTECTED" }
        );
        Ok(())
    });

    let state = AppState::with_auth(
        engine.clone(),
        Limits::from_config(&config),
        heartbeat.clone(),
        config.auth.clone(),
    );
    app.subsystem(StoreSubsystem(engine.clone()));
    app.subsystem(SweeperSubsystem {
        engine: engine.clone(),
        heartbeat: heartbeat.clone(),
    });
    app.metrics_source(KyuMetrics {
        engine: engine.clone(),
        heartbeat: heartbeat.clone(),
    });
    // A long poll waits up to Limits::MAX_WAIT_S (300 s) on purpose; the
    // kit's request timeout (30 s) must not cut it short. The prefix also
    // covers publish/ack/nack, which answer at once anyway.
    app.exempt_from_timeout("/t/");
    // Public as far as the kit is concerned: the hub's own door policy runs
    // inside (require_token as a route layer; the login page is open).
    app.api_routes(router(state.clone()));

    {
        let notifiers = state.notifiers.clone();
        let engine = engine.clone();
        let heartbeat = heartbeat.clone();
        app.on_start(move || {
            if let Some(dir) = legacy_data_dir {
                tracing::warn!(
                    dir = %dir,
                    "KYU_DATA_DIR is the 2.x name; it still works, but rename it to \
                     KYU_STATE_DIR in the environment file — the alias goes away in 4.0"
                );
            }
            // W2 · say out loud which of the two modes this is. An unprotected
            // hub is a legitimate choice; a hub you *think* is protected is
            // not, and the only defence against that is saying so on every
            // single startup.
            if protected {
                tracing::info!("this hub requires a token (KYU_TOKEN)");
            } else {
                tracing::warn!(
                    "this hub has NO token: anyone who can reach it can read every \
                     message, publish, and use the dashboard buttons. Set KYU_TOKEN \
                     and KYU_SECRET_KEY to protect it."
                );
            }
            // The sweeper is what makes delivery at-least-once rather than
            // at-most-once: without it an expired lease would never come back.
            // Started after the bind, so its first beat is never older than
            // the listener.
            let _sweeper = sweeper::spawn(engine, heartbeat, move |woken| {
                for (topic, subscription) in woken {
                    notifiers.wake(topic, std::slice::from_ref(subscription));
                }
            });
        });
    }
    // W12 · after the drain, settle the write-ahead log. Bounded by the
    // kit's shutdown budget (KYU_SHUTDOWN_TIMEOUT_MS); a checkpoint that does
    // not finish costs nothing but a file-level backup's restorability —
    // WAL and synchronous=FULL keep the data intact either way.
    app.on_flush(move || match store_for_flush.checkpoint() {
        Ok(()) => tracing::info!("store checkpointed; stopping"),
        Err(error) => tracing::warn!(
            %error,
            "could not checkpoint the store; stopping anyway. The data is intact, but the \
             write-ahead log is still on disk, so a file-level backup of the data directory \
             may not restore. Take one with POST /api/backup instead."
        ),
    });
    app.run().await
}
