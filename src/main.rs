//! Thin binary (AR1), on chassis since 3.0.0: the kit owns the command
//! line, the configuration knobs, logging, `/healthz`, `/metrics`,
//! readiness, the graceful stop and signed self-update. This file assembles
//! the hub on top of it. Since step 2 (2026-09-06) the kit also owns the
//! door and the dashboard shell; the hub keeps its pages, its API and its
//! store.

use std::collections::BTreeMap;
use std::process::ExitCode;
use std::sync::Arc;

use chassis::storage::{AppDb, Db};
use chassis::{App, AppSpec, Control};
use kyu::config::{Config, HELP_EXTRA, refuse_retired_data_dir};
use kyu::engine::Engine;
use kyu::engine::clock::{Clock, SystemClock};
use kyu::http::{AppState, Limits, assets};
use kyu::kit::{import_app_tokens, mount};
use kyu::store::STORE_FILE_NAME;
use kyu::store::migrations::MIGRATIONS;
use kyu::sweeper::{self, Heartbeat};

#[tokio::main]
async fn main() -> ExitCode {
    match start().await {
        Ok(code) | Err(code) => code,
    }
}

/// Assembles the hub on the kit and runs it; `Err` is an exit code whose
/// reason was already printed.
async fn start() -> Result<ExitCode, ExitCode> {
    // Done on the kit's environment snapshot rather than with set_var,
    // which is unsound once threads exist.
    let env: BTreeMap<String, String> = std::env::vars().collect();
    if let Err(message) = refuse_retired_data_dir(&env) {
        eprintln!("{message}");
        return Err(ExitCode::from(1));
    }
    // K2-1: the kit seals its client store with the same key kyu sealed the
    // app tokens with; the one-time import below needs the raw value.
    let secret_hex = env.get("KYU_SECRET_KEY").cloned();
    let mut app = build_app(env)?;
    if !app.needs_project_config() {
        return Ok(app.run().await);
    }
    let loaded = app
        .loaded
        .as_ref()
        .expect("a start or --check loads configuration");
    let state_dir = loaded.state_dir.clone();
    let config =
        Config::from_kit(&loaded.state_dir, app.limits.max_body_bytes as u64).map_err(fail)?;

    if matches!(app.control, Some(Control::Check)) {
        return check(app, &config).await;
    }
    // chassis 3.11.0: the kit owns the store. kyu declares it (schema, pool,
    // copy before a migration, its tables as data kinds on the Storage page,
    // the browser keys that follow the login) and the kit opens `kyu.db` at
    // start, before anything serves. Failing there is correct: serving
    // requests without somewhere durable to put them would break K1's
    // promise that a confirmed publish is a kept one.
    let db = kyu::storage::declare(&mut app).map_err(|e| {
        eprintln!("kyu: {e}");
        ExitCode::FAILURE
    })?;
    if let (Some(key), Some(hex)) = (config.auth.key(), secret_hex.as_deref()) {
        import_tokens(&state_dir, key, hex)?;
    }
    let (engine, state, heartbeat) = assemble(&mut app, &config, db);
    start_sweeper(&mut app, &state, engine, heartbeat);
    Ok(app.run().await)
}

/// The engine and the HTTP state on top of the store, hung on the kit.
fn assemble(app: &mut App, config: &Config, db: AppDb) -> (Arc<Engine>, AppState, Heartbeat) {
    let clock = SystemClock;
    let heartbeat = Heartbeat::starting_at(clock.now_ms());
    let engine = Arc::new(Engine::with_defaults(db, Arc::new(clock), config.defaults));
    let mut state = AppState::with_auth(
        engine.clone(),
        Limits::from_config(config),
        heartbeat.clone(),
        config.auth.clone(),
    );
    state.backup_keep = config.backup_keep;
    state.max_concurrent_polls = config.max_concurrent_polls;
    state.shutdown = Some(app.shutdown_signal());
    mount(app, state.clone(), engine.clone(), heartbeat.clone());
    (engine, state, heartbeat)
}

/// Prints a failure the way every start error is printed.
fn fail(error: anyhow::Error) -> ExitCode {
    eprintln!("kyu: {error:#}");
    ExitCode::FAILURE
}

