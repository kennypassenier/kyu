//! Thin binary (AR1), on chassis since 3.0.0: the kit owns the command
//! line, the configuration knobs, logging, `/healthz`, `/metrics`,
//! readiness, the graceful stop and signed self-update. This file assembles
//! the hub on top of it. Since step 2 (2026-09-06) the kit also owns the
//! door and the dashboard shell; the hub keeps its pages, its API and its
//! store.

use std::collections::BTreeMap;
use std::process::ExitCode;
use std::sync::Arc;

use chassis::{App, AppSpec};
use kyu::config::Config;
use kyu::engine::Engine;
use kyu::engine::clock::{Clock, SystemClock};
use kyu::http::{AppState, Limits, assets};
use kyu::kit::{import_app_tokens, mount};
use kyu::store::Store;
use kyu::sweeper::{self, Heartbeat};

/// What `--help` says beyond the kit's knobs: the hub's own environment.
const HELP_EXTRA: &str = "The hub's own environment (read next to the knobs above):
  KYU_TOKEN            the login token (16+ chars) — required since 3.0.0, the kit refuses to start without it
  KYU_SECRET_KEY       64 hex chars sealing app tokens and sessions; set together with KYU_TOKEN
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

    // K2-1: the kit seals its client store with the same key kyu sealed the
    // app tokens with; the one-time import below needs the raw value.
    let secret_hex = env.get("KYU_SECRET_KEY").cloned();
    let spec = AppSpec {
        name: "kyu",
        version: env!("CARGO_PKG_VERSION"),
        repository: Some("kennypassenier/kyu"),
        help_extra: Some(HELP_EXTRA),
        ..Default::default()
    };
    let args: Vec<String> = std::env::args().collect();
    // The hub's routes need the store, which needs the state directory the
    // kit resolves — so they are attached below. The only open routes are the
    // hub's two page assets; the kit owns the door since step 2 (W2 amended
    // 2026-09-06): the API needs a client token, the pages the admin login.
    let mut app = match App::from_args_with_env(spec, args, env, assets()) {
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
    let state_dir = loaded.state_dir.clone();

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
    app.on_check(move || {
        println!("store OK at {store_path}");
        Ok(())
    });

    let state = AppState::with_auth(
        engine.clone(),
        Limits::from_config(&config),
        heartbeat.clone(),
        config.auth.clone(),
    );
    mount(&mut app, state.clone(), engine.clone(), heartbeat.clone());
    // K2-1 (2026-09-06): the kit owns the clients now. The app tokens 2.x
    // issued keep working because they are copied, unchanged, into the kit's
    // store the first time this version starts — kyu-runner and newsflash
    // never notice. The 2.x `apps` table stays behind as history.
    if let (Some(key), Some(hex)) = (config.auth.key(), secret_hex.as_deref()) {
        match import_app_tokens(&state_dir, &engine, key, hex) {
            Ok(0) => {}
            Ok(count) => eprintln!(
                "kyu: imported {count} app token(s) from 2.x into the kit's client store; \
                 every existing token keeps working"
            ),
            Err(e) => {
                eprintln!("kyu: {e:#}");
                return ExitCode::FAILURE;
            }
        }
    }

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
            // W2 (amended 2026-09-06): the kit refuses to start without
            // KYU_TOKEN and KYU_SECRET_KEY, so by the time this runs the door
            // is closed; say so once, the way 2.x did.
            tracing::info!("this hub requires a token (KYU_TOKEN); the kit owns the door");
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
