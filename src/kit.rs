//! The hub's glue to chassis (3.0.0): what `/healthz` and `/metrics` say,
//! in the kit's shape, with the hub's own words and metric names (W6).
//! The kit answers the routes; these types answer the kit.

use std::sync::Arc;

use crate::dashboard::TopicView;

use chassis::{App, ScrapeSource, Subsystem, SubsystemStatus};

use crate::engine::Engine;
use crate::store::queries;
use crate::sweeper::Heartbeat;

/// `store`: the write probe plus the last failed write. Two questions,
/// because they fail differently: the probe takes the write lock, which
/// catches a read-only store; the failure record catches what a probe
/// cannot — a disk that is full but writable only refuses at commit time.
pub struct StoreSubsystem(pub Arc<Engine>);

impl Subsystem for StoreSubsystem {
    fn name(&self) -> &str {
        "store"
    }

    fn check(&self) -> SubsystemStatus {
        if let Err(error) = self.0.store().probe_writable() {
            return SubsystemStatus::failing(format!(
                "unwritable: {error:#}. Check free space on the data volume first, then that \
                 it is still mounted, writable by this user and not locked by another \
                 process; the hub refuses publishes it cannot store and recovers by itself \
                 once writes succeed again"
            ));
        }
        if let Some(ago) = self.0.store().recent_write_failure() {
            return SubsystemStatus::failing(format!(
                "a write failed {} seconds ago; the store may be full. Check free space on the \
                 data volume first, then that it is still mounted and writable by this user; \
                 the hub refuses publishes it cannot store and recovers by itself once writes \
                 succeed again",
                ago.as_secs()
            ));
        }
        SubsystemStatus::ok("writable")
    }
}

/// `sweeper`: whether the background work is still happening. While it is
/// stopped, expired leases are not returned to the queue and nothing is
/// dead-lettered, so messages appear to hang rather than to fail.
pub struct SweeperSubsystem {
    pub engine: Arc<Engine>,
    pub heartbeat: Heartbeat,
}

impl Subsystem for SweeperSubsystem {
    fn name(&self) -> &str {
        "sweeper"
    }

    fn check(&self) -> SubsystemStatus {
        let now = self.engine.now_ms();
        if self.heartbeat.is_alive(now) {
            return SubsystemStatus::ok("alive");
        }
        let behind_ms = now.saturating_sub(self.heartbeat.last_beat_ms());
        SubsystemStatus::failing(format!(
            "stalled: last ran {behind_ms} ms ago. Restart the service; until then expired \
             leases stay claimed and nothing is dead-lettered"
        ))
    }
}

/// The hub's `kyu_*` series (W6), appended verbatim to the kit's `/metrics`
/// so every Grafana panel keeps its query.
pub struct KyuMetrics {
    pub engine: Arc<Engine>,
    pub heartbeat: Heartbeat,
}

impl ScrapeSource for KyuMetrics {
    fn scrape(&self) -> String {
        match render(&self.engine, &self.heartbeat) {
            Ok(text) => text,
            Err(error) => {
                tracing::warn!(error = %format!("{error:#}"), "the metrics scrape failed");
                String::new()
            }
        }
    }
}

fn render(engine: &Engine, heartbeat: &Heartbeat) -> anyhow::Result<String> {
    let now = engine.now_ms();
    engine.store().read(|conn| {
        let counts = queries::delivery_counts(conn)?;
        let topics = queries::scalar(conn, "SELECT count(*) FROM topics")?;
        let subscriptions = queries::scalar(conn, "SELECT count(*) FROM subscriptions")?;
        let messages = queries::scalar(conn, "SELECT count(*) FROM messages")?;
        let bytes = queries::scalar(
            conn,
            "SELECT page_count * page_size FROM pragma_page_count(), pragma_page_size()",
        )
        .unwrap_or(0);

        let mut out = String::new();
        out.push_str("# HELP kyu_topics Topics on this hub.\n");
        out.push_str("# TYPE kyu_topics gauge\n");
        out.push_str(&format!("kyu_topics {topics}\n"));
        out.push_str("# HELP kyu_subscriptions Subscriptions across all topics.\n");
        out.push_str("# TYPE kyu_subscriptions gauge\n");
        out.push_str(&format!("kyu_subscriptions {subscriptions}\n"));
        out.push_str("# HELP kyu_messages Messages currently retained.\n");
        out.push_str("# TYPE kyu_messages gauge\n");
        out.push_str(&format!("kyu_messages {messages}\n"));
        out.push_str("# HELP kyu_store_bytes Size of the store on disk.\n");
        out.push_str("# TYPE kyu_store_bytes gauge\n");
        out.push_str(&format!("kyu_store_bytes {bytes}\n"));
        out.push_str("# HELP kyu_deliveries Deliveries by topic, subscription and state.\n");
        out.push_str("# TYPE kyu_deliveries gauge\n");
        for count in counts {
            out.push_str(&format!(
                "kyu_deliveries{{topic=\"{}\",subscription=\"{}\",state=\"{}\"}} {}\n",
                escape_label(&count.topic),
                escape_label(&count.subscription),
                count.state,
                count.count
            ));
        }
        out.push_str("# HELP kyu_sweeper_age_ms Time since the sweeper last ran.\n");
        out.push_str("# TYPE kyu_sweeper_age_ms gauge\n");
        out.push_str(&format!(
            "kyu_sweeper_age_ms {}\n",
            now.saturating_sub(heartbeat.last_beat_ms())
        ));
        Ok(out)
    })
}

