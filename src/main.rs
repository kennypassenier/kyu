//! Thin binary (AR1), on chassis since 3.0.0: the kit owns the command
//! line, the configuration knobs, logging, `/healthz`, `/metrics`,
//! readiness, the graceful stop and signed self-update. This file assembles
//! the hub on top of it. Since step 2 (2026-09-06) the kit also owns the
//! door and the dashboard shell; the hub keeps its pages, its API and its
//! store.

use std::collections::BTreeMap;
use std::process::ExitCode;
use std::sync::Arc;

use chassis::{App, AppSpec, Control};
use kyu::config::{Config, HELP_EXTRA, refuse_retired_data_dir};
use kyu::engine::Engine;
use kyu::engine::clock::{Clock, SystemClock};
use kyu::http::{AppState, Limits, assets};
use kyu::kit::{import_app_tokens, mount};
use kyu::store::Store;
use kyu::sweeper::{self, Heartbeat};

#[tokio::main]
async fn main() -> ExitCode {
    // Done on the kit's environment snapshot rather than with set_var,
    // which is unsound once threads exist.
    let env: BTreeMap<String, String> = std::env::vars().collect();
    if let Err(message) = refuse_retired_data_dir(&env) {
        eprintln!("{message}");
        return ExitCode::from(1);
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
    // fix-15 (chassis-rs 3.2.0): without this the kit's dashboard pages and
    // layout would show the binary name ("kyu") as the brand — which here
    // happens to already be the service's own display name (README,
    // templates), but set it explicitly rather than rely on that match.
    app.brand_title("kyu");
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

    // `--check` reads the store and writes nothing (fix-check-1). It used to
    // open the store through the same path as a start, which migrates — and
    // on CT 109 the fix-state-1 drill, run as root, migrated the live store
    // and left a root-owned snapshot behind. The kit's self-update runs
    // `<staging> --check` before the swap, so a check that writes would move
    // the schema forward before the new binary is even in place.
    if matches!(app.control, Some(Control::Check)) {
        let inspection = match Store::inspect(&config.data_dir) {
            Ok(inspection) => inspection,
            Err(e) => {
                eprintln!("kyu: {e:#}");
                return ExitCode::FAILURE;
            }
        };
        app.on_check(move || {
            println!("{}", inspection.describe());
            Ok(())
        });
        return app.run().await;
    }

    // Opening the store migrates it forward, snapshotting first if there is
    // anything to lose (AR10). Failing here is correct: serving requests
    // without somewhere durable to put them would break K1's promise that a
    // confirmed publish is a kept one.
    let store = match Store::open(&config.data_dir) {
        Ok(store) => Arc::new(store),
        Err(e) => {
            eprintln!("kyu: {e:#}");
            return ExitCode::FAILURE;
        }
    };
    let store_for_flush = store.clone();
    let store_for_pause = store.clone();

    let clock = SystemClock;
    let heartbeat = Heartbeat::starting_at(clock.now_ms());
    let engine = Arc::new(Engine::with_defaults(
        store,
        Arc::new(clock),
        config.defaults,
    ));
    let state = AppState::with_auth(
        engine.clone(),
        Limits::from_config(&config),
        heartbeat.clone(),
        config.auth.clone(),
    );
    mount(&mut app, state.clone(), engine.clone(), heartbeat.clone());
    // chassis 3.11.0 / 3.12.0: kyu's tables as data kinds on the Storage
    // page and the browser keys that follow the login. The kit opens the
    // same `kyu.db` under the same migration list; nothing moves.
    if let Err(e) = kyu::storage::declare(&mut app) {
        eprintln!("kyu: {e}");
        return ExitCode::FAILURE;
    }
    // K2-1 (2026-09-06, amended 2026-09-20): the kit owns the clients now.
    // The app tokens 2.x issued keep working because every start copies the
    // ones whose name is not in the kit's store yet, unchanged — kyu-runner
    // and newsflash never notice. The 2.x `apps` table stays behind as
    // history; a restored store with apps the door has never seen is the
    // case this exists for (fix-state-1 follow-up).
    if let (Some(key), Some(hex)) = (config.auth.key(), secret_hex.as_deref()) {
        match import_app_tokens(&state_dir, &engine, key, hex) {
            Ok(0) => {}
            Ok(count) => eprintln!(
                "kyu: imported {count} app token(s) from the 2.x apps table into the kit's \
                 client store (missing by name); every existing token keeps working"
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
    // feat-backup-1 (chassis 3.1.0) · `<name> backup-pause` holds every
    // write wrapped in `chassis::shell::backup::writing_blocking()` (see
    // `http::handlers::spawn_engine_write`), waits for the ones already in
    // flight, then runs this hook before answering the caller: the same
    // `wal_checkpoint(TRUNCATE)` W12 runs at shutdown, so a file-level
    // backup of the state root sees no WAL/SHM to miss. An error here falls
    // the pause back to stopping the unit instead.
    app.on_backup_pause(move |_mode| {
        store_for_pause.checkpoint().map_err(|error| {
            chassis::Error::dependency(
                format!("could not checkpoint the store for a backup pause: {error:#}"),
                "the pause falls back to stopping the unit; investigate the write-ahead log",
            )
        })
    });
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
