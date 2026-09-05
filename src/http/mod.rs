//! The HTTP surface (AR2). Routes translate HTTP into engine calls and
//! back; business logic stays in [`crate::engine`].

pub mod auth;
pub mod csrf;
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

pub fn router(state: AppState) -> Router {
    // Open by design and by review: monitoring, the login page itself, and
    // the two static assets the pages need. Everything else lives in the
    // protected router below, so forgetting to think about a new route
    // fails closed rather than open (W2).
    // 3.0.0: /healthz and /metrics are the kit's (fed by `crate::kit`).
    let open = Router::new()
        .route("/static/{file}", get(handlers::static_asset))
        .route("/login", get(handlers::login_form).post(handlers::login))
        .route("/logout", post(handlers::logout));

    let protected = Router::new()
        .route("/", get(handlers::dashboard_index))
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
        .route("/apps", get(handlers::apps_page))
        .route("/apps/create", post(handlers::apps_create))
        .route("/apps/revoke", post(handlers::apps_revoke))
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
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::require_token,
        ));

    open.merge(protected)
        .layer(axum::middleware::from_fn(csrf::same_origin_only))
        .with_state(state)
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
