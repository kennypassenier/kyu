//! The hub's glue to chassis (3.0.0): what `/healthz` and `/metrics` say,
//! in the kit's shape, with the hub's own words and metric names (W6).
//! The kit answers the routes; these types answer the kit.

mod health;
mod import;
mod metrics;
mod status;

use std::sync::Arc;

use chassis::App;

use crate::engine::Engine;
use crate::sweeper::Heartbeat;

pub use health::{StoreSubsystem, SweeperSubsystem};
pub use import::import_app_tokens;
pub use metrics::KyuMetrics;
pub use status::TopicsSection;

/// Everything the hub hangs on the kit, in one place, so the binary and the
/// in-process test harness assemble the same service (K2, 2026-09-06).
pub fn mount(
    app: &mut App,
    state: crate::http::AppState,
    engine: Arc<Engine>,
    heartbeat: Heartbeat,
) {
    app.subsystem(StoreSubsystem(engine.clone()));
    app.subsystem(SweeperSubsystem {
        engine: engine.clone(),
        heartbeat: heartbeat.clone(),
    });
    app.metrics_source(KyuMetrics {
        engine: engine.clone(),
        heartbeat,
    });
    // A long poll waits up to Limits::MAX_WAIT_S (300 s) on purpose; the
    // kit's request timeout (30 s) must not cut it short. The prefix also
    // covers publish/ack/nack, which answer at once anyway.
    app.exempt_from_timeout("/t/");
    // The machine API behind client tokens, the pages behind the admin
    // login (K2-2): `/` is the kit's status page with a Topics section, the
    // full list lives on /topics, and the apps page is the kit's /clients
    // under its old label (K2-3: /apps redirects there).
    app.api_routes(crate::http::router(state.clone()));
    app.dashboard_routes(crate::http::pages(state));
    app.nav_entry("Topics", "/topics");
    // Every kit sentence about "clients" reads "app" here (K-vocabulary,
    // chassis-rs 1.8.0): the heading and nav label derive from the plural,
    // capitalised ("Apps"), so this also replaces the narrower
    // clients_label("Apps") that only relabelled the heading.
    app.vocabulary("app", "apps");
    app.status_section(TopicsSection(engine));
    // "Send test" on the Apps page publishes one message with that app's
    // token, so "does my token work?" has a button.
    app.test_route(
        "POST",
        "/t/kyu-test",
        "application/json",
        r#"{"hello":"from the dashboard"}"#,
    );
}