/// Label values are topic and subscription names, which AR8 already limits
/// to `[a-z0-9._-]` — this is the belt to that braces.
fn escape_label(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// The Topics section on the kit's status page (K2-2): the counts at a
/// glance, the full list one click away on /topics.
pub struct TopicsSection(pub Arc<Engine>);

impl chassis::StatusSection for TopicsSection {
    fn render(&self) -> chassis::Section {
        let now = self.0.now_ms();
        let topics: Vec<TopicView> = self
            .0
            .store()
            .read(crate::store::queries::topic_summaries)
            .map(|summaries| {
                summaries
                    .into_iter()
                    .map(|summary| TopicView::at(summary, now))
                    .collect()
            })
            .unwrap_or_default();
        let backlog: i64 = topics.iter().map(|topic| topic.backlog).sum();
        let dead: i64 = topics.iter().map(|topic| topic.dead).sum();
        chassis::Section {
            title: "Topics".into(),
            explain: "What this hub is holding right now: every topic's unacknowledged \
                      messages add up to the backlog; dead letters gave up and wait for you."
                .into(),
            rows: vec![
                ("Topics".into(), topics.len().to_string()),
                ("Backlog".into(), backlog.to_string()),
                ("Dead letters".into(), dead.to_string()),
            ],
            html: Some("<p><a class=\"kp-button\" href=\"/topics\">Open the topics</a></p>".into()),
        }
    }

    /// [K-actions, chassis-rs 1.8.0] "Prune every dead letter" — every dead
    /// letter across every topic and subscription, in one confirmed sweep,
    /// once the storm that produced them is diagnosed. The one-at-a-time
    /// Delete on a topic page (W15) does not scale to a storm; this is that
    /// button's hub-wide sibling, not a replacement for it.
    fn actions(&self) -> Vec<chassis::SectionAction> {
        vec![chassis::SectionAction {
            label: "Prune every dead letter".into(),
            route: "/dashboard/dead-letters/prune".into(),
            method: "POST".into(),
            destructive: true,
            confirm: Some(
                "Delete every dead letter across every topic and subscription? \
                 This cannot be undone."
                    .into(),
            ),
            busy_label: Some("Pruning…".into()),
        }]
    }
}

/// K2-1 · one-time import of the 2.x app tokens into the kit's client store.
///
/// Runs before the kit opens the store; does nothing once
/// `clients.json.enc` exists, so it is idempotent across restarts. The
/// tokens are copied unchanged: an app that could publish yesterday can
/// publish today with the same line in its environment file.
pub fn import_app_tokens(
    state_dir: &std::path::Path,
    engine: &Engine,
    key: &crate::crypto::SecretKey,
    key_hex: &str,
) -> anyhow::Result<usize> {
    use chassis::core::clients::{Client, ClientsFile};
    use chassis::shell::store::{ClientStore, EncryptedFile, FileClientStore};

    let file = state_dir.join("clients.json.enc");
    if file.exists() {
        return Ok(0);
    }
    let apps: Vec<_> = engine
        .list_apps(key)?
        .into_iter()
        .filter(|app| app.is_live() && !app.token.is_empty())
        .collect();
    if apps.is_empty() {
        return Ok(0);
    }
    let kit_key = chassis::core::crypto::Key::parse_hex("KYU_SECRET_KEY", key_hex, key_hex)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let store = FileClientStore::open(EncryptedFile::new(file, kit_key, "clients"))
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let now = chassis::shell::time::now_rfc3339();
    let count = apps.len();
    store
        .update(&mut |clients: &mut ClientsFile| {
            for app in &apps {
                // Client is #[non_exhaustive] since chassis-rs 2.0.0: built
                // through Client::adopted rather than field by field, so a
                // field the kit adds later defaults instead of refusing to
                // compile here.
                clients.clients.push(Client::adopted(
                    format!("app-{}", app.name),
                    app.name.clone(),
                    app.token.clone(),
                    now.clone(),
                ));
            }
            Ok(clients.clients.last().cloned().expect("at least one app"))
        })
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(count)
}

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
