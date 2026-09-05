//! The hub's glue to chassis (3.0.0): what `/healthz` and `/metrics` say,
//! in the kit's shape, with the hub's own words and metric names (W6).
//! The kit answers the routes; these types answer the kit.

use std::sync::Arc;

use chassis::{ScrapeSource, Subsystem, SubsystemStatus};

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
