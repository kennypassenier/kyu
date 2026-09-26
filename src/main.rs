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
  KYU_EXPIRED_EVENT_WINDOW_MS  one message.expired per subscription per this many ms (default a day)
  KYU_DATA_DIR         retired in 4.0.0 (the 2.x name of KYU_STATE_DIR); set, it refuses to start
Long polls (`GET /t/{topic}/next?wait=`) are exempt from the request timeout.
--check reads the store (quick_check, schema) and writes nothing: a pending migration is
reported, and applied only at start, after a snapshot.";

/// `KYU_DATA_DIR`, the 2.x name of the state root, is retired in 4.0.0:
/// kyu reads only `KYU_STATE_DIR`. A set `KYU_DATA_DIR` is refused rather
/// than ignored. Ignored, an environment file that still carried it would
/// start the hub on an empty store in the default directory, with every
/// topic and app token left behind in the old one — fix-state-1's fault
/// (CT 109, 2026-09-10) reached by another road (standing rules 12 and 45).
fn refuse_retired_data_dir(env: &BTreeMap<String, String>) -> Result<(), String> {
    match env.get("KYU_DATA_DIR") {
        None => Ok(()),
        Some(data) => Err(format!(
            "KYU_DATA_DIR ({data}) is the 2.x name of the state directory and is no \
             longer read since kyu 4.0.0, so starting would open whatever store \
             KYU_STATE_DIR points at instead. Rename it to KYU_STATE_DIR in the \
             environment file. If the store lives in {data} and KYU_STATE_DIR names \
             another directory, move kyu.db, kyu.db-wal and kyu.db-shm together \
             (all three, or the newest writes are lost)."
        )),
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn the_retired_2x_name_refuses_to_start_and_names_its_successor() {
        // 4.0 reads only KYU_STATE_DIR. An environment file that still says
        // KYU_DATA_DIR would otherwise start the hub on an empty store in the
        // default directory: fix-state-1's fault, reached by another road.
        for pairs in [
            &[("KYU_DATA_DIR", "/appdata/kyu/kyu-config/data")][..],
            &[
                ("KYU_STATE_DIR", "/appdata/kyu/kyu-config"),
                ("KYU_DATA_DIR", "/appdata/kyu/kyu-config"),
            ][..],
        ] {
            let error =
                refuse_retired_data_dir(&env(pairs)).expect_err("the 2.x name must not start");
            assert!(
                error.contains("KYU_STATE_DIR"),
                "names the successor: {error}"
            );
            assert!(
                error.contains("kyu.db-wal"),
                "the remedy moves all three files: {error}"
            );
        }
    }

    #[test]
    fn the_3x_name_alone_starts() {
        assert!(refuse_retired_data_dir(&env(&[("KYU_STATE_DIR", "/var/lib/kyu")])).is_ok());
        assert!(refuse_retired_data_dir(&env(&[])).is_ok());
    }
}
