//! The hub's `kyu_*` series on the kit's `/metrics` (W6).

use std::sync::Arc;

use chassis::ScrapeSource;

use crate::engine::Engine;
use crate::store::dedup as store_dedup;
use crate::store::queries;
use crate::sweeper::Heartbeat;

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
        out.push_str("# HELP kyu_dedup_keys Dedup keys currently remembered.\n");
        out.push_str("# TYPE kyu_dedup_keys gauge\n");
        out.push_str(&format!(
            "kyu_dedup_keys {}\n",
            store_dedup::live_keys(conn)?
        ));
        out.push_str(
            "# HELP kyu_duplicates_suppressed Publishes suppressed by a dedup key, per topic, \
             over the retained hourly statistics (30 days).\n",
        );
        out.push_str("# TYPE kyu_duplicates_suppressed gauge\n");
        for (topic, count) in store_dedup::suppressed_by_topic(conn)? {
            out.push_str(&format!(
                "kyu_duplicates_suppressed{{topic=\"{}\"}} {count}\n",
                escape_label(&topic)
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
