//! The HTTP surface (AR2). Routes translate HTTP into engine calls and
//! back; business logic stays in [`crate::engine`].

pub mod error;
pub mod handlers;
pub mod notify;

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::routing::{get, post};

use crate::config::{Auth, Config};
use crate::engine::Engine;
use crate::sweeper::Heartbeat;

use notify::Notifiers;

/// Bounds that belong to the transport rather than to delivery policy
/// (which is per subscription and lives in the database — K7, AR6).
#[derive(Debug, Clone)]
pub struct Limits {
    pub max_body_bytes: usize,
    pub default_wait_s: u64,
    pub max_wait_s: u64,
    /// How often a waiting poll looks again regardless of wakeups (AR5).
    /// Correctness must never depend on a notification arriving.
    pub recheck_interval: Duration,
}

impl Limits {
    pub const DEFAULT_WAIT_S: u64 = 30;
    pub const MAX_WAIT_S: u64 = 300;
    pub const RECHECK: Duration = Duration::from_secs(5);

    pub fn from_config(config: &Config) -> Self {
        Self {
            max_body_bytes: config.max_body_bytes as usize,
            default_wait_s: Self::DEFAULT_WAIT_S,
            max_wait_s: Self::MAX_WAIT_S,
            recheck_interval: Self::RECHECK,
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    pub engine: Arc<Engine>,
    pub notifiers: Arc<Notifiers>,
    pub limits: Limits,
    /// Shared with the sweeper so the health endpoint can tell whether the
    /// background work is still happening (W6).
    pub heartbeat: Heartbeat,
    /// The door policy (W2). `Arc` because every request reads it.
    pub auth: Arc<Auth>,
}

impl AppState {
    /// An unprotected hub. Tests that are not about the door use this;
    /// `main` always goes through [`Self::with_auth`].
    pub fn new(engine: Arc<Engine>, limits: Limits, heartbeat: Heartbeat) -> Self {
        Self::with_auth(engine, limits, heartbeat, Auth::Unprotected)
    }

    pub fn with_auth(
        engine: Arc<Engine>,
        limits: Limits,
        heartbeat: Heartbeat,
        auth: Auth,
    ) -> Self {
        Self {
            engine,
            notifiers: Arc::new(Notifiers::new()),
            limits,
            heartbeat,
            auth: Arc::new(auth),
        }
    }
}

/// The machine API (K1–K3, K6, K7): what kyu-runner, newsflash and scripts
/// call with a client token. Since step 2 of the chassis migration the kit
/// owns the door — this router is handed to `App::api_routes`, which puts
/// every route behind `Authorization: Bearer <token>` (a client token from
/// the kit's store, or the login token for scripts run by Kenny). No layer
/// here: the in-process tests mount it open, the binary mounts it closed.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/backup", post(handlers::backup))
        .route("/t/{topic}", post(handlers::publish))
        .route("/t/{topic}/next", get(handlers::receive))
        .route("/t/{topic}/ack/{id}", post(handlers::ack))
        .route("/t/{topic}/nack/{id}", post(handlers::nack))
        .route(
            "/api/t/{topic}/subs/{subscription}/policy",
            get(handlers::get_policy).put(handlers::put_policy),
        )
        .route(
            "/api/t/{topic}/subs/{subscription}/dead",
            get(handlers::list_dead),
        )
        .route(
            "/api/t/{topic}/subs/{subscription}/dead/{id}/requeue",
            post(handlers::requeue_dead),
        )
        .route(
            "/api/t/{topic}/subs/{subscription}/deliveries/{id}/delete",
            post(handlers::delete_delivery),
        )
        .route(
            "/api/t/{topic}/subs/{subscription}/unarchive",
            post(handlers::unarchive),
        )
        .route(
            "/api/t/{topic}/retention",
            get(handlers::get_retention).put(handlers::put_retention),
        )
        .with_state(state)
}

/// The hub's own dashboard pages (K10, W9, W15, W16), handed to
/// `App::dashboard_routes`: the kit renders them inside its layout behind
/// the admin login and refuses cross-origin form posts. `/` is the kit's
/// status page (a Topics section points here); `/apps` is what 2.x called
/// the clients page and keeps working as a redirect (K2-3).
pub fn pages(state: AppState) -> Router {
    Router::new()
        .route("/topics", get(handlers::topics_page))
        .route("/apps", get(handlers::apps_redirect))
        .route("/t/{topic}/dashboard", get(handlers::dashboard_topic))
        .route(
            "/t/{topic}/dashboard/subs/{subscription}",
            get(handlers::dashboard_subscription),
        )
        .route(
            "/t/{topic}/dashboard/publish",
            post(handlers::dashboard_publish),
        )
        .route(
            "/t/{topic}/dashboard/requeue",
            post(handlers::dashboard_requeue),
        )
        .route(
            "/t/{topic}/dashboard/delivery/delete",
            post(handlers::dashboard_delete_delivery),
        )
        .route(
            "/dashboard/dead-letters/prune",
            post(handlers::dashboard_prune_dead_letters),
        )
        .with_state(state)
}

/// The two files the hub's pages need beyond the kit's assets (kyu.css and
/// app.js), served open on `/assets/…` because the kit owns `/static/…`.
pub fn assets() -> Router {
    Router::new().route("/assets/{file}", get(handlers::kyu_asset))
}

/// [`router`] plus `/healthz` and `/metrics` as the kit serves them (3.0.0),
/// for in-process tests and embedders that run the hub without `chassis::App`.
/// The binary must NOT use this: the kit mounts the same two routes itself
/// and axum refuses a second handler on a path.
pub fn router_with_probes(state: AppState) -> Router {
    use axum::response::IntoResponse;
    use chassis::ScrapeSource;
    use chassis::shell::health::{Health, healthz};

    use crate::kit::{KyuMetrics, StoreSubsystem, SweeperSubsystem};

    let health = Health::new(
        env!("CARGO_PKG_VERSION"),
        Duration::from_secs(2),
        vec![
            Arc::new(StoreSubsystem(state.engine.clone())),
            Arc::new(SweeperSubsystem {
                engine: state.engine.clone(),
                heartbeat: state.heartbeat.clone(),
            }),
        ],
    );
    let metrics = Arc::new(KyuMetrics {
        engine: state.engine.clone(),
        heartbeat: state.heartbeat.clone(),
    });
    let probes = Router::new()
        .route("/healthz", get(healthz).with_state(health))
        .route(
            "/metrics",
            get(move || {
                let metrics = metrics.clone();
                async move {
                    (
                        [(
                            axum::http::header::CONTENT_TYPE,
                            "text/plain; version=0.0.4",
                        )],
                        metrics.scrape(),
                    )
                        .into_response()
                }
            }),
        );
    router(state).merge(probes)
}