/// The kit's app from the command line and environment.
fn build_app(env: BTreeMap<String, String>) -> Result<App, ExitCode> {
    let spec = AppSpec {
        name: "kyu",
        version: env!("CARGO_PKG_VERSION"),
        repository: Some("kennypassenier/kyu"),
        help_extra: Some(HELP_EXTRA),
        ..Default::default()
    };
    let args: Vec<String> = std::env::args().collect();
    // The hub's routes need the store, which needs the state directory the
    // kit resolves — so they are attached later. The only open routes are
    // the hub's two page assets; the kit owns the door since step 2 (W2
    // amended 2026-09-06): the API needs a client token, the pages the
    // admin login.
    let mut app = App::from_args_with_env(spec, args, env, assets()).map_err(|e| {
        eprintln!("{e}");
        ExitCode::FAILURE
    })?;
    // fix-15 (chassis-rs 3.2.0): without this the kit's dashboard pages and
    // layout would show the binary name ("kyu") as the brand — which here
    // happens to already be the service's own display name (README,
    // templates), but set it explicitly rather than rely on that match.
    app.brand_title("kyu");
    Ok(app)
}

/// `--check` reads the store and writes nothing (fix-check-1). It used to
/// open the store through the same path as a start, which migrates — and
/// on CT 109 the fix-state-1 drill, run as root, migrated the live store
/// and left a root-owned snapshot behind. The kit's self-update runs
/// `<staging> --check` before the swap, so a check that writes would move
/// the schema forward before the new binary is even in place.
async fn check(mut app: App, config: &Config) -> Result<ExitCode, ExitCode> {
    let path = config.data_dir.join(STORE_FILE_NAME);
    let inspection = Db::inspect(&path, MIGRATIONS.len()).map_err(|e| fail(e.into()))?;
    if inspection.is_newer() {
        eprintln!(
            "kyu: {}. Roll back to the newer build, or restore the copy its migration wrote \
             (kyu.pre-v*.db in the data directory). kyu never downgrades a schema, because \
             guessing at an unknown layout risks the messages in it.",
            inspection.describe()
        );
        return Err(ExitCode::FAILURE);
    }
    app.on_check(move || {
        println!("{}", inspection.describe());
        Ok(())
    });
    Ok(app.run().await)
}

/// K2-1 (2026-09-06, amended 2026-09-20): the kit owns the clients now.
/// The app tokens 2.x issued keep working because every start copies the
/// ones whose name is not in the kit's store yet, unchanged — kyu-runner
/// and newsflash never notice. The 2.x `apps` table stays behind as
/// history; a restored store with apps the door has never seen is the
/// case this exists for (fix-state-1 follow-up).
fn import_tokens(
    state_dir: &std::path::Path,
    key: &kyu::crypto::SecretKey,
    hex: &str,
) -> Result<(), ExitCode> {
    // The import has to finish before the kit opens its client store, which
    // is before the kit opens the database, so it reads the 2.x `apps` table
    // through a short-lived handle of its own. That open does what a start
    // does (migrates, copies first), so the kit's open finds nothing to do.
    let db = kyu::storage::open(state_dir).map_err(|e| fail(e.into()))?;
    let engine = Engine::new(db, Arc::new(SystemClock));
    match import_app_tokens(state_dir, &engine, key, hex).map_err(fail)? {
        0 => {}
        count => eprintln!(
            "kyu: imported {count} app token(s) from the 2.x apps table into the kit's \
             client store (missing by name); every existing token keeps working"
        ),
    }
    Ok(())
}

/// The sweeper is what makes delivery at-least-once rather than
/// at-most-once: without it an expired lease would never come back.
/// Started after the bind, so its first beat is never older than the
/// listener.
fn start_sweeper(app: &mut App, state: &AppState, engine: Arc<Engine>, heartbeat: Heartbeat) {
    let notifiers = state.notifiers.clone();
    app.on_start(move || {
        // W2 (amended 2026-09-06): the kit refuses to start without
        // KYU_TOKEN and KYU_SECRET_KEY, so by the time this runs the door
        // is closed; say so once, the way 2.x did.
        tracing::info!("this hub requires a token (KYU_TOKEN); the kit owns the door");
        let _sweeper = sweeper::spawn(engine, heartbeat, move |woken| {
            for (topic, subscription) in woken {
                notifiers.wake(topic, std::slice::from_ref(subscription));
            }
        });
    });
}
